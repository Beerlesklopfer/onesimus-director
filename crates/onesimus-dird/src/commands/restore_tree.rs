//! The restore file selection tree (compatibility log C-047): port of
//! lib/tree.cc (the in-memory directory tree), dird/ua_tree.cc
//! (InsertTreeHandler and the 26 commands of the file selection mode) for
//! the `restore` command.
//!
//! The tree commands are not console commands: they only exist inside the
//! dialog of [`user_select_files_from_tree`], which reads lines with the
//! `$ ` sub-prompt until `done`, `exit`, `quit` or `abort`.
//!
//! Memory: one compact [`Node`] per file or directory (name, sorted child
//! indexes, JobId/FileIndex, flags). Nodes live in one arena whose order is
//! the C++ insertion chain (FirstTreeNode/NextTreeNode); delta parts and
//! hard link references are kept outside the nodes, only where needed.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::ffi::CString;

use bareos_bnet::BnetSignal;

use crate::commands::bvfs::{decode_stat, gid_name, uid_name, Stat};
use crate::sql_list::add_commas;
use crate::ua_select::Ua;

/// lib/tree.h tree_node_type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NodeType {
    Root,
    NewDir,
    Dir,
    /// Windows directory, i.e. no leading slash for the root.
    DirWin,
    File,
}

const EXTRACT: u8 = 1;
const EXTRACT_DESCENDANT: u8 = 2;
const HARD_LINK: u8 = 4;
const SOFT_LINK: u8 = 8;
const INSERTED: u8 = 16;

const ROOT: u32 = 0;
const NO_PARENT: u32 = u32::MAX;
const NO_DELTA: u32 = u32::MAX;

/// lib/tree.h tree_node, kept small: there is one for each file.
struct Node {
    fname: Box<str>,
    /// Children ordered like the C++ rblist (NodeCompare).
    children: Vec<u32>,
    parent: u32,
    file_index: i32,
    job_id: u32,
    delta_seq: i32,
    /// Index into Tree::deltas or NO_DELTA.
    delta: u32,
    node_type: NodeType,
    flags: u8,
}

impl Node {
    fn new(fname: &str, node_type: NodeType, parent: u32) -> Self {
        Self { fname: fname.into(), children: Vec::new(), parent, file_index: 0, job_id: 0,
            delta_seq: -1, delta: NO_DELTA, node_type, flags: 0 }
    }

    fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }

    fn set(&mut self, flag: u8, on: bool) {
        if on {
            self.flags |= flag;
        } else {
            self.flags &= !flag;
        }
    }

    /// TreeNodeHasChild()
    fn has_child(&self) -> bool {
        !self.children.is_empty()
    }

    fn tag(&self, none: &'static str) -> &'static str {
        if self.has(EXTRACT) {
            "*"
        } else if self.has(EXTRACT_DESCENDANT) {
            "+"
        } else {
            none
        }
    }
}

/// NodeCompare (lib/tree.cc): first byte as (signed) char, then strcmp.
fn node_compare(a: &str, b: &str) -> Ordering {
    let first = |s: &str| s.as_bytes().first().map_or(0, |&c| c as i8);
    first(a).cmp(&first(b)).then_with(|| a.as_bytes().cmp(b.as_bytes()))
}

/// fnmatch(pattern, string, 0)
fn fnmatch(pattern: &str, name: &str) -> bool {
    let (Ok(pattern), Ok(name)) = (CString::new(pattern), CString::new(name)) else { return false };
    // SAFETY: both are valid NUL terminated strings.
    unsafe { libc::fnmatch(pattern.as_ptr(), name.as_ptr(), 0) == 0 }
}

/// TreeContext with its TREE_ROOT (dird/ua.h, lib/tree.h).
pub(crate) struct Tree {
    /// Index 0 is the root; 1.. is the insertion chain (FirstTreeNode).
    nodes: Vec<Node>,
    /// Delta parts (JobId, FileIndex) per node, oldest first.
    deltas: Vec<Vec<(u32, i32)>>,
    /// root->hardlinks: (JobId << 32) + FileIndex -> node
    hardlinks: HashMap<u64, u32>,
    cached_path: Option<(String, u32)>,
    /// Current node of the dialog.
    cwd: u32,
    /// Count of inserted rows (user feedback).
    pub cnt: u64,
    pub all: bool,
    pub file_count: u32,
    last_count: u32,
    pub delta_count: u32,
}

impl Tree {
    /// new_tree()
    pub fn new(all: bool) -> Self {
        Self { nodes: vec![Node::new("", NodeType::Root, NO_PARENT)], deltas: Vec::new(),
            hardlinks: HashMap::new(), cached_path: None, cwd: ROOT, cnt: 0, all, file_count: 0,
            last_count: 0, delta_count: 0 }
    }

    fn node(&self, index: u32) -> &Node {
        &self.nodes[index as usize]
    }

    fn node_mut(&mut self, index: u32) -> &mut Node {
        &mut self.nodes[index as usize]
    }

    /// The insertion chain (FirstTreeNode/NextTreeNode).
    fn chain(&self) -> impl Iterator<Item = (u32, &Node)> {
        self.nodes.iter().enumerate().skip(1).map(|(i, n)| (i as u32, n))
    }

