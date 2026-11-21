//! Backup — FD side: filed/backup.cc (BlastDataToStorageDaemon, SaveFile,
//! send_data/SendDataToSd, EncodeAndSendAttributes, StripPath),
//! findlib/attribs.cc SelectDataStream, accurate.cc (AccurateCheckFile,
//! AccurateFinish), estimate.cc (TallyFile) and verify.cc (VerifyFile).
//!
//! Records to the Storage daemon: a header "<FileIndex> <Stream> 0", the
//! data message(s), BNET_EOD. Encryption/signing (PKI) and plugins are not
//! available in this File Daemon (no keys or plugins can be configured).

use std::collections::HashMap;
use std::io::{Read, Write};
use crate::platform::OsStrExt;
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
use std::sync::atomic::{AtomicBool, Ordering};

use bareos_bnet::{BnetSignal, BnetStream};

use crate::accurate::{self, FileList};
use crate::acl_xattr;
use crate::attribs::*;
use crate::compression;
use crate::digest::{self, Digest};
use crate::fileset::*;
use crate::find::*;
use crate::jmsg::*;
use crate::restore::Counters;
use crate::streams::*;
use crate::config::FdConfig;
use crate::plugin::{self, PluginError, Session};
use onesimus_proto_rs::onesimus::filed::plugin::v1 as pb;

pub use bareos_format::msg::M_NOTSAVED;

/// O_NOATIME (Linux only; elsewhere 0, like `#ifdef O_NOATIME` in Bareos
/// bfile.cc).
#[cfg(target_os = "linux")]
const O_NOATIME: i32 = libc::O_NOATIME;
/// O_NOATIME (Linux only; elsewhere 0, like `#ifdef O_NOATIME` in Bareos
/// bfile.cc).
#[cfg(all(unix, not(target_os = "linux")))]
const O_NOATIME: i32 = 0;

/// SelectDataStream() (no encryption). On Windows the data of a file that
/// is not marked `Portable` is the `BackupRead` stream (`STREAM_WIN32_DATA`)
/// with its security descriptor and alternate data streams.
pub fn select_data_stream(flags: Flags) -> i32 {
    #[cfg(windows)]
    if !flags.has(FO_PORTABLE) {
        return if flags.has(FO_COMPRESS) { STREAM_WIN32_COMPRESSED_DATA } else { STREAM_WIN32_DATA };
    }
    let stream = if flags.has(FO_SPARSE) || flags.has(FO_OFFSETS) { STREAM_SPARSE_DATA } else { STREAM_FILE_DATA };
    if flags.has(FO_COMPRESS) {
        if stream == STREAM_SPARSE_DATA { STREAM_SPARSE_COMPRESSED_DATA } else { STREAM_COMPRESSED_DATA }
    } else {
        stream
    }
}

/// do_strip()
fn do_strip(count: i32, input: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut numsep = 0;
    while i < input.len() && input[i] != b'/' {
        out.push(input[i]);
        i += 1;
    }
    if i < input.len() {
        out.push(input[i]);
        i += 1;
        numsep += 1;
    }
    let mut stripped = 0;
    while stripped < count && i < input.len() {
        while i < input.len() && input[i] != b'/' {
            i += 1;
        }
        if i < input.len() {
            numsep += 1;
            i += 1;
        }
        stripped += 1;
    }
    while i < input.len() {
        if input[i] == b'/' {
            numsep += 1;
        }
        out.push(input[i]);
        i += 1;
    }
    (stripped == count && numsep > count).then_some(out)
}

/// StripPath(): the (fname, link) to send; unchanged when stripping fails.
pub fn stripped_names(ff: &FfPkt) -> (Vec<u8>, Vec<u8>) {
    let (fname, link) = (ff.fname.clone(), ff.link.clone());
    if !ff.opts.flags.has(FO_STRIPPATH) || ff.strip_path <= 0 {
        return (fname, link);
    }
    let link_differs = ff.link != ff.fname;
    let mut nf = fname.clone();
    if (ff.type_ != FT_DIREND && ff.type_ != FT_REPARSE) || !link_differs {
        match do_strip(ff.strip_path, &fname) {
            Some(s) => nf = s,
            None => return (fname, link),
        }
    }
    let mut nl = link.clone();
    if ff.type_ != FT_LNK && link_differs {
        match do_strip(ff.strip_path, &link) {
            Some(s) => nl = s,
            None => return (fname, link),
        }
    } else if !link_differs {
        nl = nf.clone();
    }
    (nf, nl)
}

/// IsBufZero()
fn is_buf_zero(b: &[u8]) -> bool {
    b.iter().all(|&x| x == 0)
}

/// The SD connection with the job counters (store_bsock side of the jcr).
pub struct SdSender<'a, S: Read + Write> {
    pub sd: &'a mut BnetStream<S>,
    pub msgs: &'a mut JobMsgs,
    pub counters: &'a mut Counters,
    /// CancelCmd() marks the socket terminated: sends fail, without a
    /// message (IsJobCanceled()).
    pub canceled: &'a AtomicBool,
    /// `status` shows these numbers while the job runs.
    pub progress: Option<&'a crate::sd::Progress>,
}

impl<S: Read + Write> SdSender<'_, S> {
    /// Publishes the counters (and the file being saved) for `status`.
    fn publish(&self, file: Option<&str>) {
        if let Some(p) = self.progress {
            let c = &*self.counters;
            p.update(c.job_files, c.job_bytes, c.num_files_examined, self.msgs.job_errors);
            if let Some(f) = file {
                p.set_file(f);
            }
        }
    }
    fn is_canceled(&self) -> bool {
        self.canceled.load(Ordering::Relaxed) || self.msgs.is_canceled()
    }
    fn send(&mut self, data: &[u8]) -> bool {
        if self.is_canceled() {
            return false;
        }
        if self.sd.send(data).is_err() {
            if !self.is_canceled() {
                self.msgs.jmsg(M_FATAL, "Network send error to SD. ERR=Broken pipe\n");
            }
            return false;
        }
        true
    }
    fn header(&mut self, stream: i32) -> bool {
        let h = bareos_format::record::data_header(self.counters.job_files, stream);
        self.send(h.as_bytes())
    }
    fn eod(&mut self) -> bool {
        if self.is_canceled() {
            return false;
        }
        if self.sd.send_signal(BnetSignal::Eod).is_err() {
            if !self.is_canceled() {
                self.msgs.jmsg(M_FATAL, "Network send error to SD. ERR=Broken pipe\n");
            }
            return false;
        }
        true
    }
}

/// What a FindFiles() handler of this module needs besides the packet.
pub struct JobEnv<'a> {
    pub canceled: &'a AtomicBool,
    pub accurate: Option<&'a mut FileList>,
    pub level_full: bool,
    pub rerunning: bool,
    /// jcr->fd_impl->base_size
    pub base_size: u64,
}

/// The reader of a file's data for a backup: the file itself, on Windows
/// the `BackupRead` stream when the data stream is a Win32 one.
fn open_for_backup(ff: &FfPkt, data_stream: i32) -> std::io::Result<Box<dyn Read>> {
    #[cfg(windows)]
    if is_win32_stream(data_stream) {
        return crate::winfs::open_backup_reader(&ff.fname).map(|r| Box::new(r) as Box<dyn Read>);
    }
    let _ = data_stream;
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    if ff.opts.flags.has(FO_NOATIME) {
        opts.custom_flags(O_NOATIME);
    }
    opts.open(std::ffi::OsStr::from_bytes(&ff.fname)).map(|f| Box::new(f) as Box<dyn Read>)
}

