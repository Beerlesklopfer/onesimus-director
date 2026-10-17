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
    let s = std::str::from_utf8(msg).ok()?;
    let mut it = s.trim_end_matches(['\0', '\n']).split_whitespace();
    if it.next()? != "rechdr" {
        return None;
    }
    Some((it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?,
        it.next()?.parse().ok()?, it.next()?.parse().ok()?))
}

/// The restore of one SD read session.
pub struct Restore<'a> {
    pub params: &'a RestoreParams,
    pub msgs: &'a mut JobMsgs,
    pub counters: &'a mut Counters,
    pub create: &'a mut CreateState,
    pub canceled: &'a AtomicBool,
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
                        for e in set_attributes(&attr, None) {
                            self.jmsg(M_ERROR, e);
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
                // PluginNameStream() without loaded plugins does nothing.
                if !self.close_previous_stream(r, ax) {
                    return false;
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
        let tmp = std::env::temp_dir().join(format!("bareos-filed-restore-test-{}", std::process::id()));
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
        Restore { params: &params, msgs: &mut msgs, counters: &mut counters, create: &mut create, canceled: &canceled }
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
        Restore { params: &params, msgs: &mut msgs, counters: &mut Counters::default(), create: &mut CreateState { keep_path_list: true, ..Default::default() }, canceled: &canceled }
            .run(&mut sd, &mut |m: &mut JobMsgs| sent.extend(m.take_queued()));
        assert!(sent.iter().any(|m| m.contains("File skipped. Already exists:")), "{sent:?}");
        assert_eq!(std::fs::read(root.join("a.txt")).unwrap(), b"hello world");
        let _ = std::fs::remove_dir_all(&tmp);
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
        Restore { params: &params, msgs: &mut msgs, counters: &mut Counters::default(), create: &mut CreateState::default(), canceled: &canceled }
            .run(&mut sd, &mut |m: &mut JobMsgs| sent.extend(m.take_queued()));
        assert_eq!(msgs.status(), JS_FATAL_ERROR);
        assert!(sent.iter().any(|m| m.contains("Fatal error: Record header scan error: bogus")), "{sent:?}");
    }

    #[test]
    fn win32_decomposition() {
        let tmp = std::env::temp_dir().join(format!("bareos-filed-w32-{}", std::process::id()));
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