    /// search_and_insert_tree_node()
    fn search_and_insert(&mut self, fname: &str, node_type: NodeType, parent: u32) -> u32 {
        let found = {
            let nodes = &self.nodes;
            nodes[parent as usize].children
                .binary_search_by(|&child| node_compare(&nodes[child as usize].fname, fname))
        };
        match found {
            Ok(position) => {
                let index = self.node(parent).children[position];
                self.node_mut(index).set(INSERTED, false);
                index
            }
            Err(position) => {
                let index = self.nodes.len() as u32;
                let mut node = Node::new(fname, node_type, parent);
                node.set(INSERTED, true);
                self.nodes.push(node);
                self.node_mut(parent).children.insert(position, index);
                index
            }
        }
    }

    /// make_tree_path()
    fn make_tree_path(&mut self, path: &str) -> u32 {
        if path.is_empty() {
            return ROOT;
        }
        let (parent, fname, node_type) = match path.rfind('/') {
            Some(p) => (self.make_tree_path(&path[..p]), &path[p + 1..], NodeType::NewDir),
            None => (ROOT, path, NodeType::DirWin),
        };
        self.search_and_insert(fname, node_type, parent)
    }

    /// insert_tree_node(path, fname, type, root, NULL)
    fn insert_tree_node(&mut self, path: &str, fname: &str, node_type: NodeType) -> u32 {
        // If trailing slash on path, strip it
        let path = path.strip_suffix('/').unwrap_or(path);
        // If no filename, strip last component of path as "filename"
        let (path, fname) = match fname.is_empty().then(|| path.rfind('/')).flatten() {
            Some(p) => (&path[..p], &path[p + 1..]),
            None => (path, fname),
        };
        let (fname, parent) = if fname.is_empty() {
            (path, ROOT)
        } else {
            let parent = match &self.cached_path {
                Some((cached, parent)) if cached == path => *parent,
                _ => {
                    let parent = self.make_tree_path(path);
                    self.cached_path = Some((path.to_string(), parent));
                    parent
                }
            };
            (fname, parent)
        };
        self.search_and_insert(fname, node_type, parent)
    }

    /// TreeRemoveNode(): the node leaves its parent; like the C++ tree,
    /// whose node memory is never released here (the name follows it),
    /// it stays in the insertion chain.
    fn remove_node(&mut self, index: u32) {
        let parent = self.node(index).parent;
        self.node_mut(parent).children.retain(|&child| child != index);
    }

    /// InsertTreeHandler (ua_tree.cc:219) for one row of GetFileList:
    /// Path, Name, FileIndex, JobId, LStat, DeltaSeq.
    pub fn insert_row(&mut self, ua: &mut Ua, path: &str, name: &str, file_index: i32, job_id: u32,
        lstat: &str, delta_seq: i32) {
        let node_type = if name.is_empty() {
            if path.starts_with('/') { NodeType::Dir } else { NodeType::DirWin }
        } else {
            NodeType::File
        };
        let stat = decode_stat(lstat);
        let link_fi = stat.link_fi as i32;
        let hard_link = link_fi != 0;
        let index = self.insert_tree_node(path, name, node_type);

        if delta_seq > 0 {
            let node = self.node(index);
            if delta_seq == node.delta_seq + 1 {
                let part = (node.job_id, node.file_index);
                let delta = match node.delta {
                    NO_DELTA => {
                        self.deltas.push(Vec::new());
                        let delta = (self.deltas.len() - 1) as u32;
                        self.node_mut(index).delta = delta;
                        delta
                    }
                    delta => delta,
                };
                self.deltas[delta as usize].push(part);
            } else {
                // File looks to be deleted
                if node.delta_seq == -1 {
                    self.remove_node(index);
                } else {
                    ua.warning_msg(format!("Something is wrong with the Delta sequence of {name}, \
                        skipping new parts. Current sequence is {}\n", node.delta_seq));
                }
                return;
            }
        }

        let node = self.node(index);
        let mut ok = true;
        if !node.has(INSERTED) && job_id == node.job_id
            && ((hard_link && file_index > node.file_index) || (!hard_link && file_index < node.file_index)) {
            ok = false;
        }
        if ok {
            let all = self.all;
            let node = self.node_mut(index);
            node.set(HARD_LINK, hard_link);
            node.file_index = file_index;
            node.job_id = job_id;
            node.node_type = node_type;
            node.set(SOFT_LINK, stat.mode as u32 & libc::S_IFMT == libc::S_IFLNK);
            node.delta_seq = delta_seq;
            if all {
                node.set(EXTRACT, true);
            }
            // Insert file having hardlinks into hardlink hashtable.
            if stat.nlink > 1 && node_type != NodeType::Dir && node_type != NodeType::DirWin {
                let key = (u64::from(job_id) << 32) + file_index as u32 as u64;
                if link_fi == 0 {
                    self.hardlinks.insert(key, index);
                } else {
                    let file_key = (u64::from(job_id) << 32) + link_fi as u32 as u64;
                    if let Some(&first) = self.hardlinks.get(&file_key) {
                        self.hardlinks.insert(key, first);
                    }
                }
            }
        }

        if self.node(index).has(INSERTED) {
            self.file_count += 1;
            if self.delta_count > 0 && self.file_count - self.last_count > self.delta_count {
                ua.send_msg("+");
                self.last_count = self.file_count;
            }
        }
        self.cnt += 1;
    }