/// calculate_file_chksum(): (digest name, base64 digest) of the whole file
/// (zero blocks skipped like ReadDigest() when FO_SPARSE).
pub fn file_checksum(ff: &FfPkt, counters: &mut Counters, msgs: &mut JobMsgs, is_verify: bool) -> Option<Option<(&'static str, i32, String)>> {
    let (mut d, stream) = Digest::for_flags(ff.opts.flags)?;
    let name = d.name();
    let fname = String::from_utf8_lossy(&ff.fname).into_owned();
    let mut f = match open_for_backup(ff, select_data_stream(ff.opts.flags)) {
        Ok(f) => f,
        Err(e) => {
            msgs.jmsg(M_ERROR, &format!("     Cannot open {fname}: ERR={}.\n", crate::berrno::s(&e)));
            msgs.job_errors += 1;
            return Some(None);
        }
    };
    const BUFSIZ: usize = 256 * 1024;
    let mut buf = vec![0u8; BUFSIZ];
    let mut file_addr: u64 = 0;
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if ff.opts.flags.has(FO_SPARSE) {
                    let zero = ((n == BUFSIZ && file_addr + (n as u64) < ff.statp.size as u64)
                        || ((ff.type_ == FT_RAW || ff.type_ == FT_FIFO) && ff.statp.size == 0))
                        && is_buf_zero(&buf);
                    file_addr += n as u64;
                    if zero {
                        continue;
                    }
                }
                d.update(&buf[..n]);
                if is_verify {
                    counters.job_bytes += n as u64;
                }
                counters.read_bytes += n as u64;
            }
            Err(e) => {
                msgs.jmsg(M_ERROR, &format!("Error reading file {fname}: ERR={}\n", crate::berrno::s(&e)));
                msgs.job_errors += 1;
                return Some(None);
            }
        }
    }
    Some(Some((name, stream, digest::to_base64(&d.finalize()))))
}

/// AccurateCheckFile()
pub fn accurate_check_file(ff: &mut FfPkt, env: &mut JobEnv, counters: &mut Counters, msgs: &mut JobMsgs) -> bool {
    ff.delta_seq = 0;
    ff.accurate_found = false;
    let Some(list) = env.accurate.as_deref_mut() else { return true };
    let (sf, sl) = stripped_names(ff);
    let is_dir = ff.statp.mode & crate::stmode::S_IFMT == crate::stmode::S_IFDIR;
    let key = if is_dir { sl } else { sf };
    let Some(payload) = list.files.get(&key).cloned() else { return true };
    ff.accurate_found = true;
    ff.delta_seq = payload.delta_seq;
    let (cat, _, _) = decode_stat(&payload.lstat);
    let opts = if !env.rerunning && env.level_full { ff.basejob_opts.clone() } else { ff.accurate_opts.clone() };
    let can_checksum = ff.type_ != FT_LNKSAVED && ff.statp.mode & crate::stmode::S_IFMT == crate::stmode::S_IFREG && ff.opts.flags.any_digest();
    let fname = String::from_utf8_lossy(&ff.fname).into_owned();
    let mut warnings = Vec::new();
    let status = {
        let ffc = ff.clone();
        let mut cmp = || match file_checksum(&ffc, counters, msgs, false) {
            Some(Some((_, _, b64))) => Some(b64 == payload.chksum),
            _ => Some(false),
        };
        accurate::changed(&opts, &cat, &ff.statp, can_checksum, &payload.chksum, env.rerunning, &mut cmp,
            &mut |w| warnings.push(w), &fname)
    };
    for w in warnings {
        msgs.jmsg(M_WARNING, &w);
    }
    if let Some(p) = list.files.get_mut(&key) {
        if env.level_full {
            if !status {
                env.base_size += ff.statp.size as u64;
                p.seen = true;
            }
        } else {
            p.seen = true;
        }
    }
    status
}

/// The backup of one job (b_ctx/b_save_ctx and the jcr fields used).
pub struct Backup<'a, 'b, S: Read + Write> {
    pub sd: SdSender<'a, S>,
    pub env: JobEnv<'b>,
    pub flush: &'a mut dyn FnMut(&mut JobMsgs),
    /// jcr->buf_size
    pub buf_size: usize,
    /// MaximumWorkersPerJob (> 0: the pipelined send of SendPlainData)
    pub workers: u32,
    pub acl_errors: u32,
    pub xattr_errors: u32,
    pub xattr_disabled: bool,
    /// The File Daemon configuration (plugin directives).
    pub config: &'b FdConfig,
    /// Plugin lines started in this job: the instance index of the next one.
    pub plugin_index: usize,
}

/// ReadFile of a plugin as a reader for send_data().
struct PluginReader<'s, 'c> {
    session: &'s Session,
    path: Vec<u8>,
    offset: u64,
    eof: bool,
    canceled: &'c dyn Fn() -> bool,
    error: Option<PluginError>,
}

impl Read for PluginReader<'_, '_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        // A block larger than buf would be lost: ask for at most buf.len()
        // by reading only when buf can take a whole chunk.
        if self.eof || buf.is_empty() {
            return Ok(0);
        }
        if buf.len() < self.session.chunk as usize {
            return Err(std::io::Error::other("plugin read buffer smaller than its chunk"));
        }
        match self.session.read(&self.path, self.offset, self.canceled) {
            Ok((data, eof)) => {
                self.eof = eof;
                self.offset += data.len() as u64;
                buf[..data.len()].copy_from_slice(&data);
                Ok(data.len())
            }
            Err(e) => {
                let text = e.to_string();
                self.error = Some(e);
                Err(std::io::Error::other(text))
            }
        }
    }
}

impl<S: Read + Write> Backup<'_, '_, S> {
    fn jmsg(&mut self, t: i32, text: impl AsRef<str>) {
        self.sd.msgs.jmsg(t, text.as_ref());
    }

    /// EncodeAndSendAttributes(); returns the data stream.
    pub fn encode_and_send_attributes(&mut self, ff: &mut FfPkt) -> Option<i32> {
        let data_stream = select_data_stream(ff.opts.flags);
        let attribs = encode_stat(&ff.statp, ff.link_fi, data_stream);
        // get_next_findex()
        self.sd.counters.job_files += 1;
        ff.file_index = self.sd.counters.job_files as i32;
        if !self.sd.header(STREAM_UNIX_ATTRIBUTES) {
            return None;
        }
        let (fname, link) = if ff.type_ != FT_DELETED { stripped_names(ff) } else { (ff.fname.clone(), ff.link.clone()) };
        let mut rec = format!("{} {} ", self.sd.counters.job_files, ff.type_).into_bytes();
        let delta = ff.delta_seq.to_string();
        match ff.type_ {
            FT_JUNCTION | FT_LNK | FT_LNKSAVED => {
                rec.extend_from_slice(&fname);
                rec.push(0);
                rec.extend_from_slice(attribs.as_bytes());
                rec.push(0);
                rec.extend_from_slice(&link);
                rec.push(0);
            }
            FT_DIREND | FT_REPARSE => {
                rec.extend_from_slice(&link);
                rec.push(0);
                rec.extend_from_slice(attribs.as_bytes());
                rec.push(0);
                rec.push(0);
            }
            _ => {
                rec.extend_from_slice(&fname);
                rec.push(0);
                rec.extend_from_slice(attribs.as_bytes());
                rec.push(0);
                rec.push(0);
            }
        }
        // attribsEx (the Win32 data on Windows, else empty), delta_seq
        rec.extend_from_slice(&ff.attribs_ex);
        rec.push(0);
        rec.extend_from_slice(delta.as_bytes());
        rec.push(0);
        let ok = self.sd.send(&rec);
        self.sd.eod();
        ok.then_some(data_stream)
    }

