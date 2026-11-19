//! Restore — FD side: filed/restore.cc DoRestore().
//!
//! The Storage daemon sends, per record, a header message
//! `rechdr <VolSessionId> <VolSessionTime> <FileIndex> <Stream> <size>` and
//! the record data. For every file: the attributes (STREAM_UNIX_ATTRIBUTES),
//! then data (plain, sparse, gzip/compressed), then ACL/xattr streams and
//! digests. The stream ends with BNET_EOD.
//!
//! Not supported by this File Daemon (reported like the original does when
//! the keys or the platform support are missing): encrypted data (no PKI
//! keys can be configured), MacOS resource forks/Finder info, plugins.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use bareos_bnet::{BnetMessage, BnetSignal, BnetStream};

use crate::acl_xattr::{self, AclXattrState, Exit, REPORT_ERR_MAX_PER_JOB};
use crate::attribs::*;
use crate::compression;
use crate::create_file::{self, Cf, CreateState};
use crate::jmsg::*;
use crate::streams::*;
use crate::plugin::{self, PluginError, PluginLine, Session};
use onesimus_proto_rs::onesimus::filed::plugin::v1 as pb;

/// Largest block an online-restore plugin receives per WriteFile.
const ONLINE_CHUNK: u32 = 256 * 1024;

/// What the `restore` command asked for.
pub struct RestoreParams {
    pub where_: Where,
    pub replace: u8,
    pub prefix_links: bool,
}

/// Job counters the restore updates (jcr->JobFiles, ReadBytes, JobBytes).
#[derive(Default, Debug, Clone, Copy)]
pub struct Counters {
    pub job_files: u32,
    pub read_bytes: u64,
    pub job_bytes: u64,
    pub num_files_examined: u32,
}

/// processWin32BackupAPIBlock(): the file data of a BackupRead stream
/// (WIN32_STREAM_ID headers of 20 bytes, the stream name, the content; only
/// BACKUP_DATA content is written).
#[derive(Default)]
struct Win32Decomp {
    header: Vec<u8>,
    skip_name: u64,
    data_left: u64,
    in_data: bool,
}

impl Win32Decomp {
    fn process(&mut self, mut buf: &[u8], out: &mut File) -> std::io::Result<()> {
        const WIN32_BACKUP_DATA: u32 = 1;
        while !buf.is_empty() {
            if self.skip_name > 0 {
                let n = (self.skip_name as usize).min(buf.len());
                self.skip_name -= n as u64;
                buf = &buf[n..];
            } else if self.data_left > 0 {
                let n = (self.data_left.min(buf.len() as u64)) as usize;
                if self.in_data {
                    out.write_all(&buf[..n])?;
                }
                self.data_left -= n as u64;
                buf = &buf[n..];
            } else {
                let n = (20 - self.header.len()).min(buf.len());
                self.header.extend_from_slice(&buf[..n]);
                buf = &buf[n..];
                if self.header.len() == 20 {
                    let h = std::mem::take(&mut self.header);
                    let id = u32::from_le_bytes(h[0..4].try_into().unwrap());
                    self.data_left = u64::from_le_bytes(h[8..16].try_into().unwrap());
                    self.skip_name = u64::from(u32::from_le_bytes(h[16..20].try_into().unwrap()));
                    self.in_data = id == WIN32_BACKUP_DATA;
                }
            }
        }
        Ok(())
    }
}

/// r_ctx
#[derive(Default)]
struct Rctx {
    /// The (masked) plugin line of the files between two plugin-name records.
    plugin: Option<String>,
    /// The plugin name when the current plugin files restore online.
    online: Option<String>,
    /// Open online-restore sessions by plugin name, finished at the end.
    sessions: std::collections::BTreeMap<String, Session>,
    /// The file being written through its plugin.
    online_file: Option<OnlineFile>,
    /// Directories of plugin files: their attributes are set at the end, so
    /// files created in them later do not change their times again.
    deferred_dirs: Vec<Attributes>,
    stream: i32,
    prev_stream: i32,
    size: u32,
    extract: bool,
    bfd: Option<File>,
    file_addr: u64,
    sparse: bool,
    compress: bool,
    win32: Option<Win32Decomp>,
    /// rctx.type (set from the attributes when a file is closed)
    type_: i32,
    attr: Attributes,
    /// jcr->fd_impl->last_fname / last_type
    last_fname: Vec<u8>,
    last_type: i32,
    delayed: Vec<(i32, Vec<u8>)>,
}

/// The "known unknowns" of DoRestore().
#[derive(Default)]
struct NonSupport {
    data: u32,
    attr: u32,
    rsrc: u32,
    finfo: u32,
    acl: u32,
    progname: u32,
    crypto: u32,
    xattr: u32,
}

/// IsRestoreStreamSupported() (findlib/bfile.cc, non-Windows).
pub fn is_restore_stream_supported(stream: i32) -> bool {
    matches!(stream,
        STREAM_GZIP_DATA | STREAM_SPARSE_GZIP_DATA | STREAM_WIN32_GZIP_DATA | STREAM_COMPRESSED_DATA
        | STREAM_SPARSE_COMPRESSED_DATA | STREAM_WIN32_COMPRESSED_DATA | STREAM_ENCRYPTED_FILE_COMPRESSED_DATA
        | STREAM_ENCRYPTED_WIN32_COMPRESSED_DATA | STREAM_WIN32_DATA | STREAM_UNIX_ATTRIBUTES | STREAM_FILE_DATA
        | STREAM_MD5_DIGEST | STREAM_UNIX_ATTRIBUTES_EX | STREAM_SPARSE_DATA | STREAM_PROGRAM_NAMES
        | STREAM_PROGRAM_DATA | STREAM_SHA1_DIGEST | STREAM_SHA256_DIGEST | STREAM_SHA512_DIGEST
        | STREAM_XXH128_DIGEST | STREAM_SIGNED_DIGEST | STREAM_ENCRYPTED_FILE_DATA
        | STREAM_ENCRYPTED_FILE_GZIP_DATA | STREAM_ENCRYPTED_WIN32_DATA | STREAM_ENCRYPTED_WIN32_GZIP_DATA | 0)
}