    /// tree_getpath()
    fn getpath(&self, index: u32) -> String {
        let mut chain = Vec::new();
        let mut current = index;
        while current != NO_PARENT {
            chain.push(current);
            current = self.node(current).parent;
        }
        let mut path = String::new();
        for &i in chain.iter().rev() {
            let node = self.node(i);
            // Fixup for Win32: a lone / is dropped before a Win32 directory.
            if node.node_type == NodeType::DirWin && path == "/" {
                path.clear();
            }
            path.push_str(&node.fname);
            if (node.node_type != NodeType::File && path != "/")
                || (node.has(SOFT_LINK) && node.has_child()) {
                path.push('/');
            }
        }
        path
    }

    /// tree_cwd()
    fn tree_cwd(&self, path: &str, node: u32) -> Option<u32> {
        if path == "." {
            return Some(node);
        }
        // Handle relative path
        if let Some(rest) = path.strip_prefix("..") {
            if rest.is_empty() || rest.starts_with('/') {
                let parent = match self.node(node).parent {
                    NO_PARENT => node,
                    parent => parent,
                };
                return if rest.is_empty() { Some(parent) } else { self.tree_cwd(&rest[1..], parent) };
            }
        }
        match path.strip_prefix('/') {
            Some(rest) => self.tree_relcwd(rest, ROOT),
            None => self.tree_relcwd(path, node),
        }
    }

    /// tree_relcwd()
    fn tree_relcwd(&self, path: &str, node: u32) -> Option<u32> {
        if path.is_empty() {
            return Some(node);
        }
        let separator = path.find('/');
        let segment = &path[..separator.unwrap_or(path.len())];
        let found = self.node(node).children.iter().copied().find(|&child| {
            let fname = &*self.node(child).fname;
            fname == segment && !segment.is_empty() || fnmatch(segment, fname)
        })?;
        let cd = self.node(found);
        if cd.node_type == NodeType::File && !cd.has_child() {
            return None;
        }
        match separator {
            None => Some(found),
            Some(p) => self.tree_relcwd(&path[p + 1..], found),
        }
    }

    /// SetExtract() (ua_tree.cc:349): returns the number of changed nodes.
    fn set_extract(&mut self, index: u32, extract: bool) -> u32 {
        let mut count = 0;
        let node = self.node(index);
        if node.has(EXTRACT) != extract && node.node_type != NodeType::NewDir && node.node_type != NodeType::Root {
            count += 1;
        }
        self.node_mut(index).set(EXTRACT, extract);

        let node = self.node(index);
        if node.node_type != NodeType::File || (node.has(SOFT_LINK) && node.has_child()) {
            // Recursive set children within directory
            // (the child lists do not change while marking)
            for i in 0..node.children.len() {
                let child = self.node(index).children[i];
                count += self.set_extract(child, extract);
            }
        } else if extract && node.has(HARD_LINK) {
            // Every hardlink is in the hashtable and points to the linked file.
            let key = (u64::from(node.job_id) << 32) + node.file_index as u32 as u64;
            if let Some(&linked) = self.hardlinks.get(&key) {
                count += self.set_extract(linked, extract);
            }
        }

        // Walk up tree marking any unextracted parent to be extracted.
        let mut current = index;
        if extract {
            loop {
                let parent = self.node(current).parent;
                if parent == NO_PARENT || self.node(parent).has(EXTRACT_DESCENDANT) {
                    break;
                }
                current = parent;
                self.node_mut(current).set(EXTRACT_DESCENDANT, true);
            }
        } else {
            loop {
                let parent = self.node(current).parent;
                if parent == NO_PARENT || !self.node(parent).has(EXTRACT_DESCENDANT) {
                    break;
                }
                current = parent;
                let descendant = self.node(current).children.iter()
                    .any(|&c| self.node(c).has(EXTRACT) || self.node(c).has(EXTRACT_DESCENDANT));
                self.node_mut(current).set(EXTRACT_DESCENDANT, descendant);
            }
        }
        count
    }

    /// MarkElements() (ua_tree.cc:447)
    fn mark_elements(&mut self, args: &[String], extract: bool) -> u32 {
        let mut count = 0;
        for arg in &args[1..] {
            let arg = strip_trailing_slash(arg);
            let mut parts = split_path(arg);
            let mut stack: Vec<(u32, usize)> = Vec::new();
            if parts[0].is_empty() {
                // we are trying to mark from the root
                if parts.len() == 1 {
                    parts.push("*".into());
                }
                stack.push((ROOT, 1));
            } else if parts[0].len() == 2 && parts[0].as_bytes()[1] == b':' {
                // maybe we are trying to mark from a windows root
                let mut found_root = false;
                for &child in &self.node(ROOT).children {
                    let node = self.node(child);
                    if node.node_type == NodeType::DirWin && fnmatch(&parts[0], &node.fname) {
                        stack.push((child, 1));
                        found_root = true;
                    }
                }
                if found_root {
                    if parts.len() == 1 {
                        parts.push("*".into());
                    }
                } else {
                    stack.push((self.cwd, 0));
                }
            } else {
                stack.push((self.cwd, 0));
            }

            while let Some((node, part_index)) = stack.pop() {
                let part = &parts[part_index];
                let children = self.node(node).children.len();
                let child_at = |tree: &Self, i: usize| tree.node(node).children[i];
                if part == "**" {
                    // ** inside a path: zero or more subdirectories
                    stack.push((node, part_index + 1));
                    for i in 0..children {
                        let child = child_at(self, i);
                        if self.node(child).node_type != NodeType::File {
                            stack.push((child, part_index));
                        }
                    }
                } else {
                    for i in 0..children {
                        let child = child_at(self, i);
                        if fnmatch(part, &self.node(child).fname) {
                            if part_index + 1 == parts.len() {
                                count += self.set_extract(child, extract);
                            } else if self.node(child).node_type != NodeType::File {
                                stack.push((child, part_index + 1));
                            }
                        }
                    }
                }
            }
        }
        count
    }