    /// SaveFile(): 1 ok, 0 error.
    pub fn save_file(&mut self, ff: &mut FfPkt, links: &mut HashMap<(u64, u64), CurLink>) -> i32 {
        if self.env.canceled.load(Ordering::Relaxed) || self.sd.msgs.is_canceled() {
            return 0;
        }
        (self.flush)(self.sd.msgs);
        self.sd.counters.num_files_examined += 1;
        let fname = String::from_utf8_lossy(&ff.fname).into_owned();
        let top = String::from_utf8_lossy(&ff.top_fname).into_owned();
        self.sd.publish(Some(&fname));
        let errno_text = |e: i32| crate::berrno::BErr(e).to_string();
        let mut has_file_data = false;
        match ff.type_ {
            FT_LNKSAVED | FT_LNK | FT_RESTORE_FIRST | FT_PLUGIN_CONFIG | FT_REPARSE | FT_JUNCTION | FT_DIREND | FT_FIFO
            | FT_DELETED => {}
            FT_REGE | FT_REG | FT_RAW => has_file_data = true,
            FT_DIRBEGIN => {
                self.sd.counters.num_files_examined -= 1;
                return 1;
            }
            FT_NORECURSE => {
                self.jmsg(M_INFO, format!("     Recursion turned off. Will not descend from {top} into {fname}\n"));
                ff.type_ = FT_DIREND;
            }
            FT_NOFSCHG => ff.type_ = FT_DIREND,
            FT_INVALIDFS => {
                self.jmsg(M_INFO, format!("     Disallowed filesystem. Will not descend from {top} into {fname}\n"));
                ff.type_ = FT_DIREND;
            }
            FT_INVALIDDT => {
                self.jmsg(M_INFO, format!("     Disallowed drive type. Will not descend into {fname}\n"));
            }
            FT_SPEC => {
                if ff.statp.mode & crate::stmode::S_IFMT == crate::stmode::S_IFSOCK {
                    self.jmsg(M_SKIPPED, format!("     Socket file skipped: {fname}\n"));
                    return 1;
                }
            }
            FT_NOACCESS => {
                self.jmsg(M_NOTSAVED, format!("     Could not access \"{fname}\": ERR={}\n", errno_text(ff.ff_errno)));
                self.sd.msgs.job_errors += 1;
                return 1;
            }
            FT_NOFOLLOW => {
                self.jmsg(M_NOTSAVED, format!("     Could not follow link \"{fname}\": ERR={}\n", errno_text(ff.ff_errno)));
                self.sd.msgs.job_errors += 1;
                return 1;
            }
            FT_NOSTAT => {
                self.jmsg(M_NOTSAVED, format!("     Could not stat \"{fname}\": ERR={}\n", errno_text(ff.ff_errno)));
                self.sd.msgs.job_errors += 1;
                return 1;
            }
            FT_DIRNOCHG | FT_NOCHG => {
                self.jmsg(M_SKIPPED, format!("     Unchanged file skipped: {fname}\n"));
                return 1;
            }
            FT_ISARCH => {
                self.jmsg(M_NOTSAVED, format!("     Archive file not saved: {fname}\n"));
                return 1;
            }
            FT_NOOPEN => {
                self.jmsg(M_NOTSAVED, format!("     Could not open directory \"{fname}\": ERR={}\n", errno_text(ff.ff_errno)));
                self.sd.msgs.job_errors += 1;
                return 1;
            }
            t => {
                self.jmsg(M_NOTSAVED, format!("     Unknown file type {t}; not saved: {fname}\n"));
                self.sd.msgs.job_errors += 1;
                return 1;
            }
        }
        let mut digest = if has_file_data { Digest::for_flags(ff.opts.flags) } else { None };

        let Some(data_stream) = self.encode_and_send_attributes(ff) else { return 0 };
        if is_ft_object(ff.type_) || ff.type_ == FT_DELETED {
            return self.good_rtn();
        }

        let fmt = ff.statp.mode & crate::stmode::S_IFMT;
        let win32 = cfg!(windows) && is_win32_stream(data_stream);
        let do_read = if win32 && matches!(ff.type_, FT_REG | FT_REGE | FT_DIREND) {
            // an empty file can still have a security descriptor and streams;
            // a directory has its security descriptor (and no data)
            true
        } else if ff.type_ != FT_LNKSAVED && fmt == crate::stmode::S_IFREG {
            ff.statp.size > 0
        } else {
            matches!(ff.type_, FT_RAW | FT_FIFO | FT_REPARSE | FT_JUNCTION)
        };
        if do_read {
            match open_for_backup(ff, data_stream) {
                Ok(mut file) => {
                    let ok = self.send_data(data_stream, ff, &mut *file, digest.as_mut().map(|(d, _)| d));
                    if ff.opts.flags.has(FO_CHKCHANGES) {
                        self.has_file_changed(ff);
                    }
                    if !ok {
                        return self.bail_out();
                    }
                }
                Err(e) => {
                    ff.ff_errno = e.raw_os_error().unwrap_or(0);
                    self.jmsg(M_NOTSAVED, format!("     Cannot open \"{fname}\": ERR={}.\n", crate::berrno::s(&e)));
                    self.sd.msgs.job_errors += 1;
                    return self.good_rtn();
                }
            }
        }

        // ACLs for anything but symlinks, extended attributes for all files
        if ff.opts.flags.has(FO_ACL) && ff.type_ != FT_LNK {
            let (streams, errors) = acl_xattr::build_acl_streams(&ff.fname, ff.type_ == FT_DIREND);
            for e in errors {
                self.jmsg(M_ERROR, e);
                self.acl_errors += 1;
            }
            for (stream, content) in streams {
                if !(self.sd.header(stream) && self.sd.send(&content)) {
                    return self.bail_out();
                }
                self.sd.counters.job_bytes += content.len() as u64;
                if !self.sd.eod() {
                    return self.bail_out();
                }
            }
        }
        if ff.opts.flags.has(FO_XATTR) && !self.xattr_disabled {
            match acl_xattr::build_xattr_stream(&ff.fname, ff.opts.flags.has(FO_ACL)) {
                Ok(Some(content)) => {
                    if !(self.sd.header(acl_xattr::XATTR_STREAM) && self.sd.send(&content)) {
                        return self.bail_out();
                    }
                    self.sd.counters.job_bytes += content.len() as u64;
                    if !self.sd.eod() {
                        return self.bail_out();
                    }
                }
                Ok(None) => {}
                Err((true, m)) => {
                    self.xattr_disabled = true;
                    self.jmsg(M_INFO, m);
                }
                Err((false, m)) => {
                    self.jmsg(M_ERROR, m);
                    self.xattr_errors += 1;
                }
            }
        }

        // TerminateDigest()
        if let Some((d, stream)) = digest.take() {
            let bytes = d.finalize();
            if !(self.sd.header(stream) && self.sd.send(&bytes)) {
                return self.bail_out();
            }
            if let Some(l) = ff.linked.and_then(|k| links.get_mut(&k)) {
                l.digest = Some((stream, bytes.clone()));
            }
            if !self.sd.eod() {
                return self.bail_out();
            }
        }
        // The digest of the original file of a hard link
        if ff.type_ == FT_LNKSAVED {
            if let Some((stream, bytes)) = ff.digest.clone() {
                if !(self.sd.header(stream) && self.sd.send(&bytes) && self.sd.eod()) {
                    return self.bail_out();
                }
            }
        }
        self.good_rtn()
    }

