//! Console dialogs: port of C++ dird/ua_input.cc (GetCmd, GetPint,
//! GetYesno, IsYesno, GetConfirmation) and the prompt machinery of
//! ua_select.cc (StartPrompt/AddPrompt/DoPrompt, FormatPrompts,
//! DoKeywordPrompt, select_*_resource).
//!
//! Command handlers stay synchronous functions returning a CommandResult.
//! A handler that needs input mid-command creates a [`Ua`] for its output:
//! messages are buffered, and only when the handler asks the console for
//! input is the buffer sent (in C++ order, with the API-mode message
//! signals) followed by the prompt. The console session attaches its BNET
//! connection for the duration of one command with [`with_console`]; any
//! other caller (gRPC command queue, runscripts, unit tests) has no console
//! and gets batch-mode behaviour (C++ `ua->batch`): GetCmd fails and
//! DoPrompt lists the choices and reports that selection is impossible.

use std::cell::Cell;
use std::io::{Read, Write};

use bareos_bnet::{BnetMessage, BnetSignal, BnetStream};

use crate::acl::{self, Kind};
use crate::command_ast::CommandAst;
use crate::command_executor::{ApiMode, ExecContext};
use crate::command_queue::CommandResult;
use crate::director_config::sort_case_insensitive;

/// The console connection a dialog talks to.
pub trait Console {
    /// Send one data message; false when the connection is gone.
    fn send(&mut self, text: &str) -> bool;
    /// Send one signal; false when the connection is gone.
    fn signal(&mut self, signal: BnetSignal) -> bool;
    /// Next data message from the console; signals are skipped (C++ GetCmd
    /// `if (status == BNET_SIGNAL) continue;`). None on error or terminate.
    fn recv(&mut self) -> Option<String>;
}

impl<S: Read + Write> Console for BnetStream<S> {
    fn send(&mut self, text: &str) -> bool {
        self.send_str(text).is_ok()
    }

    fn signal(&mut self, signal: BnetSignal) -> bool {
        self.send_signal(signal).is_ok()
    }

    fn recv(&mut self) -> Option<String> {
        loop {
            match BnetStream::recv(self).ok()? {
                BnetMessage::Data(data) => return Some(String::from_utf8_lossy(&data).into_owned()),
                BnetMessage::Signal(BnetSignal::Terminate) => return None,
                BnetMessage::Signal(_) => continue,
            }
        }
    }
}

type ConsolePtr = *mut (dyn Console + 'static);

thread_local! {
    /// Console of the command running on this thread (console sessions run
    /// each command synchronously on their own thread).
    static CONSOLE: Cell<Option<ConsolePtr>> = const { Cell::new(None) };
}

/// Run `f` (one command dispatch) with `console` available to dialogs.
pub fn with_console<R>(console: &mut dyn Console, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<ConsolePtr>);
    impl Drop for Restore {
        fn drop(&mut self) {
            CONSOLE.with(|c| c.set(self.0));
        }
    }
    let ptr: *mut (dyn Console + '_) = console;
    // SAFETY: only the lifetime bound is erased. The pointer is stored for
    // the duration of `f` only and removed again by `Restore` (also on
    // unwind), so it is never used after `console`'s borrow ends.
    let ptr: ConsolePtr = unsafe { std::mem::transmute(ptr) };
    let _restore = Restore(CONSOLE.with(|c| c.replace(Some(ptr))));
    f()
}

fn has_console() -> bool {
    CONSOLE.with(|c| c.get().is_some())
}

/// Use the attached console. The pointer is taken out while in use, so no
/// second mutable reference can exist.
fn with_attached<R>(f: impl FnOnce(&mut dyn Console) -> R) -> Option<R> {
    let ptr = CONSOLE.with(|c| c.take())?;
    // SAFETY: the pointer was installed by `with_console`, whose borrow is
    // still alive (we are inside its `f`), and it is exclusively ours while
    // taken out of the cell.
    let result = f(unsafe { &mut *ptr });
    CONSOLE.with(|c| c.set(Some(ptr)));
    Some(result)
}

/// Message kinds of UaContext (SendMsg, ErrorMsg, WarningMsg, InfoMsg).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Text(String),
    Error(String),
    Warning(String),
    Info(String),
}

/// Output and input of one command, the counterpart of C++ UaContext.
pub struct Ua<'a> {
    pub ctx: &'a ExecContext,
    segments: Vec<Segment>,
    /// Something went to the console directly (a prompt was shown).
    interacted: bool,
    /// StartPrompt/AddPrompt list: header first, then the unique items.
    prompts: Vec<String>,
    /// Last answer of GetCmd (C++ `ua->cmd`).
    pub cmd: String,
}

impl<'a> Ua<'a> {
    pub fn new(ctx: &'a ExecContext) -> Self {
        Self { ctx, segments: Vec::new(), interacted: false, prompts: Vec::new(), cmd: String::new() }
    }

    /// C++ `ua->api` (set for `.api 1` and `.api 2`).
    pub fn api(&self) -> bool {
        self.ctx.api_mode != ApiMode::Human
    }

