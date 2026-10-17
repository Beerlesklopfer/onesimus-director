//! Backup — FD side: filed/backup.cc (BlastDataToStorageDaemon, SaveFile,
//! send_data/SendDataToSd, EncodeAndSendAttributes, StripPath),
//! findlib/attribs.cc SelectDataStream, accurate.cc (AccurateCheckFile,
//! AccurateFinish), estimate.cc (TallyFile) and verify.cc (VerifyFile).
//!
//! Records to the Storage daemon: a header "<FileIndex> <Stream> 0", the
//! data message(s), BNET_EOD. Encryption/signing (PKI) and plugins are not
//! available in this File Daemon (no keys or plugins can be configured).

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Write};
use std::os::unix::ffi::OsStrExt;
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

pub const M_NOTSAVED: i32 = 8;

/// SelectDataStream() (no encryption, portable data).
pub fn select_data_stream(flags: Flags) -> i32 {
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
}

impl<S: Read + Write> SdSender<'_, S> {
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
        let h = format!("{} {} 0", self.counters.job_files, stream);
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

/// calculate_file_chksum(): (digest name, base64 digest) of the whole file
/// (zero blocks skipped like ReadDigest() when FO_SPARSE).
pub fn file_checksum(ff: &FfPkt, counters: &mut Counters, msgs: &mut JobMsgs, is_verify: bool) -> Option<Option<(&'static str, i32, String)>> {
    let (mut d, stream) = Digest::for_flags(ff.opts.flags)?;
    let name = d.name();
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    if ff.opts.flags.has(FO_NOATIME) {
        opts.custom_flags(libc::O_NOATIME);
    }
    let fname = String::from_utf8_lossy(&ff.fname).into_owned();
    let mut f = match opts.open(std::ffi::OsStr::from_bytes(&ff.fname)) {
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
    let is_dir = ff.statp.mode & libc::S_IFMT == libc::S_IFDIR;
    let key = if is_dir { sl } else { sf };
    let Some(payload) = list.files.get(&key).cloned() else { return true };
    ff.accurate_found = true;
    ff.delta_seq = payload.delta_seq;
    let (cat, _, _) = decode_stat(&payload.lstat);
    let opts = if !env.rerunning && env.level_full { ff.basejob_opts.clone() } else { ff.accurate_opts.clone() };
    let can_checksum = ff.type_ != FT_LNKSAVED && ff.statp.mode & libc::S_IFMT == libc::S_IFREG && ff.opts.flags.any_digest();
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
        // attribsEx (empty), delta_seq
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
                if ff.statp.mode & libc::S_IFMT == libc::S_IFSOCK {
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

        let fmt = ff.statp.mode & libc::S_IFMT;
        let do_read = if ff.type_ != FT_LNKSAVED && fmt == libc::S_IFREG {
            ff.statp.size > 0
        } else {
            matches!(ff.type_, FT_RAW | FT_FIFO | FT_REPARSE | FT_JUNCTION)
        };
        if do_read {
            let mut opts = std::fs::OpenOptions::new();
            opts.read(true);
            if ff.opts.flags.has(FO_NOATIME) {
                opts.custom_flags(libc::O_NOATIME);
            }
            match opts.open(std::ffi::OsStr::from_bytes(&ff.fname)) {
                Ok(mut file) => {
                    let ok = self.send_data(data_stream, ff, &mut file, digest.as_mut().map(|(d, _)| d));
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
                    if !(self.sd.header(STREAM_XATTR_LINUX) && self.sd.send(&content)) {
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
        match lstat(&ff.fname) {
            Err(e) => self.jmsg(M_WARNING, format!("Cannot stat file {fname}: ERR={}\n", crate::berrno::BErr(e))),
            Ok(st) => {
                let st = stat_from(&st);
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
    fn send_data(&mut self, stream: i32, ff: &FfPkt, file: &mut File, mut digest: Option<&mut Digest>) -> bool {
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
fn read_full(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
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

impl<S: Read + Write> Handler for BackupHandler<'_, '_, '_, S> {
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
    fn handle(&mut self, ff: &mut FfPkt, _l: &mut HashMap<(u64, u64), CurLink>, _top: bool) -> i32 {
        if self.env.canceled.load(Ordering::Relaxed) {
            return 0;
        }
        match ff.type_ {
            FT_LNKSAVED | FT_REGE | FT_REG | FT_LNK | FT_NORECURSE | FT_NOFSCHG | FT_INVALIDFS | FT_INVALIDDT
            | FT_REPARSE | FT_JUNCTION | FT_DIREND | FT_SPEC | FT_RAW | FT_FIFO => {}
            _ => return 1,
        }
        if ff.type_ != FT_LNKSAVED && ff.statp.mode & libc::S_IFMT == libc::S_IFREG && ff.statp.size > 0 {
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
        if ff.type_ != FT_LNKSAVED && ff.statp.mode & libc::S_IFMT == libc::S_IFREG && ff.opts.flags.any_digest() {
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
    use super::*;

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