    fn good_rtn(&self) -> i32 {
        if self.env.canceled.load(Ordering::Relaxed) || self.sd.msgs.is_canceled() { 0 } else { 1 }
    }

    fn bail_out(&self) -> i32 {
        0
    }

    /// HasFileChanged()
    fn has_file_changed(&mut self, ff: &FfPkt) {
        if ff.type_ != FT_REG {
            return;
        }
        let fname = String::from_utf8_lossy(&ff.fname).into_owned();
        match stat_of(&ff.fname) {
            Err(e) => self.jmsg(M_WARNING, format!("Cannot stat file {fname}: ERR={}\n", crate::berrno::BErr(e))),
            Ok(st) => {
                if st.mtime != ff.statp.mtime {
                    self.jmsg(M_ERROR, format!("{fname}: mtime changed during backup.\n"));
                } else if st.ctime != ff.statp.ctime {
                    self.jmsg(M_ERROR, format!("{fname}: ctime changed during backup.\n"));
                } else if st.size != ff.statp.size || st.blksize != ff.statp.blksize || st.blocks != ff.statp.blocks {
                    self.jmsg(M_ERROR, format!("{fname}: size changed during backup.\n"));
                }
            }
        }
    }

    /// One data message to the SD (SendDataToSd after the sparse check):
    /// optional file address, data or comp_stream_header + compressed data.
    fn send_block(&mut self, ff: &FfPkt, header: Option<u64>, data: &[u8]) -> bool {
        let mut msg = Vec::with_capacity(data.len() + 32);
        if let Some(h) = header {
            msg.extend_from_slice(&h.to_be_bytes());
        }
        if ff.opts.flags.has(FO_COMPRESS) {
            match compression::compress_data(ff.opts.compress_algo, ff.opts.compress_level, data) {
                Ok(c) => msg.extend_from_slice(&compression::with_header(ff.opts.compress_algo, ff.opts.compress_level, &c)),
                Err(e) => {
                    self.jmsg(M_FATAL, e);
                    self.sd.msgs.set_status(JS_ERROR_TERMINATED);
                    return false;
                }
            }
        } else {
            msg.extend_from_slice(data);
        }
        if !self.sd.send(&msg) {
            return false;
        }
        self.sd.counters.job_bytes += msg.len() as u64;
        true
    }