    /// Nodes marked for extraction, in chain order: (JobId, FileIndex,
    /// delta parts oldest first, counts as selected file).
    pub fn for_each_extracted(&self, mut f: impl FnMut(u32, i32, &[(u32, i32)], bool)) {
        for (_, node) in self.chain() {
            if node.has(EXTRACT) {
                let deltas = match node.delta {
                    NO_DELTA => &[][..],
                    delta => &self.deltas[delta as usize][..],
                };
                f(node.job_id, node.file_index, deltas, node.node_type != NodeType::NewDir);
            }
        }
    }
}

/// StripTrailingSlash()
fn strip_trailing_slash(arg: &str) -> &str {
    arg.strip_suffix('/').unwrap_or(arg)
}

/// split_path() (ua_tree.cc:417)
fn split_path(mut v: &str) -> Vec<String> {
    let mut parts = Vec::new();
    loop {
        match v.find('/') {
            None => {
                // ** at the end of a path is treated the same as *
                parts.push(if v.is_empty() || v == "**" { "*".to_string() } else { v.to_string() });
                break;
            }
            Some(pos) => {
                parts.push(v[..pos].to_string());
                v = &v[pos + 1..];
            }
        }
    }
    parts
}

// --- the dialog ---------------------------------------------------------------

type Command = fn(&mut Ua, &mut Tree, &[String]) -> bool;

/// ua_tree.cc commands[]: key, function, help, audit_event.
const COMMANDS: [(&str, Command, &str, bool); 26] = [
    ("abort", quit_cmd, "abort and do not do restore", true),
    ("add", mark_cmd, "add dir/file to be restored recursively, wildcards allowed", true),
    ("cd", cd_cmd, "change current directory", true),
    ("count", count_cmd, "count marked files in and below the cd", false),
    ("delete", unmark_cmd, "delete dir/file to be restored recursively in dir", true),
    ("dir", dir_cmd, "long list current directory, wildcards allowed", false),
    (".dir", dot_dir_cmd, "long list current directory, wildcards allowed", false),
    ("done", done_cmd, "leave file selection mode", true),
    ("estimate", estimate_cmd, "estimate restore size", false),
    ("exit", done_cmd, "same as done command", true),
    ("find", find_cmd, "find files, wildcards allowed", false),
    ("help", help_cmd, "print help", false),
    ("ls", ls_cmd, "list current directory, wildcards allowed", false),
    (".ls", dot_ls_cmd, "list current directory, wildcards allowed", false),
    (".lsdir", dot_lsdir_cmd, "list subdir in current directory, wildcards allowed", false),
    ("lsmark", lsmark_cmd, "list the marked files in and below the cd", false),
    (".lsmark", dot_lsmark_cmd, "list the marked files in", false),
    ("mark", mark_cmd, "mark dir/file to be restored recursively, wildcards allowed", true),
    ("markdir", markdir_cmd, "mark directory name to be restored (no files)", true),
    ("pwd", pwd_cmd, "print current working directory", false),
    (".pwd", dot_pwd_cmd, "print current working directory", false),
    ("unmark", unmark_cmd, "unmark dir/file to be restored recursively in dir", true),
    ("unmarkdir", unmarkdir_cmd, "unmark directory name only no recursion", true),
    ("quit", quit_cmd, "quit and do not do restore", true),
    (".help", dot_help_cmd, "print help", false),
    ("?", help_cmd, "print help", false),
];