/// BgetMsg(): the next data message; None at a signal (EOD, terminate)
/// or a network error. Heartbeats are skipped.
fn bget_msg<S: Read + Write>(sd: &mut BnetStream<S>) -> Result<Vec<u8>, String> {
    loop {
        match sd.recv() {
            Ok(BnetMessage::Data(d)) => return Ok(d),
            Ok(BnetMessage::Signal(BnetSignal::Heartbeat | BnetSignal::HeartbeatResponse)) => continue,
            Ok(BnetMessage::Signal(BnetSignal::Poll)) => {
                let _ = sd.send_signal(BnetSignal::Eod);
                continue;
            }
            Ok(BnetMessage::Signal(s)) => return Err(format!("{s:?}")),
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn parse_rechdr(msg: &[u8]) -> Option<(u32, u32, i32, i32, u32)> {
    let v = crate::scan::scan(msg, "rechdr %u %u %d %d %u", 5)?;
    Some((v[0].int() as u32, v[1].int() as u32, v[2].int() as i32, v[3].int() as i32, v[4].int() as u32))
}

/// The restore of one SD read session.
pub struct Restore<'a> {
    pub params: &'a RestoreParams,
    pub msgs: &'a mut JobMsgs,
    pub counters: &'a mut Counters,
    pub create: &'a mut CreateState,
    pub canceled: &'a AtomicBool,
    /// Plugin directives; None: plugin files restore only to files.
    pub config: Option<&'a crate::config::FdConfig>,
    /// "pluginoptions" lines of the job (restore_mode=online and its options).
    pub plugin_options: &'a [String],
}

/// The file being restored through a plugin and the next write offset.
struct OnlineFile {
    plugin: String,
    entry: pb::FileEntry,
    offset: u64,
}

/// The plugin line of a "pluginoptions" entry that asks for an online
/// restore of plugin `stored.name`, merged over the masked line from the
/// backup: masked values are dropped, the restore options win.
fn online_line(stored: &PluginLine, plugin_options: &[String]) -> Option<PluginLine> {
    let wanted = plugin_options.iter().filter_map(|o| plugin::parse_line(o).ok()).find(|o| {
        o.name == stored.name && o.options.get("restore_mode").map(String::as_str) == Some("online")
    })?;
    let mut merged = stored.clone();
    merged.options.retain(|_, v| v != crate::secrets::MASK);
    merged.options.extend(wanted.options);
    Some(merged)
}

/// The FileEntry a plugin receives for a restored file.
fn plugin_entry(attr: &Attributes) -> pb::FileEntry {
    let kind = match attr.type_ {
        FT_DIREND | FT_DIRBEGIN => pb::FileKind::Directory,
        FT_LNK => pb::FileKind::Symlink,
        _ => pb::FileKind::Virtual,
    };
    let mut path = attr.fname.clone();
    if kind == pb::FileKind::Directory && path.len() > 1 && path.last() == Some(&b'/') {
        path.pop();
    }
    pb::FileEntry {
        path,
        kind: kind as i32,
        size: u64::try_from(attr.statp.size).unwrap_or(0),
        modified_unix_nanos: attr.statp.mtime.saturating_mul(1_000_000_000),
        mode: attr.statp.mode & 0o7777,
        symlink_target: if kind == pb::FileKind::Symlink { attr.lname.clone() } else { Vec::new() },
        attributes: Default::default(),
        source_ref: Vec::new(),
    }
}

impl Restore<'_> {
    fn jmsg(&mut self, t: i32, text: impl AsRef<str>) {
        self.msgs.jmsg(t, text.as_ref());
    }

    fn last_fname(r: &Rctx) -> String {
        String::from_utf8_lossy(&r.last_fname).into_owned()
    }

    /// DoRestore(). `flush` sends queued job messages to the Director.
    pub fn run<S: Read + Write>(&mut self, sd: &mut BnetStream<S>, flush: &mut dyn FnMut(&mut JobMsgs)) {
        self.msgs.set_status(JS_RUNNING);
        self.jmsg(M_INFO, format!("Version: {} ({}) {}\n", crate::version::FULL, crate::version::DATE, crate::version::os_info()));
        flush(self.msgs);

        let mut r = Rctx::default();
        let mut ns = NonSupport::default();
        let mut ax = AclXattrState::default();
        let mut prev_file_index = 0i32;
        let ok = 'body: {
            while !self.canceled.load(Ordering::Relaxed) && !self.msgs.is_canceled() {
                flush(self.msgs);
                let Ok(hdr) = bget_msg(sd) else { break };
                r.prev_stream = r.stream;
                let Some((_vsid, _vst, file_index, full_stream, size)) = parse_rechdr(&hdr) else {
                    self.jmsg(M_FATAL, format!("Record header scan error: {}\n", String::from_utf8_lossy(&hdr)));
                    break 'body false;
                };
                r.stream = full_stream & STREAMMASK_TYPE;
                r.size = size;
                let data = match bget_msg(sd) {
                    Ok(d) => d,
                    Err(e) => {
                        self.jmsg(M_FATAL, format!("Data record error. ERR={e}\n"));
                        break 'body false;
                    }
                };
                if size as usize != data.len() {
                    self.jmsg(M_FATAL, format!("Actual data size {} not same as header {size}\n", data.len()));
                    break 'body false;
                }
                if r.stream != STREAM_UNIX_ATTRIBUTES && r.stream != STREAM_UNIX_ATTRIBUTES_EX
                    && r.stream != STREAM_PLUGIN_NAME && prev_file_index != file_index
                {
                    self.jmsg(M_FATAL, format!(
                        "Expected to write data to stream {} of file {file_index}, but this file was never created. Current file is: {prev_file_index}.\n",
                        r.stream));
                    break 'body false;
                }
                prev_file_index = file_index;
                if !self.record(&mut r, &mut ns, &mut ax, &data) {
                    break 'body false;
                }
            }
            self.close_previous_stream(&mut r, &mut ax)
        };
        // Deepest first, so a parent's time is set after its children's.
        let mut dirs = std::mem::take(&mut r.deferred_dirs);
        dirs.sort_by_key(|a| std::cmp::Reverse(a.ofname.len()));
        for attr in dirs {
            for e in set_attributes(&attr, None) {
                self.jmsg(M_ERROR, e);
            }
        }
        // Report the result to every online-restore plugin and stop it.
        let mut ok = ok;
        if r.online_file.is_some() {
            ok &= self.close_online(&mut r, true);
        }
        for (name, session) in std::mem::take(&mut r.sessions) {
            let (finished, notes) = session.finish_with_notes(ok);
            for note in notes {
                self.jmsg(M_INFO, format!("Plugin: \"{name}\": {note}\n"));
            }
            if let Err(e) = finished {
                if ok {
                    self.jmsg(M_FATAL, format!("Plugin: \"{name}\": finish: {e}\n"));
                    ok = false;
                }
            }
        }
        self.msgs.set_status(if ok { JS_TERMINATED } else { JS_ERROR_TERMINATED });

        if ax.acl_errors > 0 {
            self.jmsg(M_WARNING, format!("Encountered {} acl errors while doing restore\n", ax.acl_errors));
        }
        if ax.xattr_errors > 0 {
            self.jmsg(M_WARNING, format!("Encountered {} xattr errors while doing restore\n", ax.xattr_errors));
        }
        if ns.data > 1 || ns.attr > 1 {
            self.jmsg(M_WARNING, format!("{} non-supported data streams and {} non-supported attrib streams ignored.\n", ns.data, ns.attr));
        }
        if ns.rsrc > 0 {
            self.jmsg(M_INFO, format!("{} non-supported resource fork streams ignored.\n", ns.rsrc));
        }
        if ns.finfo > 0 {
            self.jmsg(M_INFO, format!("{} non-supported Finder Info streams ignored.\n", ns.finfo));
        }
        if ns.acl > 0 {
            self.jmsg(M_INFO, format!("{} non-supported acl streams ignored.\n", ns.acl));
        }
        if ns.crypto > 0 {
            self.jmsg(M_INFO, format!("{} non-supported crypto streams ignored.\n", ns.crypto));
        }
        if ns.xattr > 0 {
            self.jmsg(M_INFO, format!("{} non-supported xattr streams ignored.\n", ns.xattr));
        }
        let _ = ns.progname;
        flush(self.msgs);
    }

    /// One record of the switch(stream) of DoRestore(); false: bail out.
    fn record(&mut self, r: &mut Rctx, ns: &mut NonSupport, ax: &mut AclXattrState, data: &[u8]) -> bool {
        match r.stream {
            STREAM_UNIX_ATTRIBUTES | STREAM_UNIX_ATTRIBUTES_EX => {
                if !self.close_previous_stream(r, ax) {
                    return false;
                }
                if r.type_ == FT_DELETED || is_ft_object(r.type_) {
                    return true;
                }
                let mut attr = match unpack_attributes_record(r.stream, data) {
                    Ok(a) => a,
                    Err(m) => {
                        self.jmsg(M_FATAL, m);
                        return false;
                    }
                };
                let (st, link_fi, ds) = decode_stat(&attr.attr);
                attr.statp = st;
                attr.link_fi = link_fi;
                attr.data_stream = ds;
                if !is_restore_stream_supported(attr.data_stream) {
                    if ns.data == 0 {
                        self.jmsg(M_WARNING, format!("{} stream not supported on this Client.\n", stream_to_ascii(attr.data_stream)));
                    }
                    ns.data += 1;
                    return true;
                }
                if let Some(name) = r.online.clone() {
                    return self.open_online_file(r, name, attr);
                }
                if let Some(line) = &r.plugin {
                    // Without Where a plugin file would be written to its
                    // virtual path below /.
                    if matches!(&self.params.where_, Where::Prefix(w) if w.is_empty()) {
                        let name = String::from_utf8_lossy(&attr.fname).into_owned();
                        self.jmsg(M_FATAL, format!(
                            "Plugin: \"{line}\": \"{name}\" can only be restored to files: set Where, \
                             or restore online with Plugin Options \"<plugin>:restore_mode=online\".\n"));
                        return false;
                    }
                }
                build_attr_output_fnames(&mut attr, &self.params.where_, self.params.prefix_links);
                self.counters.num_files_examined += 1;
                r.extract = false;
                let status = {
                    let msgs = &mut *self.msgs;
                    let mut m = |t: i32, s: String| msgs.jmsg(t, &s);
                    create_file::create_file(&mut attr, self.params.replace, self.create, &mut m)
                };
                r.last_fname = attr.ofname.clone();
                r.last_type = attr.type_;
                r.win32 = is_win32_stream(attr.data_stream).then(Win32Decomp::default);
                match status {
                    Cf::Error => {}
                    Cf::Skip => self.counters.job_files += 1,
                    Cf::Extract(f) => {
                        r.extract = true;
                        r.bfd = Some(f);
                        r.file_addr = 0;
                        self.jmsg(M_RESTORED, ls_output(&attr));
                        self.counters.job_files += 1;
                    }
                    Cf::Created => {
                        r.file_addr = 0;
                        self.jmsg(M_RESTORED, ls_output(&attr));
                        self.counters.job_files += 1;
                        // A plugin may list a directory before its content
                        // (the Bareos FD sends it after, as FT_DIREND).
                        if r.plugin.is_some() && attr.type_ == FT_DIREND {
                            r.deferred_dirs.push(attr.clone());
                        } else {
                            for e in set_attributes(&attr, None) {
                                self.jmsg(M_ERROR, e);
                            }
                        }
                    }
                }
                r.attr = attr;
            }

            STREAM_ENCRYPTED_SESSION_DATA => {
                if r.extract {
                    // No PKI keys can be configured for this File Daemon.
                    self.jmsg(M_ERROR, "No private decryption keys have been defined to decrypt encrypted backup data.\n");
                    r.extract = false;
                    r.bfd = None;
                }
            }

            STREAM_FILE_DATA | STREAM_SPARSE_DATA | STREAM_WIN32_DATA | STREAM_GZIP_DATA
            | STREAM_SPARSE_GZIP_DATA | STREAM_WIN32_GZIP_DATA | STREAM_COMPRESSED_DATA
            | STREAM_SPARSE_COMPRESSED_DATA | STREAM_WIN32_COMPRESSED_DATA | STREAM_ENCRYPTED_FILE_DATA
            | STREAM_ENCRYPTED_WIN32_DATA | STREAM_ENCRYPTED_FILE_GZIP_DATA | STREAM_ENCRYPTED_WIN32_GZIP_DATA
            | STREAM_ENCRYPTED_FILE_COMPRESSED_DATA | STREAM_ENCRYPTED_WIN32_COMPRESSED_DATA => {
                if !r.extract {
                    return true;
                }
                let process = r.prev_stream == r.stream
                    || matches!(r.prev_stream, STREAM_UNIX_ATTRIBUTES | STREAM_UNIX_ATTRIBUTES_EX | STREAM_ENCRYPTED_SESSION_DATA);
                if !process {
                    return true;
                }
                r.sparse = matches!(r.stream, STREAM_SPARSE_DATA | STREAM_SPARSE_GZIP_DATA | STREAM_SPARSE_COMPRESSED_DATA);
                r.compress = matches!(r.stream,
                    STREAM_SPARSE_GZIP_DATA | STREAM_SPARSE_COMPRESSED_DATA | STREAM_GZIP_DATA | STREAM_COMPRESSED_DATA
                    | STREAM_WIN32_GZIP_DATA | STREAM_WIN32_COMPRESSED_DATA);
                if matches!(r.stream,
                    STREAM_ENCRYPTED_FILE_DATA | STREAM_ENCRYPTED_WIN32_DATA | STREAM_ENCRYPTED_FILE_GZIP_DATA
                    | STREAM_ENCRYPTED_WIN32_GZIP_DATA | STREAM_ENCRYPTED_FILE_COMPRESSED_DATA
                    | STREAM_ENCRYPTED_WIN32_COMPRESSED_DATA)
                {
                    // SetupDecryptionContext() without session data (rctx.cs)
                    let f = Self::last_fname(r);
                    self.jmsg(M_ERROR, format!("Missing encryption session data stream for {f}\n"));
                    r.extract = false;
                    r.bfd = None;
                    return true;
                }
                if self.extract_data(r, data).is_none() {
                    r.extract = false;
                    r.bfd = None;
                }
            }

            STREAM_ENCRYPTED_MACOS_FORK_DATA | STREAM_MACOS_FORK_DATA => ns.rsrc += 1,
            STREAM_HFSPLUS_ATTRIBUTES => ns.finfo += 1,

            s if is_acl_stream(s) => {
                if (!r.extract && r.last_type != FT_DIREND) || r.last_fname.is_empty() {
                    return true;
                }
                if r.last_type != FT_DIREND {
                    r.delayed.push((s, data.to_vec()));
                } else if !self.do_restore_acl(r, ax, s, data) {
                    return false;
                }
            }

            s if is_xattr_stream(s) => {
                if (!r.extract && r.last_type != FT_DIREND) || r.last_fname.is_empty() {
                    return true;
                }
                if r.last_type != FT_DIREND && s != STREAM_XATTR_SOLARIS {
                    r.delayed.push((s, data.to_vec()));
                } else if !self.do_restore_xattr(r, ax, s, data) {
                    return false;
                }
            }

            STREAM_SIGNED_DIGEST => {
                // Signature verification needs PKI signer keys, which this
                // File Daemon cannot be configured with.
                if r.extract {
                    ns.crypto += 1;
                }
            }

            STREAM_MD5_DIGEST | STREAM_SHA1_DIGEST | STREAM_SHA256_DIGEST | STREAM_SHA512_DIGEST | STREAM_XXH128_DIGEST => {}

            STREAM_PROGRAM_NAMES | STREAM_PROGRAM_DATA => {
                if ns.progname == 0 {
                    tracing::info!("Got Program Name or Data Stream. Ignored.");
                }
                ns.progname += 1;
            }

            STREAM_PLUGIN_NAME => {
                if !self.close_previous_stream(r, ax) {
                    return false;
                }
                // "<index> 1 <portable> <line>\0" opens, "<index> 0" closes.
                let text = String::from_utf8_lossy(data);
                let text = text.trim_end_matches('\0');
                let mut fields = text.splitn(4, ' ');
                let _index = fields.next();
                r.plugin = match (fields.next(), fields.nth(1)) {
                    (Some("1"), Some(line)) => Some(crate::secrets::mask_plugin_secrets(line)),
                    _ => None,
                };
                r.online = None;
                if let Some(shown) = r.plugin.clone() {
                    if !self.start_online(r, &shown) {
                        return false;
                    }
                }
            }

            STREAM_RESTORE_OBJECT => {}

            s => {
                if !self.close_previous_stream(r, ax) {
                    return false;
                }
                self.jmsg(M_WARNING, format!("Unknown stream={s} ignored. This shouldn't happen!\n"));
            }
        }
        true
    }

    /// A plugin-name record opened plugin files: when the job asked for an
    /// online restore of this plugin, start (or reuse) its session.
    fn start_online(&mut self, r: &mut Rctx, shown: &str) -> bool {
        let Ok(stored) = plugin::parse_line(shown) else { return true };
        let Some(line) = online_line(&stored, self.plugin_options) else { return true };
        if !r.sessions.contains_key(&line.name) {
            let Some(config) = self.config else {
                self.jmsg(M_FATAL, format!("Plugin: \"{shown}\": online restore is not available here.\n"));
                return false;
            };
            let job = pb::JobContext {
                job_id: u64::from(self.msgs.job_id),
                job_name: if self.msgs.job.is_empty() { "restore".into() } else { self.msgs.job.clone() },
                operation: pb::JobOperation::Restore as i32,
            };
            let opened = self.open_session(config, &line, job, r.sessions.len());
            match opened {
                Ok(session) => {
                    self.jmsg(M_INFO, format!("Plugin: \"{shown}\": restoring online through {}.\n", session.plugin_id));
                    r.sessions.insert(line.name.clone(), session);
                }
                Err(e) => {
                    self.jmsg(M_FATAL, format!("Plugin: \"{shown}\": online restore: {e}\n"));
                    return false;
                }
            }
        }
        r.online = Some(line.name);
        true
    }

    fn open_session(&self, config: &crate::config::FdConfig, line: &PluginLine, job: pb::JobContext, index: usize)
        -> Result<Session, PluginError> {
        let flag: &AtomicBool = self.canceled;
        let canceled = move || flag.load(Ordering::Relaxed);
        #[cfg(test)]
        if let Some(open) = test_hook::take() {
            let mut s = open(line, job, index)?;
            s.start(&canceled)?;
            return Ok(s);
        }
        let mut s = Session::open(config, line, job, index, ONLINE_CHUNK)?;
        s.start(&canceled)?;
        Ok(s)
    }

    /// The attributes of a plugin file in an online restore: no file on
    /// disk; directories and links are passed to the plugin at once.
    fn open_online_file(&mut self, r: &mut Rctx, plugin: String, attr: Attributes) -> bool {
        let entry = plugin_entry(&attr);
        self.counters.num_files_examined += 1;
        r.last_fname = attr.fname.clone();
        r.last_type = attr.type_;
        self.jmsg(M_RESTORED, ls_output(&attr));
        self.counters.job_files += 1;
        r.attr = attr;
        if entry.kind == pb::FileKind::Virtual as i32 {
            r.extract = true;
            r.file_addr = 0;
            r.online_file = Some(OnlineFile { plugin, entry, offset: 0 });
            return true;
        }
        let flag: &AtomicBool = self.canceled;
        let canceled = move || flag.load(Ordering::Relaxed);
        let Some(session) = r.sessions.get(&plugin) else { return false };
        let done = session.write(&entry, 0, &[], &canceled).and_then(|()| session.close(&entry.path, false, &canceled));
        self.online_result(done, &entry.path)
    }

    fn online_result(&mut self, result: Result<(), PluginError>, path: &[u8]) -> bool {
        match result {
            Ok(()) => true,
            Err(PluginError::Canceled) => false,
            Err(e) => {
                let name = String::from_utf8_lossy(path).into_owned();
                self.jmsg(M_FATAL, format!("Plugin: online restore of \"{name}\": {e}\n"));
                false
            }
        }
    }

    /// One block of an online-restored file to its plugin.
    fn write_online(&mut self, r: &mut Rctx, data: &[u8]) -> bool {
        let flag: &AtomicBool = self.canceled;
        let canceled = move || flag.load(Ordering::Relaxed);
        let Some(of) = r.online_file.as_mut() else { return false };
        let Some(session) = r.sessions.get(&of.plugin) else { return false };
        let mut result = Ok(());
        for block in data.chunks(session.chunk as usize) {
            result = session.write(&of.entry, of.offset, block, &canceled);
            if result.is_err() {
                break;
            }
            of.offset += block.len() as u64;
        }
        let path = of.entry.path.clone();
        self.counters.job_bytes += data.len() as u64;
        self.online_result(result, &path)
    }

    /// The end of an online-restored file: CloseFile (abort when it failed).
    fn close_online(&mut self, r: &mut Rctx, abort: bool) -> bool {
        let Some(of) = r.online_file.take() else { return true };
        let flag: &AtomicBool = self.canceled;
        let canceled = move || flag.load(Ordering::Relaxed);
        let Some(session) = r.sessions.get(&of.plugin) else { return false };
        let result = session.close(&of.entry.path, abort, &canceled);
        self.online_result(result, &of.entry.path)
    }

    /// ExtractData(); None on errors (already reported).
    fn extract_data(&mut self, r: &mut Rctx, buf: &[u8]) -> Option<usize> {
        self.counters.read_bytes += buf.len() as u64;
        let mut data: &[u8] = buf;
        let fname = Self::last_fname(r);
        if r.sparse {
            if data.len() < 8 {
                self.jmsg(M_ERROR, format!("Seek to 0 error on {fname}: ERR=Invalid argument\n"));
                return None;
            }
            let faddr = u64::from_be_bytes(data[..8].try_into().unwrap());
            if let Some(of) = r.online_file.as_mut() {
                of.offset = faddr;
                r.file_addr = faddr;
            }
            if r.file_addr != faddr {
                r.file_addr = faddr;
                let f = r.bfd.as_mut()?;
                if let Err(e) = f.seek(SeekFrom::Start(faddr)) {
                    self.jmsg(M_ERROR, format!("Seek to {faddr} error on {fname}: ERR={}\n", crate::berrno::s(&e)));
                    return None;
                }
            }
            data = &data[8..];
        }
        let decompressed;
        if r.compress {
            match compression::decompress_data(&fname, r.stream, data) {
                Ok(d) => {
                    decompressed = d;
                    data = &decompressed;
                }
                Err(m) => {
                    self.jmsg(M_ERROR, m);
                    return None;
                }
            }
        }
        if r.online_file.is_some() {
            let n = data.len();
            return self.write_online(r, data).then_some(n);
        }
        let f = r.bfd.as_mut()?;
        let res = match r.win32.as_mut() {
            Some(w) => w.process(data, f).map_err(|e| format!("Write error in Win32 Block Decomposition on {fname}: {}\n", crate::berrno::s(&e))),
            None => f.write_all(data).map_err(|e| format!("Write error on {fname}: {}\n", crate::berrno::s(&e))),
        };
        if let Err(m) = res {
            self.jmsg(M_ERROR, m);
            return None;
        }
        self.counters.job_bytes += data.len() as u64;
        r.file_addr += data.len() as u64;
        Some(data.len())
    }

    /// do_reStoreAcl()
    fn do_restore_acl(&mut self, r: &Rctx, ax: &mut AclXattrState, stream: i32, content: &[u8]) -> bool {
        match acl_xattr::restore_acl(&r.last_fname, r.last_type, stream, content) {
            Exit::Ok => true,
            Exit::Fatal(m) => {
                self.jmsg(M_FATAL, m);
                false
            }
            Exit::Error(m) | Exit::Warning(m) => {
                if ax.acl_errors < REPORT_ERR_MAX_PER_JOB {
                    self.jmsg(M_WARNING, m);
                }
                ax.acl_errors += 1;
                true
            }
        }
    }

    /// do_restore_xattr() + HandleXattrExitCode()
    fn do_restore_xattr(&mut self, r: &Rctx, ax: &mut AclXattrState, stream: i32, content: &[u8]) -> bool {
        match acl_xattr::restore_xattr(&r.last_fname, stream, content, ax) {
            Exit::Ok => true,
            Exit::Fatal(m) => {
                self.jmsg(M_FATAL, m);
                false
            }
            Exit::Warning(m) => {
                self.jmsg(M_WARNING, m);
                true
            }
            Exit::Error(m) => {
                if !m.is_empty() {
                    self.jmsg(M_ERROR, m);
                }
                ax.xattr_errors += 1;
                true
            }
        }
    }

    /// ClosePreviousStream()
    fn close_previous_stream(&mut self, r: &mut Rctx, ax: &mut AclXattrState) -> bool {
        if r.online_file.is_some() {
            r.extract = false;
            r.type_ = r.attr.type_;
            r.delayed.clear();
            return self.close_online(r, false);
        }
        if r.extract {
            if r.size > 0 && r.bfd.is_none() {
                self.jmsg(M_ERROR, "Logic error: output file should be open\n");
            }
            for e in set_attributes(&r.attr, r.bfd.take()) {
                self.jmsg(M_ERROR, e);
            }
            r.extract = false;
            // PopDelayedDataStreams()
            for (s, content) in std::mem::take(&mut r.delayed) {
                let ok = if is_acl_stream(s) {
                    self.do_restore_acl(r, ax, s, &content)
                } else if is_xattr_stream(s) {
                    self.do_restore_xattr(r, ax, s, &content)
                } else {
                    self.jmsg(M_WARNING, format!("Unknown stream={s} ignored. This shouldn't happen!\n"));
                    true
                };
                if !ok {
                    return false;
                }
            }
            r.type_ = r.attr.type_;
        } else if r.bfd.is_some() {
            self.jmsg(M_ERROR, "Logic error: output file should not be open\n");
            r.bfd = None;
        }
        r.delayed.clear();
        true
    }
}