    /// send_data(): the data stream of an open file.
    fn send_data(&mut self, stream: i32, ff: &FfPkt, file: &mut dyn Read, mut digest: Option<&mut Digest>) -> bool {
        if !self.sd.header(stream) {
            return false;
        }
        let sparse = ff.opts.flags.has(FO_SPARSE);
        let rsize = if sparse || ff.opts.flags.has(FO_OFFSETS) { self.buf_size - 8 } else { self.buf_size };
        let size = ff.statp.size;
        let unsized_file = (ff.type_ == FT_RAW || ff.type_ == FT_FIFO) && size == 0;
        // SendPlainData(): the pipelined variant for files of at least two
        // buffers (MaximumWorkersPerJob > 0), else SendPlainDataSerially().
        let pipelined = self.workers > 0 && (size as u64) >= 2 * rsize as u64;
        let mut buf = vec![0u8; rsize];
        let mut file_addr: u64 = 0;
        let mut bytes_read: u64 = 0;
        let mut read_error: Option<std::io::Error> = None;
        loop {
            if self.env.canceled.load(Ordering::Relaxed) {
                return false;
            }
            let n = match read_full(file, &mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) => {
                    read_error = Some(e);
                    break;
                }
            };
            let data = &buf[..n];
            let header;
            if pipelined {
                let skip = sparse
                    && ((n == rsize && (n as u64 + rsize as u64) < size as u64) || unsized_file)
                    && is_buf_zero(data);
                header = (sparse && !skip).then_some(bytes_read);
                bytes_read += n as u64;
                if skip {
                    continue;
                }
            } else if sparse {
                let zero = ((n == rsize && file_addr + (n as u64) < size as u64) || unsized_file) && is_buf_zero(data);
                header = (!zero).then_some(file_addr);
                file_addr += n as u64;
                if zero {
                    continue;
                }
                self.sd.counters.read_bytes += n as u64;
            } else {
                header = None;
                self.sd.counters.read_bytes += n as u64;
            }
            if let Some(d) = digest.as_deref_mut() {
                d.update(data);
            }
            if !self.send_block(ff, header, data) {
                return false;
            }
            self.sd.publish(None);
        }
        if pipelined {
            self.sd.counters.read_bytes += bytes_read;
        }
        if let Some(e) = read_error {
            let fname = String::from_utf8_lossy(&ff.fname).into_owned();
            self.jmsg(M_ERROR, format!("Read error on file {fname}. ERR={}\n", crate::berrno::s(&e)));
            let errs = self.sd.msgs.job_errors;
            self.sd.msgs.job_errors += 1;
            if errs > 1000 {
                self.jmsg(M_FATAL, format!("Too many errors. JobErrors={}.\n", self.sd.msgs.job_errors));
            }
        }
        self.sd.eod()
    }

    /// A `Plugin =` line: start the signed plugin and save every file it
    /// lists, each framed by plugin-name records like the Bareos FD.
    /// `covered`: the `File =` names of the FileSet, which a plugin's broker
    /// may check its instance's files against (warnings only).
    pub fn save_plugin_covered(&mut self, include: &FfPkt, text: &str, covered: &[Vec<u8>]) -> i32 {
        let shown = crate::secrets::mask_plugin_secrets(text);
        let line = match plugin::parse_line(text) {
            Ok(l) => l,
            Err(e) => {
                self.jmsg(M_FATAL, format!("Plugin: \"{shown}\": {e}\n"));
                return 0;
            }
        };
        let job = pb::JobContext {
            job_id: u64::from(self.sd.msgs.job_id),
            job_name: if self.sd.msgs.job.is_empty() { "backup".into() } else { self.sd.msgs.job.clone() },
            operation: pb::JobOperation::Backup as i32,
        };
        // The send buffer must take a whole block (send_data reads buf_size).
        let chunk = self.buf_size.clamp(4096, 256 * 1024) as u32;
        let index = self.plugin_index;
        self.plugin_index += 1;
        match Session::open_covered(self.config, &line, job, index, chunk, covered) {
            Ok(session) => {
                self.jmsg(M_INFO, format!("Plugin: \"{shown}\": {}.\n", session.loaded));
                for w in &session.warnings {
                    self.jmsg(M_WARNING, format!("Plugin: \"{shown}\": {w}\n"));
                }
                self.run_plugin(include, &shown, session)
            }
            Err(e) => {
                self.jmsg(M_FATAL, format!("Plugin: \"{shown}\": {e}\n"));
                0
            }
        }
    }

    /// The files of an opened plugin session into the SD stream.
    pub(crate) fn run_plugin(&mut self, include: &FfPkt, shown: &str, mut session: Session) -> i32 {
        let flag: &AtomicBool = self.env.canceled;
        let canceled = move || flag.load(Ordering::Relaxed);
        let mut ok = match session.start(&canceled) {
            Ok(()) => true,
            Err(e) => {
                self.jmsg(M_FATAL, format!("Plugin: \"{shown}\": start: {e}\n"));
                false
            }
        };
        while ok {
            match session.next_file(&canceled) {
                Ok(None) => break,
                Ok(Some(entry)) => ok = self.save_plugin_file(&session, include, shown, entry, &canceled),
                Err(PluginError::Canceled) => ok = false,
                Err(e) => {
                    self.jmsg(M_FATAL, format!("Plugin: \"{shown}\": {e}\n"));
                    ok = false;
                }
            }
        }
        let plugin_id = session.plugin_id.clone();
        if let Err(e) = session.finish(ok) {
            if ok {
                self.jmsg(M_FATAL, format!("Plugin: \"{shown}\": finish: {e}\n"));
                ok = false;
            }
        }
        self.jmsg(M_INFO, format!("Plugin: \"{shown}\": {plugin_id} finished, {}.\n",
            if ok { "job part succeeded" } else { "job part failed" }));
        i32::from(ok)
    }

    /// One file of a plugin: plugin name, attributes, data, digest, end.
    fn save_plugin_file(&mut self, session: &Session, include: &FfPkt, shown: &str, entry: pb::FileEntry,
        canceled: &dyn Fn() -> bool) -> bool {
        if !entry.source_ref.is_empty() {
            return self.save_scan_file(session, include, shown, &entry);
        }
        let Some(mut ff) = plugin_ff(include, &entry) else {
            let path = String::from_utf8_lossy(&entry.path).into_owned();
            self.jmsg(M_FATAL, format!("Plugin: \"{shown}\": unsupported entry \"{path}\"\n"));
            return false;
        };
        let fname = String::from_utf8_lossy(&ff.fname).into_owned();
        self.sd.counters.num_files_examined += 1;
        self.sd.publish(Some(&fname));
        if !self.send_plugin_name(shown, true) {
            return false;
        }
        let regular = ff.type_ == FT_REG;
        let mut digest = if regular { Digest::for_flags(ff.opts.flags) } else { None };
        let Some(stream) = self.encode_and_send_attributes(&mut ff) else { return false };
        if regular {
            let mut reader = PluginReader { session, path: entry.path.clone(), offset: 0, eof: false, canceled, error: None };
            let sent = self.send_data(stream, &ff, &mut reader, digest.as_mut().map(|(d, _)| d));
            if let Some(e) = reader.error {
                if e != PluginError::Canceled {
                    self.jmsg(M_FATAL, format!("Plugin: \"{shown}\": reading \"{fname}\": {e}\n"));
                }
                return false;
            }
            if !sent {
                return false;
            }
        }
        if let Some((d, stream)) = digest.take() {
            let bytes = d.finalize();
            if !(self.sd.header(stream) && self.sd.send(&bytes) && self.sd.eod()) {
                return false;
            }
        }
        self.send_plugin_name(shown, false)
    }

    /// One host scan entry (docs/plans/PODMAN-PLUGIN.md, "Erweiterung von
    /// FileEntry"): the guest chose only whether and when; stat, ACL,
    /// xattrs and bytes come from the host's own scan table. ReadFile is
    /// never called, and the guest's size, mode and time are ignored. The
    /// stream carries host IDs, mode & 0o777, only `user.*` xattrs and no
    /// devices (V-1, S-13).
    #[cfg(unix)]
    fn save_scan_file(&mut self, session: &Session, include: &FfPkt, shown: &str, entry: &pb::FileEntry) -> bool {
        use crate::podman_scan::EntryKind;
        let taken = match session.take_scan(entry) {
            Ok(t) => t,
            Err(e) => {
                let path = String::from_utf8_lossy(&entry.path).into_owned();
                self.jmsg(M_FATAL, format!("Plugin: \"{shown}\": \"{path}\": {e}\n"));
                return false;
            }
        };
        let e = &taken.entry;
        let mut opts = include.opts.clone();
        // ACL and xattrs come from the scan, not from a path lookup.
        for bit in [FO_SPARSE, FO_OFFSETS, FO_ACL, FO_XATTR, FO_CHKCHANGES] {
            opts.flags.clear(bit);
        }
        let path = e.path.clone();
        let (type_, link, link_fi) = match &e.kind {
            EntryKind::Regular | EntryKind::Ownership(_) => (FT_REG, path.clone(), 0),
            EntryKind::Directory => {
                let mut d = path.clone();
                d.push(b'/');
                (FT_DIREND, d, 0)
            }
            EntryKind::Symlink(target) => (FT_LNK, target.clone(), 0),
            EntryKind::Fifo => (FT_SPEC, path.clone(), 0),
            EntryKind::Hardlink(_) => {
                let target = taken.link_target.clone().unwrap_or_default();
                let fi = session.saved_index(&target).unwrap_or(0);
                (FT_LNKSAVED, target, fi)
            }
        };
        let mut ff = FfPkt { fname: path.clone(), link, top_fname: path.clone(), statp: e.stat.clone(), type_, link_fi, opts,
            ..Default::default() };
        let fname = String::from_utf8_lossy(&ff.fname).into_owned();
        self.sd.counters.num_files_examined += 1;
        self.sd.publish(Some(&fname));
        if !self.send_plugin_name(shown, true) {
            return false;
        }
        let regular = ff.type_ == FT_REG;
        let mut digest = if regular { Digest::for_flags(ff.opts.flags) } else { None };
        let Some(stream) = self.encode_and_send_attributes(&mut ff) else { return false };
        if regular {
            let sent = match (&e.kind, taken.file) {
                (EntryKind::Ownership(bytes), _) => {
                    self.send_data(stream, &ff, &mut std::io::Cursor::new(bytes.clone()), digest.as_mut().map(|(d, _)| d))
                }
                (_, Some(mut file)) => {
                    let ok = self.send_data(stream, &ff, &mut file, digest.as_mut().map(|(d, _)| d));
                    // consistency=crash: a file that changed while read is a warning.
                    if let Ok(m) = file.metadata() {
                        use std::os::unix::fs::MetadataExt;
                        if m.size() as i64 != e.stat.size || m.mtime() != e.stat.mtime {
                            self.jmsg(M_WARNING, format!("Plugin: \"{shown}\": \"{fname}\" changed during backup\n"));
                        }
                    }
                    ok
                }
                (_, None) => false,
            };
            if !sent {
                return false;
            }
        }
        let mut extra = Vec::new();
        if !e.acl_access.is_empty() {
            extra.push((STREAM_ACL_LINUX_ACCESS_ACL, crate::podman_scan::acl_stream_text(&e.acl_access)));
        }
        if !e.acl_default.is_empty() && type_ == FT_DIREND {
            extra.push((STREAM_ACL_LINUX_DEFAULT_ACL, crate::podman_scan::acl_stream_text(&e.acl_default)));
        }
        for (stream, text) in extra {
            let mut v = text.into_bytes();
            v.push(0);
            if !(self.sd.header(stream) && self.sd.send(&v) && self.sd.eod()) {
                return false;
            }
        }
        if !e.xattrs.is_empty() {
            let content = acl_xattr::serialize_xattrs(&e.xattrs);
            if !(self.sd.header(STREAM_XATTR_LINUX) && self.sd.send(&content) && self.sd.eod()) {
                return false;
            }
        }
        if let Some((d, stream)) = digest.take() {
            let bytes = d.finalize();
            if !(self.sd.header(stream) && self.sd.send(&bytes) && self.sd.eod()) {
                return false;
            }
        }
        session.mark_saved(&path, ff.file_index);
        self.send_plugin_name(shown, false)
    }

    /// Host scan entries come from the Podman plugin, which is Linux-only.
    #[cfg(windows)]
    fn save_scan_file(&mut self, _session: &Session, _include: &FfPkt, shown: &str, _entry: &pb::FileEntry) -> bool {
        self.jmsg(M_FATAL, format!("Plugin: \"{shown}\": host scan entries are not supported on this platform\n"));
        false
    }

    /// send_plugin_name(): "<index> 1 1 <line>\0" before a plugin file,
    /// "<index> 0" after it. The line is stored masked: no secret on a volume.
    fn send_plugin_name(&mut self, shown: &str, start: bool) -> bool {
        let index = self.sd.counters.job_files + u32::from(start);
        let header = format!("{index} {STREAM_PLUGIN_NAME} 0");
        let mut data = if start { format!("{index} 1 1 {shown}").into_bytes() } else { format!("{index} 0").into_bytes() };
        if start {
            data.push(0);
        }
        self.sd.send(header.as_bytes()) && self.sd.send(&data) && self.sd.eod()
    }

    /// AccurateFinish(): the deleted files (Incremental/Differential) or
    /// the base files (Full) to the SD.
    pub fn accurate_finish(&mut self) -> bool {
        if self.env.canceled.load(Ordering::Relaxed) {
            return true;
        }
        let Some(list) = self.env.accurate.take() else { return true };
        if self.env.level_full {
            if !self.env.rerunning {
                for (name, p) in list.files_seen(true) {
                    let (st, _, _) = decode_stat(&p.lstat);
                    let mut ff = FfPkt { fname: name.clone(), link: name, type_: FT_BASE, statp: st, ..Default::default() };
                    if self.encode_and_send_attributes(&mut ff).is_none() {
                        return false;
                    }
                }
            }
            let mb = self.env.base_size / (1024 * 1024);
            self.jmsg(M_INFO, format!("Space saved with Base jobs: {mb} MB\n"));
        } else {
            for (name, p) in list.files_seen(false) {
                let (st, _, _) = decode_stat(&p.lstat);
                let statp = Stat { mtime: st.mtime, ctime: st.ctime, ..Default::default() };
                let mut ff = FfPkt { fname: name.clone(), link: name, type_: FT_DELETED, statp, ..Default::default() };
                if self.encode_and_send_attributes(&mut ff).is_none() {
                    return false;
                }
            }
        }
        true
    }
}