thread_local! {
    /// Set by quit/abort (C++ `ua->quit` of the tree's UaContext).
    static QUIT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// `user->signal()`: only with a console (a signal without one would drop
/// the buffered output).
fn signal(ua: &mut Ua, signal: BnetSignal) {
    if ua.has_console() {
        ua.signal(signal);
    }
}

/// ParseArgsOnly(): arguments without keyword/value split, at most
/// MAX_CMD_ARGS, ending at the first empty one.
fn parse_args_only(line: &str) -> Vec<String> {
    crate::session::split_args(line).into_iter().take_while(|a| !a.is_empty()).take(30).collect()
}

/// UserSelectFilesFromTree (ua_tree.cc:123): false when the user quit.
pub(crate) fn user_select_files_from_tree(ua: &mut Ua, tree: &mut Tree) -> bool {
    ua.send_msg("\nYou are now entering file selection mode where you add (mark) and\n\
        remove (unmark) files to be restored. No files are initially added, unless\n\
        you used the \"all\" keyword on the command line.\n\
        Enter \"done\" to leave this mode.\n\n");
    signal(ua, BnetSignal::StartRtree);

    // Enter interactive command handler allowing selection of individual files.
    tree.cwd = ROOT;
    ua.send_msg(format!("cwd is: {}\n", tree.getpath(tree.cwd)));

    QUIT.with(|q| q.set(false));
    let api = ua.api();
    loop {
        let Some(line) = ua.get_cmd_ext("$ ", true) else { break };
        if api {
            signal(ua, BnetSignal::CmdBegin);
        }
        let args = parse_args_only(&line);
        if args.is_empty() {
            ua.warning_msg(format!("Invalid command \"{line}\". Enter \"done\" to exit.\n"));
            if api {
                signal(ua, BnetSignal::CmdFailed);
            }
            continue;
        }
        // bstrncasecmp(argk[0], key, strlen(argk[0])): a prefix of the key
        let typed = args[0].to_ascii_lowercase();
        let command = COMMANDS.iter().find(|(key, ..)| key.starts_with(&typed));
        let Some(&(_, func, _, audit_event)) = command else {
            if typed.starts_with('.') {
                // Some unknown dot command -- probably .messages, ignore it
                continue;
            }
            ua.warning_msg(format!("Invalid command \"{line}\". Enter \"done\" to exit.\n"));
            if api {
                signal(ua, BnetSignal::CmdFailed);
            }
            continue;
        };
        crate::audit::tree_command(ua.ctx, audit_event, &args[0], &line);
        let status = func(ua, tree, &args);
        if api {
            signal(ua, BnetSignal::CmdOk);
        }
        if !status {
            break;
        }
    }
    signal(ua, BnetSignal::EndRtree);
    !QUIT.with(|q| q.replace(false))
}

fn newly(ua: &mut Ua, count: u32, none: &str, one: &str, many: &str) {
    match count {
        0 => ua.send_msg(none),
        1 => ua.send_msg(one),
        n => ua.send_msg(format!("{} {many}", add_commas(&n.to_string()))),
    }
}

fn mark_cmd(ua: &mut Ua, tree: &mut Tree, args: &[String]) -> bool {
    if args.len() < 2 || !tree.node(tree.cwd).has_child() {
        ua.send_msg("No files marked.\n");
        return true;
    }
    let count = tree.mark_elements(args, true);
    newly(ua, count, "No files newly marked.\n", "1 file newly marked.\n", "files newly marked.\n");
    true
}

fn unmark_cmd(ua: &mut Ua, tree: &mut Tree, args: &[String]) -> bool {
    if args.len() < 2 || !tree.node(tree.cwd).has_child() {
        ua.send_msg("No files newly unmarked.\n");
        return true;
    }
    let count = tree.mark_elements(args, false);
    newly(ua, count, "No files newly unmarked.\n", "1 file newly unmarked.\n", "files newly unmarked.\n");
    true
}

/// Markdircmd/UnMarkdircmd: directories in the cwd matching an argument.
fn set_dirs(tree: &mut Tree, args: &[String], extract: bool) -> u32 {
    let mut count = 0;
    for arg in &args[1..] {
        let arg = strip_trailing_slash(arg);
        let children = tree.node(tree.cwd).children.clone();
        for child in children {
            let node = tree.node_mut(child);
            if fnmatch(arg, &node.fname) && matches!(node.node_type, NodeType::Dir | NodeType::DirWin) {
                node.set(EXTRACT, extract);
                count += 1;
            }
        }
    }
    count
}

fn markdir_cmd(ua: &mut Ua, tree: &mut Tree, args: &[String]) -> bool {
    if args.len() < 2 || !tree.node(tree.cwd).has_child() {
        ua.send_msg("No directories marked.\n");
        return true;
    }
    let count = set_dirs(tree, args, true);
    newly(ua, count, "No directories newly marked.\n", "1 directory newly marked.\n",
        "directories newly marked.\n");
    true
}

fn unmarkdir_cmd(ua: &mut Ua, tree: &mut Tree, args: &[String]) -> bool {
    if args.len() < 2 || !tree.node(tree.cwd).has_child() {
        ua.send_msg("No directories newly unmarked.\n");
        return true;
    }
    match set_dirs(tree, args, false) {
        0 => ua.send_msg("No directories newly unmarked.\n"),
        1 => ua.send_msg("1 directory newly unmarked.\n"),
        // "%d" without thousands separators
        n => ua.send_msg(format!("{n} directories newly unmarked.\n")),
    }
    true
}

fn count_cmd(ua: &mut Ua, tree: &mut Tree, _: &[String]) -> bool {
    let (mut total, mut extract) = (0u64, 0u64);
    for (_, node) in tree.chain() {
        if node.node_type != NodeType::NewDir {
            total += 1;
            if node.has(EXTRACT) {
                extract += 1;
            }
        }
    }
    ua.send_msg(format!("{} total files/dirs. {} marked to be restored.\n",
        add_commas(&total.to_string()), add_commas(&extract.to_string())));
    true
}

fn find_cmd(ua: &mut Ua, tree: &mut Tree, args: &[String]) -> bool {
    if args.len() == 1 {
        ua.send_msg("No file specification given.\n");
        return true;
    }
    for arg in &args[1..] {
        for (index, node) in tree.chain() {
            if fnmatch(arg, &node.fname) {
                ua.send_msg(format!("{}{}\n", node.tag(""), tree.getpath(index)));
            }
        }
    }
    true
}

/// The children of the cwd matching the optional pattern argk[1].
fn matching(tree: &Tree, args: &[String]) -> Vec<u32> {
    tree.node(tree.cwd).children.iter().copied()
        .filter(|&child| args.len() == 1 || fnmatch(&args[1], &tree.node(child).fname))
        .collect()
}

fn slash(node: &Node) -> &'static str {
    if node.has_child() { "/" } else { "" }
}