/// Tests open online-restore sessions from a test bundle (outside the
/// root-owned plugin directory) through this hook.
#[cfg(test)]
pub(crate) mod test_hook {
    use super::*;
    type Open = Box<dyn FnOnce(&PluginLine, pb::JobContext, usize) -> Result<Session, PluginError>>;
    thread_local!(static OPEN: std::cell::RefCell<Option<Open>> = const { std::cell::RefCell::new(None) });
    pub fn set(open: Open) {
        OPEN.with(|o| *o.borrow_mut() = Some(open));
    }
    pub fn take() -> Option<Open> {
        OPEN.with(|o| o.borrow_mut().take())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// A stream that reads prepared SD output and discards what is written.
    struct FakeSd {
        input: Cursor<Vec<u8>>,
    }
    impl Read for FakeSd {
        fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(b)
        }
    }
    impl Write for FakeSd {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn b64(v: i64) -> String {
        const MAP: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut u = v as u64;
        let mut out = Vec::new();
        if u == 0 {
            out.push(b'A');
        }
        while u > 0 {
            out.push(MAP[(u & 63) as usize]);
            u >>= 6;
        }
        out.reverse();
        String::from_utf8(out).unwrap()
    }

    fn lstat(mode: u32, size: i64, mtime: i64, data_stream: i32) -> String {
        let uid = nix::unistd::getuid().as_raw() as i64;
        let gid = nix::unistd::getgid().as_raw() as i64;
        [1, 2, mode as i64, 1, uid, gid, 0, size, 4096, 1, mtime, mtime, mtime, 0, 0, data_stream as i64]
            .iter().map(|v| b64(*v)).collect::<Vec<_>>().join(" ")
    }