    /// C++ `ua->batch`: JSON mode (DotApiCmd) or no console at all.
    pub fn batch(&self) -> bool {
        self.ctx.api_mode == ApiMode::Json || !has_console()
    }

    /// True when a console is attached (C++ `ua->UA_sock != nullptr`).
    pub fn has_console(&self) -> bool {
        has_console()
    }

    pub fn send_msg(&mut self, text: impl Into<String>) {
        let text = text.into();
        if !text.is_empty() {
            self.segments.push(Segment::Text(text));
        }
    }

    pub fn error_msg(&mut self, text: impl Into<String>) {
        self.segments.push(Segment::Error(text.into()));
    }

    pub fn warning_msg(&mut self, text: impl Into<String>) {
        self.segments.push(Segment::Warning(text.into()));
    }

    pub fn info_msg(&mut self, text: impl Into<String>) {
        self.segments.push(Segment::Info(text.into()));
    }

    /// Append a result produced by not yet ported code (text, then errors
    /// or the other way round as recorded).
    pub fn send_result(&mut self, result: CommandResult) {
        let text = (!result.text_output.is_empty()).then(|| Segment::Text(result.text_output));
        let error = (!result.error_message.is_empty()).then(|| Segment::Error(result.error_message));
        let parts = if result.errors_first { [error, text] } else { [text, error] };
        self.segments.extend(parts.into_iter().flatten());
    }

    /// C++ `if (!OpenClientDb(ua, use_private)) return ...`: messages of the
    /// catalog selection go into this command's output.
    pub fn open_client_db(&mut self, ast: &CommandAst, use_private: bool) -> bool {
        match crate::ua_db::open_client_db(ast, self.ctx, use_private) {
            Ok(prefix) => {
                self.send_msg(prefix);
                true
            }
            Err(result) => {
                self.send_result(result);
                false
            }
        }
    }

    /// Send the buffered messages to the console (UaContext::vSendMsg: in
    /// API mode a message signal precedes ErrorMsg/WarningMsg/InfoMsg).
    fn flush(&mut self) -> bool {
        let api = self.api();
        let segments = std::mem::take(&mut self.segments);
        self.interacted = true;
        with_attached(|console| {
            segments.iter().all(|segment| {
                let (signal, text) = match segment {
                    Segment::Text(t) => (None, t),
                    Segment::Error(t) => (Some(BnetSignal::ErrorMsg), t),
                    Segment::Warning(t) => (Some(BnetSignal::WarningMsg), t),
                    Segment::Info(t) => (Some(BnetSignal::InfoMsg), t),
                };
                (!api || signal.is_none_or(|s| console.signal(s))) && console.send(text)
            })
        })
        .unwrap_or(false)
    }

    fn signal(&mut self, signal: BnetSignal) -> bool {
        self.flush() && with_attached(|c| c.signal(signal)).unwrap_or(false)
    }

    /// C++ GetCmd(ua, prompt, subprompt=false).
    pub fn get_cmd(&mut self, prompt: &str) -> Option<String> {
        self.get_cmd_ext(prompt, false)
    }

    /// C++ GetCmd(): no console or batch mode fails at once. `.api` clients
    /// get BNET_TEXT_INPUT before the prompt (unless `subprompt`), plain
    /// consoles BNET_SUB_PROMPT after it. Trailing junk is stripped; a lone
    /// dot or a lost connection fails. The answer is also kept in `cmd`.
    pub fn get_cmd_ext(&mut self, prompt: &str, subprompt: bool) -> Option<String> {
        self.cmd.clear();
        if self.batch() {
            return None;
        }
        let api = self.api();
        if !subprompt && api && !self.signal(BnetSignal::TextInput) {
            return None;
        }
        self.send_msg(prompt);
        if !self.flush() {
            return None;
        }
        if (!api || subprompt) && !with_attached(|c| c.signal(BnetSignal::SubPrompt)).unwrap_or(false) {
            return None;
        }
        let answer = with_attached(|c| c.recv()).flatten()?;
        let answer = answer.trim_end_matches(|c: char| c.is_ascii_whitespace()).to_string();
        self.cmd = answer.clone();
        // TODO(C-011): GetCmd runs DotMessagesCmd for an answer ".messages".
        (answer != ".").then_some(answer)
    }

    /// C++ GetPint(): a positive number (Is_a_number + strtod), asked again
    /// with a warning until valid. An empty answer to "Enter slot..." is 0.
    pub fn get_pint(&mut self, prompt: &str) -> Option<u32> {
        loop {
            let answer = self.get_cmd(prompt)?;
            if answer.is_empty() && prompt.starts_with("Enter slot") {
                return Some(0);
            }
            match parse_number(&answer) {
                Some(value) if value >= 0.0 => return Some(value.min(u32::MAX as f64) as u32),
                _ => self.warning_msg(format!("Expected a positive integer, got: {answer}\n")),
            }
        }
    }