/// bread() of a regular file: read() until the buffer is full or EOF.
fn read_full(f: &mut (impl Read + ?Sized), buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match f.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                if n > 0 {
                    return Ok(n);
                }
                return Err(e);
            }
        }
    }
    Ok(n)
}

/// The FindFiles() handler of a backup.
pub struct BackupHandler<'x, 'a, 'b, S: Read + Write> {
    pub b: &'x mut Backup<'a, 'b, S>,
    pub fileset: &'x FileSet,
}

/// A plugin path as stored: absolute, without `.`, `..` or empty parts, so a
/// restore to `Where` stays below `Where`. None when it cannot be made so.
pub fn plugin_path(raw: &[u8]) -> Option<Vec<u8>> {
    let inner = raw.strip_prefix(b"/").unwrap_or(raw);
    let inner = inner.strip_suffix(b"/").unwrap_or(inner);
    if inner.is_empty() || inner.split(|&b| b == b'/').any(|c| c.is_empty() || c == b"." || c == b"..") {
        return None;
    }
    let mut out = Vec::with_capacity(inner.len() + 1);
    out.push(b'/');
    out.extend_from_slice(inner);
    Some(out)
}

/// The FfPkt of a plugin entry: its virtual path, type and attributes, with
/// the include's options minus those that need a real file (sparse,
/// offsets, ACLs, xattrs, change check). None for an unsupported kind.
pub fn plugin_ff(include: &FfPkt, entry: &pb::FileEntry) -> Option<FfPkt> {
    let mut opts = include.opts.clone();
    for bit in [FO_SPARSE, FO_OFFSETS, FO_ACL, FO_XATTR, FO_CHKCHANGES] {
        opts.flags.clear(bit);
    }
    let secs = entry.modified_unix_nanos.div_euclid(1_000_000_000);
    // SAFETY: geteuid/getegid have no preconditions and cannot fail.
    #[cfg(unix)]
    let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
    // Windows has no POSIX owner (Bareos Win32 stores 0 as uid/gid).
    #[cfg(windows)]
    let (uid, gid) = (0, 0);
    // Only permission bits from the guest: setuid, setgid and sticky would
    // come back as root-owned special files on a restore to files (S-13).
    let perm = entry.mode & 0o777;
    let mut statp = Stat { nlink: 1, uid, gid, atime: secs, mtime: secs, ctime: secs, blksize: 4096, ..Default::default() };
    let path = plugin_path(&entry.path)?;
    let (type_, link) = match pb::FileKind::try_from(entry.kind).ok()? {
        pb::FileKind::Regular | pb::FileKind::Virtual => {
            statp.mode = crate::stmode::S_IFREG | if perm == 0 { 0o600 } else { perm };
            statp.size = i64::try_from(entry.size).ok()?;
            (FT_REG, path.clone())
        }
        pb::FileKind::Directory => {
            statp.mode = crate::stmode::S_IFDIR | if perm == 0 { 0o700 } else { perm };
            let mut dir = path.clone();
            if dir.last() != Some(&b'/') {
                dir.push(b'/');
            }
            (FT_DIREND, dir)
        }
        pb::FileKind::Symlink => {
            statp.mode = crate::stmode::S_IFLNK | 0o777;
            (FT_LNK, entry.symlink_target.clone())
        }
        // FIFO and hard link exist only as host scan entries (source_ref,
        // see podman_scan); a guest cannot describe one itself.
        pb::FileKind::Fifo | pb::FileKind::Hardlink | pb::FileKind::Unspecified => return None,
    };
    Some(FfPkt { fname: path.clone(), link, top_fname: path, statp, type_, opts, ..Default::default() })
}