    struct Rec(Vec<u8>);
    impl Rec {
        fn add(&mut self, fi: i32, stream: i32, data: &[u8]) {
            let hdr = format!("rechdr 1 2 {fi} {stream} {}", data.len());
            for m in [hdr.as_bytes(), data] {
                self.0.extend_from_slice(&(m.len() as i32).to_be_bytes());
                self.0.extend_from_slice(m);
            }
        }
        fn attr(&mut self, fi: i32, ft: i32, name: &str, lstat: &str, link: &str) {
            let rec = format!("{fi} {ft} {name}\0{lstat}\0{link}\0\0");
            self.add(fi, STREAM_UNIX_ATTRIBUTES, rec.as_bytes());
        }
    }

    #[test]
    fn restores_files_dirs_links_sparse_and_gzip() {
        let tmp = std::env::temp_dir().join(format!("ofd-restore-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let mut r = Rec(Vec::new());
        let t = 1_600_000_000;
        // plain file
        r.attr(1, FT_REG, "/src/a.txt", &lstat(0o100640, 11, t, STREAM_FILE_DATA), "");
        r.add(1, STREAM_FILE_DATA, b"hello ");
        r.add(1, STREAM_FILE_DATA, b"world");
        r.add(1, STREAM_MD5_DIGEST, &[0u8; 16]);
        // sparse file: 4 bytes at offset 0 and 4 bytes at 1 MiB
        r.attr(2, FT_REG, "/src/sparse", &lstat(0o100600, (1 << 20) + 4, t, STREAM_SPARSE_DATA), "");
        let mut blk = 0u64.to_be_bytes().to_vec();
        blk.extend_from_slice(b"head");
        r.add(2, STREAM_SPARSE_DATA, &blk);
        let mut blk = (1u64 << 20).to_be_bytes().to_vec();
        blk.extend_from_slice(b"tail");
        r.add(2, STREAM_SPARSE_DATA, &blk);
        // gzip (compressed with header)
        let text = b"compressible ".repeat(100);
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&text).unwrap();
        let z = enc.finish().unwrap();
        let mut c = compression::COMPRESS_GZIP.to_be_bytes().to_vec();
        c.extend_from_slice(&(z.len() as u32).to_be_bytes());
        c.extend_from_slice(&6u16.to_be_bytes());
        c.extend_from_slice(&1u16.to_be_bytes());
        c.extend_from_slice(&z);
        r.attr(3, FT_REG, "/src/sub/c.txt", &lstat(0o100644, text.len() as i64, t, STREAM_COMPRESSED_DATA), "");
        r.add(3, STREAM_COMPRESSED_DATA, &c);
        // empty file, symlink, hard link, directory end
        r.attr(4, FT_REGE, "/src/empty", &lstat(0o100600, 0, t, 0), "");
        r.attr(5, FT_LNK, "/src/link", &lstat(0o120777, 5, t, 0), "a.txt");
        r.attr(6, FT_LNKSAVED, "/src/hard", &lstat(0o100640, 11, t, 0), "/src/a.txt");
        r.attr(7, FT_DIREND, "/src/sub/", &lstat(0o040750, 0, t, 0), "");
        r.attr(8, FT_DIREND, "/src/", &lstat(0o040755, 0, t, 0), "");
        r.0.extend_from_slice(&(BnetSignal::Eod as i32).to_be_bytes());

        let mut sd = BnetStream::new(FakeSd { input: Cursor::new(r.0) });
        let params = RestoreParams { where_: Where::Prefix(tmp.as_os_str().as_encoded_bytes().to_vec()), replace: b'a', prefix_links: false };
        let mut msgs = JobMsgs::new("test-fd");
        msgs.job_id = 1;
        let mut counters = Counters::default();
        let mut create = CreateState::default();
        let canceled = AtomicBool::new(false);
        let mut sent = Vec::new();
        Restore { params: &params, msgs: &mut msgs, counters: &mut counters, create: &mut create, canceled: &canceled, config: None, plugin_options: &[] }
            .run(&mut sd, &mut |m: &mut JobMsgs| sent.extend(m.take_queued()));
        assert_eq!(msgs.status(), JS_TERMINATED, "{sent:?}");
        assert_eq!(msgs.job_errors, 0, "{sent:?}");
        assert_eq!(counters.job_files, 8);

        let root = tmp.join("src");
        assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"hello world");
        let sp = std::fs::read(root.join("sparse")).unwrap();
        assert_eq!(sp.len(), (1 << 20) + 4);
        assert_eq!(&sp[..4], b"head");
        assert_eq!(&sp[1 << 20..], b"tail");
        assert_eq!(std::fs::read(root.join("sub/c.txt")).unwrap(), text);
        assert_eq!(std::fs::read(root.join("empty")).unwrap(), b"");
        assert_eq!(std::fs::read_link(root.join("link")).unwrap(), std::path::Path::new("a.txt"));
        use std::os::unix::fs::MetadataExt;
        let a = std::fs::metadata(root.join("a.txt")).unwrap();
        assert_eq!(a.ino(), std::fs::metadata(root.join("hard")).unwrap().ino());
        assert_eq!(a.mode() & 0o7777, 0o640);
        assert_eq!(a.mtime(), t);
        let d = std::fs::metadata(root.join("sub")).unwrap();
        assert_eq!((d.mode() & 0o7777, d.mtime()), (0o750, t));

        // replace=never: everything exists, so everything is skipped
        let mut r2 = Rec(Vec::new());
        r2.attr(1, FT_REG, "/src/a.txt", &lstat(0o100640, 3, t, STREAM_FILE_DATA), "");
        r2.add(1, STREAM_FILE_DATA, b"new");
        r2.0.extend_from_slice(&(BnetSignal::Eod as i32).to_be_bytes());
        let mut sd = BnetStream::new(FakeSd { input: Cursor::new(r2.0) });
        let params = RestoreParams { replace: b'n', ..params };
        let mut msgs = JobMsgs::new("test-fd");
        msgs.job_id = 1;
        let mut sent = Vec::new();
        Restore { params: &params, msgs: &mut msgs, counters: &mut Counters::default(), create: &mut CreateState { keep_path_list: true, ..Default::default() }, canceled: &canceled, config: None, plugin_options: &[] }
            .run(&mut sd, &mut |m: &mut JobMsgs| sent.extend(m.take_queued()));
        assert!(sent.iter().any(|m| m.contains("File skipped. Already exists:")), "{sent:?}");
        assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"hello world");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    fn plugin_records(t: i64) -> Vec<u8> {
        let mut r = Rec(Vec::new());
        let line = "postgresql:instance=main:password=*****";
        // The directory first, as a plugin may list it.
        r.add(9, STREAM_PLUGIN_NAME, format!("9 1 1 {line}\0").as_bytes());
        r.attr(9, FT_DIREND, "/@postgresql/main/db/a", &lstat(0o040750, 0, t, 0), "/@postgresql/main/db/a/");
        r.add(9, STREAM_PLUGIN_NAME, b"9 0");
        r.add(1, STREAM_PLUGIN_NAME, format!("1 1 1 {line}\0").as_bytes());
        r.attr(1, FT_REG, "/@postgresql/main/db/a/a.dump", &lstat(0o100600, 5, t, STREAM_FILE_DATA), "");
        r.add(1, STREAM_FILE_DATA, b"hello");
        r.add(1, STREAM_PLUGIN_NAME, b"1 0");
        r.add(2, STREAM_PLUGIN_NAME, format!("2 1 1 {line}\0").as_bytes());
        r.attr(2, FT_REG, "/@postgresql/main/globals.sql", &lstat(0o100600, 3, t, STREAM_FILE_DATA), "");
        r.add(2, STREAM_FILE_DATA, b"sql");
        r.add(2, STREAM_PLUGIN_NAME, b"2 0");
        r.0.extend_from_slice(&(BnetSignal::Eod as i32).to_be_bytes());
        r.0
    }
    fn run_restore(input: Vec<u8>, where_: Vec<u8>) -> (JobMsgs, Counters, Vec<String>) {
        let mut sd = BnetStream::new(FakeSd { input: Cursor::new(input) });
        let params = RestoreParams { where_: Where::Prefix(where_), replace: b'a', prefix_links: false };
        let mut msgs = JobMsgs::new("test-fd");
        msgs.job_id = 1;
        let mut counters = Counters::default();
        let canceled = AtomicBool::new(false);
        let mut sent = Vec::new();
        Restore { params: &params, msgs: &mut msgs, counters: &mut counters, create: &mut CreateState::default(), canceled: &canceled, config: None, plugin_options: &[] }
            .run(&mut sd, &mut |m: &mut JobMsgs| sent.extend(m.take_queued()));
        (msgs, counters, sent)
    }