    /// C++ GetYesno(): BNET_YESNO before each question in API mode; answers
    /// longer than three characters or empty are asked again silently.
    pub fn get_yesno(&mut self, prompt: &str) -> Option<bool> {
        loop {
            if self.api() && !self.batch() && !self.signal(BnetSignal::YesNo) {
                return None;
            }
            let answer = self.get_cmd(prompt)?;
            if answer.is_empty() || answer.len() > 3 {
                continue;
            }
            if let Some(yes) = is_yesno(&answer) {
                return Some(yes);
            }
            self.warning_msg("Invalid response. You must answer yes or no.\n");
        }
    }

    /// C++ GetConfirmation(): `yes` argument, else ask (never in API or
    /// batch mode); without console `fallback`.
    pub fn get_confirmation(&mut self, ast: &CommandAst, prompt: &str, fallback: bool) -> bool {
        if find_arg(ast, "yes").is_some() {
            return true;
        }
        if !self.has_console() {
            return fallback;
        }
        if self.api() || self.batch() {
            return false;
        }
        self.get_yesno(prompt).unwrap_or(false)
    }

    /// C++ StartPrompt().
    pub fn start_prompt(&mut self, header: &str) {
        self.prompts = vec![header.to_string()];
    }

    /// C++ AddPrompt(): items stay unique.
    pub fn add_prompt(&mut self, item: &str) {
        if !self.prompts.iter().skip(1).any(|p| p == item) {
            self.prompts.push(item.to_string());
        }
    }

    /// C++ DoPrompt(): one item is selected automatically, batch mode lists
    /// the choices and fails, otherwise the list is shown and a number asked
    /// for until valid. Returns the 0-based index and the item.
    pub fn do_prompt(&mut self, automsg: &str, msg: &str) -> Option<(usize, String)> {
        let prompts = std::mem::take(&mut self.prompts);
        let count = prompts.len().saturating_sub(1);
        if count == 1 {
            if !self.api() {
                self.send_msg(format!("Automatically selected {automsg}: {}\n", prompts[1]));
            }
            return Some((0, prompts[1].clone()));
        }

        if self.batch() {
            self.send_msg(prompts.first().cloned().unwrap_or_default());
            self.send_msg(format_prompts(&prompts, 80, 20));
            self.send_msg(format!(
                "Your request has multiple choices for \"{automsg}\". Selection is not possible in batch mode.\n"));
            return None;
        }

        let api = self.api();
        if api && !self.signal(BnetSignal::StartSelect) {
            return None;
        }
        self.send_msg(prompts.first().cloned().unwrap_or_default());
        if api {
            for item in &prompts[1..] {
                self.send_msg(item.clone());
            }
            if !self.signal(BnetSignal::EndSelect) {
                return None;
            }
        } else {
            self.send_msg(format_prompts(&prompts, 80, 20));
        }

        if count == 0 {
            self.error_msg(format!("Selection list for \"{automsg}\" is empty!\n"));
            return None;
        }
        loop {
            if api && !self.signal(BnetSignal::SelectInput) {
                return None;
            }
            let Some(item) = self.get_pint(&format!("{msg} (1-{count}): ")) else {
                self.info_msg("Selection aborted, nothing done.\n");
                return None;
            };
            let item = item as usize;
            if item < 1 || item > count {
                self.warning_msg(format!("Please enter a number between 1 and {count}\n"));
                continue;
            }
            return Some((item - 1, prompts[item].clone()));
        }
    }

    /// C++ DoKeywordPrompt().
    pub fn do_keyword_prompt(&mut self, msg: &str, list: &[&str]) -> Option<usize> {
        self.start_prompt("You have the following choices:\n");
        for item in list {
            self.add_prompt(item);
        }
        self.do_prompt("", msg).map(|(index, _)| index)
    }

    /// C++ ConfirmRetention(): show the period, ask yes/mod/no; `mod` asks
    /// for a new period. Returns the confirmed period or None.
    pub fn confirm_retention(&mut self, ast: &CommandAst, mut period: u64, what: &str) -> Option<u64> {
        let yes = find_arg(ast, "yes").is_some();
        loop {
            self.info_msg(format!("The current {what} retention period is: {}\n", edit_utime(period)));
            if yes {
                return Some(period);
            }
            let answer = self.get_cmd("Continue? (yes/mod/no): ")?;
            if answer.eq_ignore_ascii_case("mod") {
                let answer = self.get_cmd("Enter new retention period: ")?;
                match crate::commands::show::parse_duration(&answer) {
                    Some(value) => period = value,
                    None => self.error_msg("Invalid period.\n"),
                }
                continue;
            }
            match is_yesno(&answer) {
                Some(true) => return Some(period),
                Some(false) => return None,
                None => {}
            }
        }
    }

    /// Result of the command. After a dialog the rest goes to the console
    /// directly (C++ order); otherwise text and errors are returned.
    pub fn finish(mut self, success: bool) -> CommandResult {
        if self.interacted && !self.batch() {
            self.flush();
            return CommandResult { success, ..Default::default() };
        }
        let mut result = CommandResult { success, ..Default::default() };
        result.errors_first = matches!(self.segments.first(), Some(Segment::Error(_)));
        for segment in self.segments {
            match segment {
                Segment::Error(t) => result.error_message.push_str(&t),
                Segment::Text(t) | Segment::Warning(t) | Segment::Info(t) => result.text_output.push_str(&t),
            }
        }
        result
    }
}