fn dot_lsdir_cmd(ua: &mut Ua, tree: &mut Tree, args: &[String]) -> bool {
    for child in matching(tree, args) {
        let node = tree.node(child);
        if node.has_child() {
            ua.send_msg(format!("{}/\n", node.fname));
        }
    }
    true
}

fn dot_help_cmd(ua: &mut Ua, _: &mut Tree, _: &[String]) -> bool {
    for (key, ..) in COMMANDS.iter().filter(|(key, ..)| !key.starts_with('.')) {
        ua.send_msg(format!("{key}\n"));
    }
    true
}

fn dot_ls_cmd(ua: &mut Ua, tree: &mut Tree, args: &[String]) -> bool {
    for child in matching(tree, args) {
        let node = tree.node(child);
        ua.send_msg(format!("{}{}\n", node.fname, slash(node)));
    }
    true
}

fn ls_cmd(ua: &mut Ua, tree: &mut Tree, args: &[String]) -> bool {
    for child in matching(tree, args) {
        let node = tree.node(child);
        ua.send_msg(format!("{}{}{}\n", node.tag(""), node.fname, slash(node)));
    }
    true
}

fn dot_lsmark_cmd(ua: &mut Ua, tree: &mut Tree, args: &[String]) -> bool {
    for child in matching(tree, args) {
        let node = tree.node(child);
        if node.has(EXTRACT) {
            ua.send_msg(format!("{}{}\n", node.fname, slash(node)));
        }
    }
    true
}

/// rlsmark(): recursive list of the marked files.
fn rlsmark(ua: &mut Ua, tree: &Tree, index: u32, level: usize, args: &[String]) {
    let indent = "  ".repeat(level.min(100));
    for &child in &tree.node(index).children {
        let node = tree.node(child);
        if (args.len() == 1 || fnmatch(&args[1], &node.fname))
            && (node.has(EXTRACT) || node.has(EXTRACT_DESCENDANT)) {
            ua.send_msg(format!("{indent}{}{}{}\n", node.tag(""), node.fname, slash(node)));
            if node.has_child() {
                rlsmark(ua, tree, child, level + 1, args);
            }
        }
    }
}

fn lsmark_cmd(ua: &mut Ua, tree: &mut Tree, args: &[String]) -> bool {
    rlsmark(ua, tree, tree.cwd, 0, args);
    true
}

/// encode_mode() (lib/util.cc)
fn encode_mode(mode: u32) -> String {
    let file_type = match mode & libc::S_IFMT {
        libc::S_IFDIR => 'd',
        libc::S_IFBLK => 'b',
        libc::S_IFCHR => 'c',
        libc::S_IFLNK => 'l',
        libc::S_IFIFO => 'f',
        libc::S_IFSOCK => 's',
        _ => '-',
    };
    let bit = |m: u32, c: char| if mode & m != 0 { c } else { '-' };
    let exec = |special: u32, x: u32, set: char, unset: char| match (mode & special != 0, mode & x != 0) {
        (true, true) => set,
        (true, false) => unset,
        (false, true) => 'x',
        (false, false) => '-',
    };
    [
        file_type,
        bit(libc::S_IRUSR, 'r'), bit(libc::S_IWUSR, 'w'), exec(libc::S_ISUID, libc::S_IXUSR, 's', 'S'),
        bit(libc::S_IRGRP, 'r'), bit(libc::S_IWGRP, 'w'), exec(libc::S_ISGID, libc::S_IXGRP, 's', 'S'),
        bit(libc::S_IROTH, 'r'), bit(libc::S_IWOTH, 'w'), exec(libc::S_ISVTX, libc::S_IXOTH, 't', 'T'),
    ].iter().collect()
}

/// encode_time(): local time "YYYY-MM-DD HH:MM:SS".
fn encode_time(time: i64) -> String {
    use chrono::TimeZone;
    match chrono::Local.timestamp_opt(time, 0) {
        chrono::LocalResult::Single(t) | chrono::LocalResult::Ambiguous(t, _) =>
            t.format("%Y-%m-%d %H:%M:%S").to_string(),
        chrono::LocalResult::None => String::new(),
    }
}

/// Catalog lookups of the tree commands (BareosDb::GetFileAttributesRecord
/// with the PathId cache of GetPathRecord).
#[derive(Default)]
struct FileAttributes {
    cached_path: Option<(String, i64)>,
}