    /// Plugin files restore to files below Where without loading the plugin:
    /// a backup stays restorable when the plugin is missing or revoked.
    #[test]
    fn plugin_files_restore_to_where_without_the_plugin() {
        let tmp = std::env::temp_dir().join(format!("ofd-restore-plugin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let (msgs, counters, sent) = run_restore(plugin_records(1_600_000_000), tmp.as_os_str().as_encoded_bytes().to_vec());
        assert_eq!(msgs.status(), JS_TERMINATED, "{sent:?}");
        assert_eq!(counters.job_files, 3);
        assert_eq!(std::fs::read(tmp.join("@postgresql/main/db/a/a.dump")).unwrap(), b"hello");
        use std::os::unix::fs::MetadataExt;
        let dir = std::fs::metadata(tmp.join("@postgresql/main/db/a")).unwrap();
        assert_eq!((dir.mode() & 0o7777, dir.mtime()), (0o750, 1_600_000_000), "directory attributes set after its content");
        assert_eq!(std::fs::read(tmp.join("@postgresql/main/globals.sql")).unwrap(), b"sql");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn online_options_replace_masked_values() {
        let stored = plugin::parse_line("postgresql:instance=main:password=*****").unwrap();
        let opts = vec!["openldap:restore_mode=online".to_string(),
            "postgresql:restore_mode=online:password=neu:target_db=x".to_string()];
        let line = online_line(&stored, &opts).unwrap();
        assert_eq!(line.options["instance"], "main");
        assert_eq!(line.options["password"], "neu");
        assert_eq!(line.options["target_db"], "x");
        assert!(online_line(&stored, &["postgresql:target_db=x".to_string()]).is_none());
    }

    #[test]
    fn plugin_files_restore_online_through_the_plugin() {
        use crate::plugin::testkit::*;
        let dir = std::env::temp_dir().join(format!("ofd-restore-online-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let resolved = signed_plugin(&dir, "postgresql", &Script::default());
        let opened = std::rc::Rc::new(std::cell::Cell::new(None::<String>));
        let seen = opened.clone();
        test_hook::set(Box::new(move |line, job, index| {
            seen.set(Some(format!("{}:{}", line.name, line.options.get("password").cloned().unwrap_or_default())));
            let runtime = onesimus_plugins::wasm::RuntimeLimits { max_chunk_bytes: 4, ..Default::default() };
            Session::open_resolved(&resolved, line, job, index, runtime)
        }));
        let config = crate::config::FdConfig::parse("Director { Name = d; Password = p }").unwrap();
        let options = vec!["postgresql:restore_mode=online:password=neu".to_string()];
        let mut sd = BnetStream::new(FakeSd { input: Cursor::new(plugin_records(1_600_000_000)) });
        let params = RestoreParams { where_: Where::Prefix(Vec::new()), replace: b'a', prefix_links: false };
        let mut msgs = JobMsgs::new("test-fd");
        msgs.job_id = 77;
        let mut counters = Counters::default();
        let canceled = AtomicBool::new(false);
        let mut sent = Vec::new();
        Restore { params: &params, msgs: &mut msgs, counters: &mut counters, create: &mut CreateState::default(),
            canceled: &canceled, config: Some(&config), plugin_options: &options }
            .run(&mut sd, &mut |m: &mut JobMsgs| sent.extend(m.take_queued()));
        assert_eq!(msgs.status(), JS_TERMINATED, "{sent:?}");
        assert_eq!(counters.job_files, 3);
        // Blocks larger than the plugin's chunk (5 bytes, chunk 4) are split.
        assert_eq!(counters.job_bytes, 8);
        assert_eq!(opened.take().as_deref(), Some("postgresql:neu"));
        assert!(sent.iter().any(|m| m.contains("restoring online through test.postgresql")), "{sent:?}");
        assert!(!std::path::Path::new("/@postgresql").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn plugin_files_without_where_are_refused() {
        let (msgs, counters, sent) = run_restore(plugin_records(1_600_000_000), Vec::new());
        assert_ne!(msgs.status(), JS_TERMINATED);
        assert_eq!(counters.job_files, 0);
        assert!(sent.iter().any(|m| m.contains("can only be restored to files: set Where")), "{sent:?}");
        assert!(!std::path::Path::new("/@postgresql").exists());
    }

    #[test]
    fn bad_header_is_fatal() {
        let mut v = Vec::new();
        v.extend_from_slice(&5i32.to_be_bytes());
        v.extend_from_slice(b"bogus");
        let mut sd = BnetStream::new(FakeSd { input: Cursor::new(v) });
        let params = RestoreParams { where_: Where::Prefix(b"/nonexistent".to_vec()), replace: b'a', prefix_links: false };
        let mut msgs = JobMsgs::new("t");
        msgs.job_id = 1;
        let canceled = AtomicBool::new(false);
        let mut sent = Vec::new();
        Restore { params: &params, msgs: &mut msgs, counters: &mut Counters::default(), create: &mut CreateState::default(), canceled: &canceled, config: None, plugin_options: &[] }
            .run(&mut sd, &mut |m: &mut JobMsgs| sent.extend(m.take_queued()));
        assert_eq!(msgs.status(), JS_FATAL_ERROR);
        assert!(sent.iter().any(|m| m.contains("Fatal error: Record header scan error: bogus")), "{sent:?}");
    }

    #[test]
    fn win32_decomposition() {
        let tmp = std::env::temp_dir().join(format!("ofd-w32-{}", std::process::id()));
        let mut s = Vec::new();
        // a security stream (id 3) with a name, then BACKUP_DATA
        for (id, name, content) in [(3u32, &b"n\0a\0"[..], &b"SEC"[..]), (1, b"", b"payload")] {
            s.extend_from_slice(&id.to_le_bytes());
            s.extend_from_slice(&0u32.to_le_bytes());
            s.extend_from_slice(&(content.len() as u64).to_le_bytes());
            s.extend_from_slice(&(name.len() as u32).to_le_bytes());
            s.extend_from_slice(name);
            s.extend_from_slice(content);
        }
        let mut f = File::create(&tmp).unwrap();
        let mut w = Win32Decomp::default();
        for chunk in s.chunks(7) {
            w.process(chunk, &mut f).unwrap();
        }
        drop(f);
        assert_eq!(std::fs::read(&tmp).unwrap(), b"payload");
        let _ = std::fs::remove_file(&tmp);
    }
}