/// C++ IsYesno(): case-insensitive yes/no.
pub fn is_yesno(answer: &str) -> Option<bool> {
    if answer.eq_ignore_ascii_case("yes") {
        Some(true)
    } else if answer.eq_ignore_ascii_case("no") {
        Some(false)
    } else {
        None
    }
}

/// C++ FindArg(): index of the first argument with that keyword.
pub fn find_arg(ast: &CommandAst, keyword: &str) -> Option<usize> {
    ast.arguments.iter().position(|a| a.key.eq_ignore_ascii_case(keyword))
}

/// C++ FindArgWithValue(): value of the first argument with that keyword;
/// None when it is missing or has no value.
pub fn find_arg_with_value<'a>(ast: &'a CommandAst, keyword: &str) -> Option<&'a str> {
    ast.arguments.iter().find(|a| a.key.eq_ignore_ascii_case(keyword))?.value.as_deref()
}

/// C++ Is_a_number() followed by strtod().
fn parse_number(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let mut i = 0;
    if matches!(bytes.first(), Some(b'-' | b'+')) {
        i += 1;
    }
    let start = i;
    while bytes.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    if i == start {
        return None;
    }
    if bytes.get(i) == Some(&b'.') {
        i += 1;
        while bytes.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
    }
    if matches!(bytes.get(i), Some(b'e' | b'E')) {
        let digit_at = |k: usize| bytes.get(k).is_some_and(u8::is_ascii_digit);
        if digit_at(i + 1) || (matches!(bytes.get(i + 1), Some(b'-' | b'+')) && digit_at(i + 2)) {
            i += 2;
            while digit_at(i) {
                i += 1;
            }
        }
    }
    if i != bytes.len() {
        return None;
    }
    text.parse::<f64>().ok()
}

/// C++ edit_utime(): "1 year 2 months 3 days 4 hours 5 mins 6 secs"; without
/// seconds the last unit keeps its trailing blank ("1 hour ").
pub fn edit_utime(mut val: u64) -> String {
    const UNITS: [(u64, &str); 5] = [
        (60 * 60 * 24 * 365, "year"),
        (60 * 60 * 24 * 30, "month"),
        (60 * 60 * 24, "day"),
        (60 * 60, "hour"),
        (60, "min"),
    ];
    let mut out = String::new();
    for (size, name) in UNITS {
        let times = val / size;
        if times > 0 {
            val -= times * size;
            out.push_str(&format!("{times} {name}{} ", if times > 1 { "s" } else { "" }));
        }
    }
    if val == 0 && out.is_empty() {
        out.push_str("0 secs");
    } else if val != 0 {
        out.push_str(&format!("{val} sec{}", if val > 1 { "s" } else { "" }));
    }
    out
}

/// C++ FormatPrompts(): numbered items, in columns when there are more than
/// `min_lines_threshold` entries (counting the header) and two columns fit.
pub fn format_prompts(prompts: &[String], window_width: usize, min_lines_threshold: usize) -> String {
    let num_prompts = prompts.len();
    let items = prompts.get(1..).unwrap_or(&[]);
    let max_prompt_length = items.iter().map(String::len).max().unwrap_or(0).max(1);
    let index_length = num_prompts.to_string().len();
    let max_formatted = max_prompt_length + index_length + 4;
    // snprintf into a buffer of max_formatted bytes keeps max_formatted-1.
    let clip = |mut s: String| {
        s.truncate(max_formatted - 1);
        s
    };
    let mut output = String::new();
    if num_prompts > min_lines_threshold && window_width > max_formatted * 2 {
        let per_line = (window_width / (max_formatted - 1)).max(1);
        let lines = num_prompts.div_ceil(per_line);
        let cells: Vec<String> = items.iter().enumerate()
            .map(|(i, p)| clip(format!("{:>w$}: {:<m$} ", i + 1, p, w = index_length, m = max_prompt_length)))
            .collect();
        for line in 0..lines {
            let mut index = line;
            while index < cells.len() {
                output.push_str(&cells[index]);
                index += lines;
            }
            output.push('\n');
        }
    } else {
        for (i, p) in items.iter().enumerate() {
            output.push_str(&clip(format!("{:>w$}: {}\n", i + 1, p, w = index_length)));
        }
    }
    output
}

/// Names of a resource type in chain order (copied out of the config lock
/// so that ACL checks can take it again).
fn resource_names(ctx: &ExecContext, f: impl FnOnce(&crate::director_config::DirectorConfig) -> Vec<String>) -> Vec<String> {
    ctx.config.as_ref()
        .and_then(|config| config.read().ok().map(|c| f(&c)))
        .unwrap_or_default()
}