impl FileAttributes {
    /// GetFileAttributesRecord(filename) for `job_id`: the decoded LStat,
    /// None when there is no record.
    fn get(&mut self, ua: &Ua, filename: &str, job_id: u32) -> Option<Stat> {
        let catalog = ua.ctx.catalog.as_ref()?;
        // SplitPathAndFile(): everything after the last / is the file name.
        let split = filename.rfind('/').map_or(0, |p| p + 1);
        let (path, fname) = filename.split_at(split);
        let escape = |s: &str| s.replace('\'', "''");
        let path_id = match &self.cached_path {
            Some((cached, id)) if cached == path => *id,
            _ => {
                let table = catalog.query_rows(&format!("SELECT PathId FROM Path WHERE Path='{}'", escape(path))).ok()?;
                let id = table.rows.first().and_then(|r| r.first().cloned().flatten())
                    .and_then(|v| v.parse::<i64>().ok()).unwrap_or(0);
                if id > 0 {
                    self.cached_path = Some((path.to_string(), id));
                }
                id
            }
        };
        let table = catalog.query_rows(&format!(
            "SELECT FileId, LStat, MD5, Fhinfo, Fhnode FROM File WHERE File.JobId={job_id} AND File.PathId={path_id} \
             AND File.Name='{}'", escape(fname))).ok()?;
        let row = table.rows.first()?;
        Some(decode_stat(row.get(1).cloned().flatten().as_deref().unwrap_or("")))
    }
}

/// ls_output(): the line of `dir` (dot_cmd: `.dir`).
fn ls_output(path: &str, tag: &str, stat: &Stat, dot_cmd: bool) -> String {
    let mode = encode_mode(stat.mode as u32);
    let tag = tag.chars().next().unwrap_or(' ');
    let (uid, gid) = (stat.uid, stat.gid);
    let nlink = stat.nlink as u32 as i32;
    if dot_cmd {
        format!("{mode},{nlink},{uid}({}),{gid}({}),{},{},{tag},{path}", uid_name(uid), gid_name(gid),
            stat.size, encode_time(stat.mtime))
    } else {
        // Display most recent time
        let time = if stat.ctime > stat.mtime { stat.ctime } else { stat.mtime };
        let short = |s: String| s.chars().take(8).collect::<String>();
        let size: String = stat.size.to_string().chars().take(12).collect();
        format!("{mode}  {nlink:2} {uid} ({}) {gid} ({})  {size:>12}  {} {tag} {path}",
            short(uid_name(uid)), short(gid_name(gid)), encode_time(time))
    }
}

/// DoDircmd()
fn do_dir_cmd(ua: &mut Ua, tree: &mut Tree, args: &[String], dot_cmd: bool) -> bool {
    let cwd = tree.node(tree.cwd);
    if !cwd.has_child() {
        ua.send_msg(format!("Node {} has no children.\n", cwd.fname));
        return true;
    }
    let mut attributes = FileAttributes::default();
    for child in matching(tree, args) {
        let node = tree.node(child);
        let tag = node.tag(" ");
        let path = tree.getpath(child);
        // Strip / from soft links to directories.
        let lookup = if node.node_type == NodeType::File && node.has_child() && path.len() > 1 {
            &path[..path.len() - 1]
        } else {
            &path[..]
        };
        let stat = attributes.get(ua, lookup, node.job_id).unwrap_or_default();
        ua.send_msg(format!("{}\n", ls_output(&path, tag, &stat, dot_cmd)));
    }
    true
}

fn dir_cmd(ua: &mut Ua, tree: &mut Tree, args: &[String]) -> bool {
    do_dir_cmd(ua, tree, args, false)
}

fn dot_dir_cmd(ua: &mut Ua, tree: &mut Tree, args: &[String]) -> bool {
    do_dir_cmd(ua, tree, args, true)
}

fn estimate_cmd(ua: &mut Ua, tree: &mut Tree, _: &[String]) -> bool {
    let (mut total, mut extract, mut bytes) = (0i64, 0i64, 0u64);
    let mut attributes = FileAttributes::default();
    for (index, node) in tree.chain() {
        if node.node_type == NodeType::NewDir {
            continue;
        }
        total += 1;
        if node.has(EXTRACT) && node.node_type == NodeType::File {
            // If regular file, get size
            extract += 1;
            if let Some(stat) = attributes.get(ua, &tree.getpath(index), node.job_id) {
                if stat.mode as u32 & libc::S_IFMT == libc::S_IFREG && stat.size > 0 {
                    bytes += stat.size as u64;
                }
            }
        } else if node.has(EXTRACT) || node.has(EXTRACT_DESCENDANT) {
            // Directory, count only
            extract += 1;
        }
    }
    ua.send_msg(format!("{} total files; {} marked to be restored; {} bytes.\n",
        total as i32, extract as i32, add_commas(&bytes.to_string())));
    true
}

fn help_cmd(ua: &mut Ua, _: &mut Tree, _: &[String]) -> bool {
    ua.send_msg("  Command    Description\n  =======    ===========\n");
    for (key, _, help, _) in COMMANDS.iter().filter(|(key, ..)| !key.starts_with('.')) {
        ua.send_msg(format!("  {key:<10} {help}\n"));
    }
    ua.send_msg("\n");
    true
}

/// cdcmd(): with the Win32 retry as absolute path for `x:`.
fn cd_cmd(ua: &mut Ua, tree: &mut Tree, args: &[String]) -> bool {
    if args.len() != 2 {
        ua.error_msg("Too few or too many arguments. Try using double quotes.\n");
        return true;
    }
    let mut node = tree.tree_cwd(&args[1], tree.cwd);
    if node.is_none() && args[1].as_bytes().get(1) == Some(&b':') {
        node = tree.tree_cwd(&format!("/{}", args[1]), tree.cwd);
    }
    match node {
        Some(node) => tree.cwd = node,
        None => ua.warning_msg("Invalid path given.\n"),
    }
    pwd_cmd(ua, tree, args)
}