impl<S: Read + Write> Handler for BackupHandler<'_, '_, '_, S> {
    fn plugin(&mut self, ff: &mut FfPkt, line: &str) -> i32 {
        let covered: Vec<Vec<u8>> = self.fileset.include.iter().flat_map(|i| i.names.iter().cloned()).collect();
        self.b.save_plugin_covered(ff, line, &covered)
    }
    fn handle(&mut self, ff: &mut FfPkt, links: &mut HashMap<(u64, u64), CurLink>, _top: bool) -> i32 {
        if ff.type_ == FT_NOFSCHG && !is_in_fileset(self.fileset, &ff.fname) {
            let fname = String::from_utf8_lossy(&ff.fname).into_owned();
            let top = String::from_utf8_lossy(&ff.top_fname).into_owned();
            self.b.jmsg(M_INFO, format!("     {fname} is a different filesystem. Will not descend from {top} into it.\n"));
        }
        self.b.save_file(ff, links)
    }
    fn check_changes(&mut self, ff: &mut FfPkt) -> Option<bool> {
        if self.b.env.accurate.is_none() && !self.b.env.rerunning {
            return None;
        }
        let env = &mut self.b.env;
        Some(accurate_check_file(ff, env, self.b.sd.counters, self.b.sd.msgs))
    }
    fn canceled(&self) -> bool {
        self.b.env.canceled.load(Ordering::Relaxed)
    }
    fn jmsg(&mut self, t: i32, text: String) {
        self.b.jmsg(t, text);
    }
}

/// TallyFile() of an estimate.
pub struct EstimateHandler<'x, 'b> {
    pub counters: &'x mut Counters,
    pub msgs: &'x mut JobMsgs,
    pub env: JobEnv<'b>,
    pub listing: bool,
}

impl Handler for EstimateHandler<'_, '_> {
    /// Estimating would run the plugin; it is counted as zero files.
    fn plugin(&mut self, _ff: &mut FfPkt, line: &str) -> i32 {
        let shown = crate::secrets::mask_plugin_secrets(line);
        self.msgs.jmsg(M_INFO, &format!("Plugin: \"{shown}\": not included in the estimate.\n"));
        1
    }
    fn handle(&mut self, ff: &mut FfPkt, _l: &mut HashMap<(u64, u64), CurLink>, _top: bool) -> i32 {
        if self.env.canceled.load(Ordering::Relaxed) {
            return 0;
        }
        match ff.type_ {
            FT_LNKSAVED | FT_REGE | FT_REG | FT_LNK | FT_NORECURSE | FT_NOFSCHG | FT_INVALIDFS | FT_INVALIDDT
            | FT_REPARSE | FT_JUNCTION | FT_DIREND | FT_SPEC | FT_RAW | FT_FIFO => {}
            _ => return 1,
        }
        if ff.type_ != FT_LNKSAVED && ff.statp.mode & crate::stmode::S_IFMT == crate::stmode::S_IFREG && ff.statp.size > 0 {
            self.counters.job_bytes += ff.statp.size as u64;
        }
        self.counters.num_files_examined += 1;
        self.counters.job_files += 1;
        if self.listing {
            let attr = Attributes {
                type_: ff.type_,
                statp: ff.statp.clone(),
                ofname: ff.fname.clone(),
                olname: ff.link.clone(),
                ..Default::default()
            };
            self.msgs.jmsg(M_RESTORED, &ls_output(&attr));
        }
        1
    }
    fn check_changes(&mut self, ff: &mut FfPkt) -> Option<bool> {
        self.env.accurate.as_ref()?;
        Some(accurate_check_file(ff, &mut self.env, self.counters, self.msgs))
    }
    fn canceled(&self) -> bool {
        self.env.canceled.load(Ordering::Relaxed)
    }
    fn jmsg(&mut self, t: i32, text: String) {
        self.msgs.jmsg(t, &text);
    }
}

/// VerifyFile(): the attributes (and digests) of the disk files to the
/// Director.
pub struct VerifyHandler<'x, D: Read + Write> {
    pub dir: &'x mut BnetStream<D>,
    pub counters: &'x mut Counters,
    pub msgs: &'x mut JobMsgs,
    pub canceled: &'x AtomicBool,
}

impl<D: Read + Write> Handler for VerifyHandler<'_, D> {
    fn handle(&mut self, ff: &mut FfPkt, _l: &mut HashMap<(u64, u64), CurLink>, _top: bool) -> i32 {
        if self.canceled.load(Ordering::Relaxed) {
            return 0;
        }
        self.msgs.flush(self.dir);
        self.counters.num_files_examined += 1;
        let fname = String::from_utf8_lossy(&ff.fname).into_owned();
        let errno_text = |e: i32| crate::berrno::BErr(e).to_string();
        match ff.type_ {
            FT_LNKSAVED | FT_REGE | FT_REG | FT_LNK | FT_REPARSE | FT_JUNCTION | FT_DIREND | FT_SPEC | FT_RAW | FT_FIFO => {}
            FT_DIRBEGIN => {
                self.counters.num_files_examined -= 1;
                return 1;
            }
            FT_NOACCESS => {
                self.msgs.jmsg(M_NOTSAVED, &format!("     Could not access {fname}: ERR={}\n", errno_text(ff.ff_errno)));
                self.msgs.job_errors += 1;
                return 1;
            }
            FT_NOFOLLOW => {
                self.msgs.jmsg(M_NOTSAVED, &format!("     Could not follow link {fname}: ERR={}\n", errno_text(ff.ff_errno)));
                self.msgs.job_errors += 1;
                return 1;
            }
            FT_NOSTAT => {
                self.msgs.jmsg(M_NOTSAVED, &format!("     Could not stat {fname}: ERR={}\n", errno_text(ff.ff_errno)));
                self.msgs.job_errors += 1;
                return 1;
            }
            FT_DIRNOCHG | FT_NOCHG => {
                self.msgs.jmsg(M_SKIPPED, &format!("     Unchanged file skipped: {fname}\n"));
                return 1;
            }
            FT_ISARCH => {
                self.msgs.jmsg(M_SKIPPED, &format!("     Archive file skipped: {fname}\n"));
                return 1;
            }
            FT_NORECURSE => {
                self.msgs.jmsg(M_SKIPPED, &format!("     Recursion turned off. Directory skipped: {fname}\n"));
                ff.type_ = FT_DIREND;
            }
            FT_NOFSCHG => {
                self.msgs.jmsg(M_SKIPPED, &format!("     File system change prohibited. Directory skipped: {fname}\n"));
                return 1;
            }
            FT_PLUGIN_CONFIG | FT_RESTORE_FIRST => return 1,
            FT_NOOPEN => {
                self.msgs.jmsg(M_NOTSAVED, &format!("     Could not open directory {fname}: ERR={}\n", errno_text(ff.ff_errno)));
                self.msgs.job_errors += 1;
                return 1;
            }
            t => {
                self.msgs.jmsg(M_NOTSAVED, &format!("     Unknown file type {t}: {fname}\n"));
                self.msgs.job_errors += 1;
                return 1;
            }
        }
        let attribs = encode_stat(&ff.statp, ff.link_fi, 0);
        self.counters.job_files += 1;
        let (sf, sl) = stripped_names(ff);
        let fi = self.counters.job_files;
        let mut msg = format!("{fi} {STREAM_UNIX_ATTRIBUTES} {} ", ff.verify_opts).into_bytes();
        match ff.type_ {
            FT_LNK | FT_LNKSAVED => {
                msg.extend_from_slice(&sf);
                msg.push(0);
                msg.extend_from_slice(attribs.as_bytes());
                msg.push(0);
                msg.extend_from_slice(&sl);
                msg.push(0);
            }
            FT_DIREND | FT_REPARSE | FT_JUNCTION => {
                msg.extend_from_slice(&sl);
                msg.push(0);
                msg.extend_from_slice(attribs.as_bytes());
                msg.push(0);
                msg.push(0);
            }
            _ => {
                msg.extend_from_slice(&sf);
                msg.push(0);
                msg.extend_from_slice(attribs.as_bytes());
                msg.push(0);
                msg.push(0);
            }
        }
        if self.dir.send(&msg).is_err() {
            self.msgs.jmsg(M_FATAL, "Network error in send to Director: ERR=Broken pipe\n");
            return 0;
        }
        if ff.type_ != FT_LNKSAVED && ff.statp.mode & crate::stmode::S_IFMT == crate::stmode::S_IFREG && ff.opts.flags.any_digest() {
            match file_checksum(ff, self.counters, self.msgs, true) {
                Some(Some((name, stream, b64))) => {
                    let _ = self.dir.send_str(&format!("{fi} {stream} {b64} *{name}-{fi}*"));
                }
                Some(None) => self.msgs.job_errors += 1,
                None => {}
            }
        }
        1
    }
    fn canceled(&self) -> bool {
        self.canceled.load(Ordering::Relaxed)
    }
    fn jmsg(&mut self, t: i32, text: String) {
        self.msgs.jmsg(t, &text);
    }
}