/// Common body of the select_*_resource functions: ACL filter,
/// SortCaseInsensitive, DoPrompt.
fn select_from(ua: &mut Ua, names: Vec<String>, kind: Kind, header: &str, automsg: &str, msg: &str) -> Option<String> {
    ua.start_prompt(header);
    let permitted: Vec<&String> = names.iter().filter(|n| acl::access(ua.ctx, kind, n)).collect();
    for name in sort_case_insensitive(permitted.iter().map(|n| n.as_str())) {
        ua.add_prompt(name);
    }
    ua.do_prompt(automsg, msg).map(|(_, name)| name)
}

/// C++ select_job_resource().
pub fn select_job_resource(ua: &mut Ua) -> Option<String> {
    let names = resource_names(ua.ctx, |c| c.jobs.iter().map(|j| j.name.clone()).collect());
    select_from(ua, names, Kind::Job, "The defined Job resources are:\n", "Job", "Select Job resource")
}

/// C++ select_restore_job_resource().
pub fn select_restore_job_resource(ua: &mut Ua) -> Option<String> {
    use bareos_config::resources::JobType;
    let names = resource_names(ua.ctx, |c| c.jobs.iter()
        .filter(|j| j.job_type == JobType::Restore).map(|j| j.name.clone()).collect());
    select_from(ua, names, Kind::Job, "The defined Restore Job resources are:\n", "Job", "Select Restore Job")
}

/// C++ select_client_resource().
pub fn select_client_resource(ua: &mut Ua) -> Option<String> {
    let names = resource_names(ua.ctx, |c| c.clients.iter().map(|x| x.name.clone()).collect());
    select_from(ua, names, Kind::Client, "The defined Client resources are:\n", "Client",
        "Select Client (File daemon) resource")
}

/// C++ select_fileset_resource().
pub fn select_fileset_resource(ua: &mut Ua) -> Option<String> {
    let names = resource_names(ua.ctx, |c| c.filesets.iter().map(|x| x.name.clone()).collect());
    select_from(ua, names, Kind::FileSet, "The defined FileSet resources are:\n", "FileSet", "Select FileSet resource")
}

/// C++ select_pool_resource().
pub fn select_pool_resource(ua: &mut Ua) -> Option<String> {
    let names = resource_names(ua.ctx, |c| c.pools.iter().map(|x| x.name.clone()).collect());
    select_from(ua, names, Kind::Pool, "The defined Pool resources are:\n", "Pool", "Select Pool resource")
}

/// C++ select_storage_resource().
pub fn select_storage_resource(ua: &mut Ua, autochanger_only: bool) -> Option<String> {
    let names = resource_names(ua.ctx, |c| c.storages.iter()
        .filter(|s| !autochanger_only || is_autochanger(c, &s.name))
        .map(|x| x.name.clone()).collect());
    let header = if autochanger_only {
        "The defined Autochanger Storage resources are:\n"
    } else {
        "The defined Storage resources are:\n"
    };
    select_from(ua, names, Kind::Storage, header, "Storage", "Select Storage resource")
}

/// `Auto Changer = yes` of a Storage resource (raw configuration view).
fn is_autochanger(config: &crate::director_config::DirectorConfig, storage: &str) -> bool {
    config.raw.iter()
        .find(|r| r.norm_type() == "storage" && r.name() == Some(storage))
        .and_then(|r| r.directives.iter().find(|d| d.norm_key() == "autochanger"))
        .and_then(|d| d.values().first().map(|v| matches!(v.to_ascii_lowercase().as_str(), "yes" | "true")))
        .unwrap_or(false)
}

/// C++ get_catalog_resource()'s prompt part (no `catalog=`, not gui).
pub fn select_catalog_resource(ua: &mut Ua) -> Option<String> {
    let names = resource_names(ua.ctx, |c| c.catalogs.iter().map(|x| x.name.clone()).collect());
    // Catalogs are listed in chain order (no SortCaseInsensitive).
    ua.start_prompt("The defined Catalog resources are:\n");
    for name in names.iter().filter(|n| acl::access(ua.ctx, Kind::Catalog, n)) {
        ua.add_prompt(name);
    }
    ua.do_prompt("Catalog", "Select Catalog resource").map(|(_, name)| name)
}

/// C++ get_client_resource(): `client=`/`fd=` or the selection list.
pub fn get_client_resource(ua: &mut Ua, ast: &CommandAst) -> Option<String> {
    if let Some(arg) = ast.arguments.iter().find(|a| {
        (a.key.eq_ignore_ascii_case("client") || a.key.eq_ignore_ascii_case("fd")) && a.value.is_some()
    }) {
        let name = arg.value.as_deref().unwrap_or_default();
        let exists = resource_names(ua.ctx, |c| c.clients.iter().map(|x| x.name.clone()).collect())
            .iter().any(|n| n == name);
        if exists {
            return Some(name.to_string());
        }
        ua.error_msg(format!("Error: Client resource {name} does not exist.\n"));
    }
    select_client_resource(ua)
}