fn pwd_cmd(ua: &mut Ua, tree: &mut Tree, _: &[String]) -> bool {
    let cwd = tree.getpath(tree.cwd);
    if ua.api() {
        ua.send_msg(cwd);
    } else {
        ua.send_msg(format!("cwd is: {cwd}\n"));
    }
    true
}

fn dot_pwd_cmd(ua: &mut Ua, tree: &mut Tree, _: &[String]) -> bool {
    ua.send_msg(tree.getpath(tree.cwd));
    true
}

fn done_cmd(_: &mut Ua, _: &mut Tree, _: &[String]) -> bool {
    false
}

fn quit_cmd(_: &mut Ua, _: &mut Tree, _: &[String]) -> bool {
    QUIT.with(|q| q.set(true));
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "P0C BHt B A A A ByK BAA Q BqvQnm BqOal9 BqUv/U A A C";
    const DIR: &str = "d Tn EHt B A A A DBe BAA A Bq0Qxk Bq0Qxc Bq0Qxc A A C";

    fn tree(rows: &[(&str, &str, i32, u32)], all: bool) -> Tree {
        let ctx = crate::ua_select::tests::context("Director { Name=dir; Password=x }",
            crate::command_executor::ApiMode::Human);
        let mut ua = Ua::new(&ctx);
        let mut tree = Tree::new(all);
        for &(path, name, fi, job) in rows {
            tree.insert_row(&mut ua, path, name, fi, job, if name.is_empty() { DIR } else { FILE }, 0);
        }
        tree
    }

    #[test]
    fn insertion_and_paths() {
        let t = tree(&[("/usr/sbin/", "zic", 1, 1), ("/usr/sbin/", "apache2", 2, 1), ("/usr/sbin/", "", 3, 1)], false);
        // usr and sbin are created as NewDir; the directory row finds sbin.
        assert_eq!(t.file_count, 2);
        assert_eq!(t.cnt, 3);
        let names: Vec<String> = t.chain().map(|(i, _)| t.getpath(i)).collect();
        assert_eq!(names, ["/usr/", "/usr/sbin/", "/usr/sbin/zic", "/usr/sbin/apache2"]);
        assert_eq!(t.getpath(ROOT), "/");
        let sbin = t.tree_cwd("/usr/sbin", ROOT).unwrap();
        assert_eq!(t.node(sbin).node_type, NodeType::Dir);
        let children: Vec<&str> = t.node(sbin).children.iter().map(|&c| &*t.node(c).fname).collect();
        assert_eq!(children, ["apache2", "zic"]);
        assert_eq!(t.tree_cwd("..", sbin), t.tree_cwd("/usr", ROOT));
        assert_eq!(t.tree_cwd("/usr/s*", ROOT), Some(sbin));
        assert_eq!(t.tree_cwd("/usr/sbin/zic", ROOT), None);
        assert_eq!(t.tree_cwd("..", ROOT), Some(ROOT));
    }

    #[test]
    fn marking() {
        let mut t = tree(&[("/usr/sbin/", "zic", 1, 1), ("/usr/sbin/", "apache2", 2, 1), ("/usr/sbin/", "", 3, 1)], false);
        let args = |s: &str| parse_args_only(s);
        assert_eq!(t.mark_elements(&args("mark usr"), true), 3);
        assert_eq!(t.mark_elements(&args("mark usr/sbin/*"), true), 0);
        assert_eq!(t.mark_elements(&args("unmark /usr/**/zic"), false), 1);
        let mut selected = Vec::new();
        t.for_each_extracted(|job, fi, _, counted| selected.push((job, fi, counted)));
        // usr is a NewDir: marked, but no saved file (FileIndex 0)
        assert_eq!(selected, [(0, 0, false), (1, 3, true), (1, 2, true)]);
        let usr = t.tree_cwd("/usr", ROOT).unwrap();
        assert!(t.node(usr).has(EXTRACT_DESCENDANT) && t.node(ROOT).has(EXTRACT_DESCENDANT));
        assert_eq!(t.mark_elements(&args("unmark *"), false), 2);
        assert!(!t.node(ROOT).has(EXTRACT_DESCENDANT));
    }

    #[test]
    fn helpers() {
        assert_eq!(split_path("a/**"), ["a", "*"]);
        assert_eq!(split_path("/"), ["", "*"]);
        assert_eq!(split_path("a/**/b"), ["a", "**", "b"]);
        assert_eq!(encode_mode(0o104755), "-rwsr-xr-x");
        assert_eq!(encode_mode(0o41777), "drwxrwxrwt");
        assert_eq!(encode_mode(0), "----------");
        assert_eq!(node_compare("\u{e9}", "a"), Ordering::Less);
        assert_eq!(parse_args_only("mark \"a b\" c=d \"\" e"), ["mark", "a b", "c=d"]);
        let stat = Stat::default();
        let line = ls_output("/usr/", " ", &stat, true);
        assert!(line.starts_with("----------,0,0(root),0(root),0,") && line.ends_with(", ,/usr/"), "{line}");
        let line = ls_output("/usr/", "+", &stat, false);
        assert!(line.starts_with("----------   0 0 (root) 0 (root)             0  ") && line.ends_with(" + /usr/"), "{line}");
    }
}