#[cfg(test)]
mod tests {

    /// S-13: a plugin cannot create setuid/setgid/sticky files.
    #[test]
    fn plugin_entry_mode_keeps_only_permission_bits() {
        let include = FfPkt::default();
        let entry = |mode| pb::FileEntry { path: b"/@t/x".to_vec(), kind: pb::FileKind::Regular as i32, size: 1, mode, ..Default::default() };
        let ff = plugin_ff(&include, &entry(0o4755)).unwrap();
        assert_eq!(ff.statp.mode, crate::stmode::S_IFREG | 0o755);
        let ff = plugin_ff(&include, &entry(0o7000)).unwrap();
        assert_eq!(ff.statp.mode, crate::stmode::S_IFREG | 0o600, "only special bits: default permissions");
        let dir = pb::FileEntry { kind: pb::FileKind::Directory as i32, ..entry(0o3775) };
        assert_eq!(plugin_ff(&include, &dir).unwrap().statp.mode, crate::stmode::S_IFDIR | 0o775);
    }
    use super::*;

    /// BNET frames written to the SD: data records and signals.
    #[derive(Debug, PartialEq)]
    enum Frame {
        Data(Vec<u8>),
        Signal(i32),
    }
    fn frames(mut b: &[u8]) -> Vec<Frame> {
        let mut out = Vec::new();
        while b.len() >= 4 {
            let n = i32::from_be_bytes(b[..4].try_into().unwrap());
            b = &b[4..];
            if n < 0 {
                out.push(Frame::Signal(n));
            } else {
                out.push(Frame::Data(b[..n as usize].to_vec()));
                b = &b[n as usize..];
            }
        }
        out
    }
    fn text(f: &Frame) -> String {
        match f {
            Frame::Data(d) => String::from_utf8_lossy(d).into_owned(),
            Frame::Signal(s) => format!("<{s}>"),
        }
    }

    #[test]
    fn plugin_files_are_framed_like_the_bareos_fd() {
        use crate::plugin::testkit::*;
        let d = std::env::temp_dir().join(format!("ofd-backup-plugin-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let script = Script {
            next: vec![file_step(b"@test/main/a.dump", 5), file_step(b"@test/main/empty", 0), complete_step()],
            read: vec![data_step(b"hel", false), data_step(b"lo", true), data_step(b"", true)],
            ..Default::default()
        };
        let resolved = signed_plugin(&d, "test", &script);
        let line = crate::plugin::parse_line("test:instance=main:password=geheim").unwrap();
        let job = pb::JobContext { job_id: 5, job_name: "j".into(), operation: pb::JobOperation::Backup as i32 };
        let runtime = onesimus_plugins::wasm::RuntimeLimits { max_chunk_bytes: 4096, ..Default::default() };
        let session = Session::open_resolved(&resolved, &line, job, 0, runtime).unwrap();

        let config = FdConfig::parse("Director { Name = d; Password = p }").unwrap();
        let canceled = AtomicBool::new(false);
        let mut sd = BnetStream::new(std::io::Cursor::new(Vec::new()));
        let mut msgs = JobMsgs::new("test-fd");
        let mut counters = Counters::default();
        let mut flush = |_: &mut JobMsgs| {};
        let shown = crate::secrets::mask_plugin_secrets("test:instance=main:password=geheim");
        let ok = {
            let mut b = Backup {
                sd: SdSender { sd: &mut sd, msgs: &mut msgs, counters: &mut counters, canceled: &canceled, progress: None },
                env: JobEnv { canceled: &canceled, accurate: None, level_full: true, rerunning: false, base_size: 0 },
                flush: &mut flush,
                buf_size: 4096,
                workers: 0,
                acl_errors: 0,
                xattr_errors: 0,
                xattr_disabled: false,
                config: &config,
                plugin_index: 0,
            };
            b.run_plugin(&FfPkt::default(), &shown, session)
        };
        assert_eq!(ok, 1);
        assert_eq!(counters.job_files, 2);
        let f = frames(&sd.into_inner().into_inner());
        let t: Vec<String> = f.iter().map(text).collect();
        let eod = "<-1>".to_string();
        // File 1: plugin name, attributes, data, end.
        assert_eq!(t[0], "1 26 0");
        assert_eq!(t[1], "1 1 1 test:instance=main:password=*****\0");
        assert_eq!(t[2], eod);
        assert_eq!(t[3], "1 1 0");
        assert!(t[4].starts_with("1 3 /@test/main/a.dump\0"), "{}", t[4]);
        assert_eq!(t[5], eod);
        assert_eq!(t[6], "1 2 0");
        assert_eq!(t[7], "hel");
        assert_eq!(t[8], "lo");
        assert_eq!(t[9], eod);
        assert_eq!(t[10..13], ["1 26 0".to_string(), "1 0".to_string(), eod.clone()]);
        // File 2: an empty file still has a (empty) data stream.
        assert_eq!(t[13], "2 26 0");
        assert_eq!(t[16], "2 1 0");
        assert!(t[17].starts_with("2 3 /@test/main/empty\0"), "{}", t[17]);
        assert!(!t.iter().any(|x| x.contains("geheim")), "a secret reached the volume");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn plugin_paths_stay_below_where() {
        assert_eq!(plugin_path(b"@pg/main/a.dump").unwrap(), b"/@pg/main/a.dump");
        assert_eq!(plugin_path(b"/@pg/main/").unwrap(), b"/@pg/main");
        for bad in [&b""[..], b"/", b"a/../b", b"../a", b"a/./b", b"a//b", b"/.."] {
            assert_eq!(plugin_path(bad), None, "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn strip_and_streams() {
        assert_eq!(do_strip(1, b"/a/b/c"), Some(b"/b/c".to_vec()));
        assert_eq!(do_strip(2, b"/a/b/c"), Some(b"/c".to_vec()));
        assert_eq!(do_strip(3, b"/a/b/c"), None);
        let mut f = Flags::default();
        assert_eq!(select_data_stream(f), STREAM_FILE_DATA);
        f.set(FO_SPARSE);
        assert_eq!(select_data_stream(f), STREAM_SPARSE_DATA);
        f.set(FO_COMPRESS);
        assert_eq!(select_data_stream(f), STREAM_SPARSE_COMPRESSED_DATA);
    }
}