/// C++ get_pool_resource(): `pool=` or the selection list.
pub fn get_pool_resource(ua: &mut Ua, ast: &CommandAst) -> Option<String> {
    if let Some(name) = find_arg_with_value(ast, "pool") {
        if acl::access(ua.ctx, Kind::Pool, name) {
            let exists = resource_names(ua.ctx, |c| c.pools.iter().map(|x| x.name.clone()).collect())
                .iter().any(|n| n == name);
            if exists {
                return Some(name.to_string());
            }
            ua.error_msg(format!("Error: Pool resource \"{name}\" does not exist.\n"));
        }
    }
    select_pool_resource(ua)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, RwLock};

    use crate::director_config::DirectorConfig;

    /// What a scripted console saw, in order.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Event {
        Text(String),
        Signal(BnetSignal),
        Answer(String),
    }

    /// Console with scripted answers; records everything sent to it.
    #[derive(Default)]
    pub struct Scripted {
        pub answers: VecDeque<String>,
        pub events: Vec<Event>,
    }

    impl Scripted {
        pub fn new(answers: &[&str]) -> Self {
            Self { answers: answers.iter().map(|a| a.to_string()).collect(), events: Vec::new() }
        }

        /// All text sent, concatenated.
        pub fn text(&self) -> String {
            self.events.iter().filter_map(|e| match e {
                Event::Text(t) => Some(t.as_str()),
                _ => None,
            }).collect()
        }

        pub fn signals(&self) -> Vec<BnetSignal> {
            self.events.iter().filter_map(|e| match e {
                Event::Signal(s) => Some(*s),
                _ => None,
            }).collect()
        }

        /// Transcript like a terminal: text, answers after prompts.
        pub fn transcript(&self) -> String {
            self.events.iter().map(|e| match e {
                Event::Text(t) => t.clone(),
                Event::Signal(_) => String::new(),
                Event::Answer(a) => format!("{a}\n"),
            }).collect()
        }
    }

    impl Console for Scripted {
        fn send(&mut self, text: &str) -> bool {
            self.events.push(Event::Text(text.to_string()));
            true
        }
        fn signal(&mut self, signal: BnetSignal) -> bool {
            self.events.push(Event::Signal(signal));
            true
        }
        fn recv(&mut self) -> Option<String> {
            let answer = self.answers.pop_front()?;
            self.events.push(Event::Answer(answer.clone()));
            Some(answer)
        }
    }

    pub fn context(config: &str, api_mode: ApiMode) -> ExecContext {
        ExecContext {
            session_id: 0,
            config: Some(Arc::new(RwLock::new(DirectorConfig::from_str(config).unwrap()))),
            session_mgr: None,
            catalog: None,
            api_mode,
            bus_rx: None,
            job_manager: None,
        }
    }

    /// Run `f` with a scripted console; returns its result and the console.
    pub fn scripted<R>(answers: &[&str], f: impl FnOnce() -> R) -> (R, Scripted) {
        let mut console = Scripted::new(answers);
        let result = with_console(&mut console, f);
        (result, console)
    }

    const CONFIG: &str = r#"
Director { Name=dir; Password=secret }
Job { Name=backup-b; Client=fd1 }
Job { Name=Backup-a; Client=fd1 }
Job { Name=restore; Type=Restore; Client=fd1 }
Client { Name=fd1; Password=x }
Pool { Name=Full }
"#;

    fn prompt_list(ua: &mut Ua, items: &[&str]) {
        ua.start_prompt("Header:\n");
        for item in items {
            ua.add_prompt(item);
        }
    }

    #[test]
    fn get_cmd_prompts_and_cancels() {
        let ctx = context(CONFIG, ApiMode::Human);
        let ((a, b), console) = scripted(&["mod  \r\n", "."], || {
            let mut ua = Ua::new(&ctx);
            ua.send_msg("before\n");
            (ua.get_cmd("Q? "), ua.get_cmd("R? "))
        });
        assert_eq!(a.as_deref(), Some("mod"));
        assert_eq!(b, None, "lone dot cancels");
        assert_eq!(console.events, vec![
            Event::Text("before\n".into()),
            Event::Text("Q? ".into()),
            Event::Signal(BnetSignal::SubPrompt),
            Event::Answer("mod  \r\n".into()),
            Event::Text("R? ".into()),
            Event::Signal(BnetSignal::SubPrompt),
            Event::Answer(".".into()),
        ]);
    }

    #[test]
    fn get_cmd_api_and_batch() {
        let ctx = context(CONFIG, ApiMode::Api);
        let (answer, console) = scripted(&["x"], || Ua::new(&ctx).get_cmd("Q? "));
        assert_eq!(answer.as_deref(), Some("x"));
        assert_eq!(console.events[..2], [Event::Signal(BnetSignal::TextInput), Event::Text("Q? ".into())]);
        let (answer, console) = scripted(&["x"], || Ua::new(&ctx).get_cmd_ext("$ ", true));
        assert_eq!(answer.as_deref(), Some("x"));
        assert_eq!(console.signals(), vec![BnetSignal::SubPrompt]);

        let json = context(CONFIG, ApiMode::Json);
        let (answer, console) = scripted(&["x"], || Ua::new(&json).get_cmd("Q? "));
        assert_eq!(answer, None);
        assert!(console.events.is_empty(), "JSON mode is batch mode");
        let human = context(CONFIG, ApiMode::Human);
        assert_eq!(Ua::new(&human).get_cmd("Q? "), None, "no console");
    }

    #[test]
    fn get_pint_rejects_until_number() {
        let ctx = context(CONFIG, ApiMode::Human);
        let (value, console) = scripted(&["abc", "-1", "", "2.7"], || Ua::new(&ctx).get_pint("N: "));
        assert_eq!(value, Some(2));
        assert_eq!(console.transcript(),
            "N: abc\nExpected a positive integer, got: abc\nN: -1\nExpected a positive integer, got: -1\n\
             N: \nExpected a positive integer, got: \nN: 2.7\n");
        let (value, _) = scripted(&[""], || Ua::new(&ctx).get_pint("Enter slot (0 for none): "));
        assert_eq!(value, Some(0));
        assert_eq!(parse_number("1e3"), Some(1000.0));
        assert_eq!(parse_number("+4"), Some(4.0));
        assert_eq!(parse_number("4."), Some(4.0));
        assert_eq!(parse_number(".5"), None);
        assert_eq!(parse_number("1e"), None);
    }

    #[test]
    fn get_yesno_rules() {
        let ctx = context(CONFIG, ApiMode::Human);
        let (value, console) = scripted(&["", "maybe", "y", "NO"], || Ua::new(&ctx).get_yesno("Ok? "));
        assert_eq!(value, Some(false));
        assert_eq!(console.transcript(),
            "Ok? \nOk? maybe\nOk? y\nInvalid response. You must answer yes or no.\nOk? NO\n");
        let api = context(CONFIG, ApiMode::Api);
        let (value, console) = scripted(&["yes"], || Ua::new(&api).get_yesno("Ok? "));
        assert_eq!(value, Some(true));
        assert_eq!(console.signals(), vec![BnetSignal::YesNo, BnetSignal::TextInput]);
    }

    #[test]
    fn get_confirmation_modes() {
        let ast = crate::session::parse_cli_command("purge yes");
        let ctx = context(CONFIG, ApiMode::Human);
        assert!(Ua::new(&ctx).get_confirmation(&ast, "Sure? ", false));
        let plain = crate::session::parse_cli_command("purge");
        assert!(Ua::new(&ctx).get_confirmation(&plain, "Sure? ", true), "no console: fallback");
        let (value, _) = scripted(&["yes"], || Ua::new(&ctx).get_confirmation(&plain, "Sure? ", false));
        assert!(value);
        let api = context(CONFIG, ApiMode::Api);
        let (value, console) = scripted(&["yes"], || Ua::new(&api).get_confirmation(&plain, "Sure? ", true));
        assert!(!value);
        assert!(console.events.is_empty());
    }

    #[test]
    fn do_prompt_single_item_is_automatic() {
        let ctx = context(CONFIG, ApiMode::Human);
        let mut ua = Ua::new(&ctx);
        prompt_list(&mut ua, &["only", "only"]);
        assert_eq!(ua.do_prompt("Job", "Select Job"), Some((0, "only".into())));
        assert_eq!(ua.finish(true).text_output, "Automatically selected Job: only\n");
        let api = context(CONFIG, ApiMode::Api);
        let mut ua = Ua::new(&api);
        prompt_list(&mut ua, &["only"]);
        assert!(ua.do_prompt("Job", "Select Job").is_some());
        assert_eq!(ua.finish(true).text_output, "", "no message in API mode");
    }

    #[test]
    fn do_prompt_batch_lists_choices() {
        let ctx = context(CONFIG, ApiMode::Human);
        let mut ua = Ua::new(&ctx);
        prompt_list(&mut ua, &["a", "b"]);
        assert_eq!(ua.do_prompt("Job", "Select Job"), None);
        assert_eq!(ua.finish(false).text_output,
            "Header:\n1: a\n2: b\nYour request has multiple choices for \"Job\". \
             Selection is not possible in batch mode.\n");
    }

    #[test]
    fn do_prompt_selects_with_retry_and_cancel() {
        let ctx = context(CONFIG, ApiMode::Human);
        let (choice, console) = scripted(&["0", "x", "3", "2"], || {
            let mut ua = Ua::new(&ctx);
            prompt_list(&mut ua, &["a", "b", "c"]);
            let choice = ua.do_prompt("Job", "Select Job resource");
            assert!(ua.finish(true).text_output.is_empty());
            choice
        });
        assert_eq!(choice, Some((2, "c".into())));
        assert_eq!(console.transcript(),
            "Header:\n1: a\n2: b\n3: c\nSelect Job resource (1-3): 0\nPlease enter a number between 1 and 3\n\
             Select Job resource (1-3): x\nExpected a positive integer, got: x\nSelect Job resource (1-3): 3\n");
        let (choice, console) = scripted(&["."], || {
            let mut ua = Ua::new(&ctx);
            prompt_list(&mut ua, &["a", "b"]);
            let choice = ua.do_prompt("Job", "Select Job resource");
            ua.finish(false);
            choice
        });
        assert_eq!(choice, None);
        assert_eq!(console.transcript(),
            "Header:\n1: a\n2: b\nSelect Job resource (1-2): .\nSelection aborted, nothing done.\n");
    }

    #[test]
    fn do_prompt_empty_list() {
        let ctx = context(CONFIG, ApiMode::Human);
        let (choice, console) = scripted(&[], || {
            let mut ua = Ua::new(&ctx);
            prompt_list(&mut ua, &[]);
            ua.send_msg("x\n");
            let choice = ua.do_prompt("Pool", "Select Pool");
            (choice, ua.finish(false))
        });
        assert_eq!(choice.0, None);
        // Nothing was asked: the result carries the text and the error.
        assert_eq!(choice.1.text_output, "x\nHeader:\n");
        assert_eq!(choice.1.error_message, "Selection list for \"Pool\" is empty!\n");
        assert!(console.events.is_empty());
    }

    #[test]
    fn do_prompt_api_signals() {
        let ctx = context(CONFIG, ApiMode::Api);
        let (_, console) = scripted(&["5", "1"], || {
            let mut ua = Ua::new(&ctx);
            prompt_list(&mut ua, &["a", "b"]);
            ua.do_prompt("Job", "Select Job resource")
        });
        use BnetSignal::*;
        assert_eq!(console.events, vec![
            Event::Signal(StartSelect),
            Event::Text("Header:\n".into()),
            Event::Text("a".into()),
            Event::Text("b".into()),
            Event::Signal(EndSelect),
            Event::Signal(SelectInput),
            Event::Signal(TextInput),
            Event::Text("Select Job resource (1-2): ".into()),
            Event::Answer("5".into()),
            Event::Signal(WarningMsg),
            Event::Text("Please enter a number between 1 and 2\n".into()),
            Event::Signal(SelectInput),
            Event::Signal(TextInput),
            Event::Text("Select Job resource (1-2): ".into()),
            Event::Answer("1".into()),
        ]);
    }

    #[test]
    fn format_prompts_index_width_and_columns() {
        let mut prompts = vec!["H\n".to_string()];
        prompts.extend((1..=9).map(|i| format!("p{i}")));
        // Index width counts the header: 10 entries -> two digits.
        assert!(format_prompts(&prompts, 80, 20).starts_with(" 1: p1\n"));
        let mut prompts = vec!["H\n".to_string()];
        prompts.extend((1..=25).map(|i| format!("item{i}")));
        let text = format_prompts(&prompts, 80, 20);
        // max len 6, index 2 -> cell "%2d: %-6s " = 11 chars, 7 per line, 4 lines
        let first = text.lines().next().unwrap();
        assert_eq!(first, " 1: item1   5: item5   9: item9  13: item13 17: item17 21: item21 25: item25 ");
        assert_eq!(text.lines().count(), 4);
    }

    #[test]
    fn select_job_resource_sorts_case_insensitive() {
        let ctx = context(CONFIG, ApiMode::Human);
        let (job, console) = scripted(&["2"], || select_job_resource(&mut Ua::new(&ctx)));
        assert_eq!(job.as_deref(), Some("backup-b"));
        assert_eq!(console.transcript(),
            "The defined Job resources are:\n1: Backup-a\n2: backup-b\n3: restore\nSelect Job resource (1-3): 2\n");
        let mut ua = Ua::new(&ctx);
        assert_eq!(select_restore_job_resource(&mut ua).as_deref(), Some("restore"));
        assert_eq!(ua.finish(true).text_output, "Automatically selected Job: restore\n");
    }

    #[test]
    fn get_pool_resource_reports_unknown_pool() {
        let ctx = context(CONFIG, ApiMode::Human);
        let ast = crate::session::parse_cli_command("x pool=nosuch");
        let mut ua = Ua::new(&ctx);
        assert_eq!(get_pool_resource(&mut ua, &ast).as_deref(), Some("Full"));
        let result = ua.finish(true);
        assert_eq!(result.error_message, "Error: Pool resource \"nosuch\" does not exist.\n");
        assert_eq!(result.text_output, "Automatically selected Pool: Full\n");
    }

    #[test]
    fn confirm_retention_mod_then_no() {
        let ctx = context(CONFIG, ApiMode::Human);
        let ast = crate::session::parse_cli_command("prune");
        let (period, console) = scripted(&["mod", "bogus", "mod", "1 day", "no"], || {
            Ua::new(&ctx).confirm_retention(&ast, 3600, "Job")
        });
        assert_eq!(period, None);
        assert_eq!(console.transcript(),
            "The current Job retention period is: 1 hour \nContinue? (yes/mod/no): mod\n\
             Enter new retention period: bogus\nInvalid period.\n\
             The current Job retention period is: 1 hour \nContinue? (yes/mod/no): mod\n\
             Enter new retention period: 1 day\n\
             The current Job retention period is: 1 day \nContinue? (yes/mod/no): no\n");
        assert_eq!(edit_utime(0), "0 secs");
        assert_eq!(edit_utime(60 * 60 * 24 * 400 + 61), "1 year 1 month 5 days 1 min 1 sec");
    }
}
