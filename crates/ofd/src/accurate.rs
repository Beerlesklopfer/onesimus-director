//! Accurate mode: filed/accurate.cc (AccurateCmd, AccurateCheckFile,
//! AccurateFinish) with the hash table file list of accurate_htable.cc.

use std::collections::HashMap;

/// accurate_payload
#[derive(Debug, Clone)]
pub struct Payload {
    pub lstat: Vec<u8>,
    pub chksum: String,
    pub delta_seq: i32,
    pub seen: bool,
}

/// BareosAccurateFilelistHtable
#[derive(Default)]
pub struct FileList {
    /// File names in the order the Director sent them.
    pub order: Vec<Vec<u8>>,
    pub files: HashMap<Vec<u8>, Payload>,
    pub duplicates: usize,
    pub capacity: usize,
}

impl FileList {
    pub fn new(capacity: usize) -> Self {
        Self { capacity, ..Default::default() }
    }

    /// One record of the accurate list: fname\0lstat\0[chksum\0delta_seq].
    pub fn add_record(&mut self, msg: &[u8]) {
        let mut parts = msg.split(|&b| b == 0);
        let fname = parts.next().unwrap_or(&[]).to_vec();
        let lstat = parts.next().unwrap_or(&[]).to_vec();
        let (chksum, delta_seq) = if fname.len() + lstat.len() + 2 >= msg.len() {
            (String::new(), 0)
        } else {
            let c = parts.next().unwrap_or(&[]);
            let d = parts.next().unwrap_or(&[]);
            if fname.len() + lstat.len() + c.len() + 3 > msg.len() {
                return;
            }
            let d: String = String::from_utf8_lossy(d).chars().take_while(|c| c.is_ascii_digit() || *c == '-').collect();
            (String::from_utf8_lossy(c).into_owned(), d.parse().unwrap_or(0))
        };
        if self.files.contains_key(&fname) {
            self.duplicates += 1;
            return;
        }
        self.order.push(fname.clone());
        self.files.insert(fname, Payload { lstat, chksum, delta_seq, seen: false });
    }

    /// EndLoad(): the warnings of accurate_htable.cc.
    pub fn end_load_messages(&self) -> Vec<String> {
        let mut m = Vec::new();
        if self.duplicates > 0 {
            m.push(format!(
                "{} duplicate files were sent by the director and removed. This may indicate problems with the database.\n",
                self.duplicates
            ));
        }
        if self.files.len() > self.capacity {
            m.push(format!(
                "The director send too many files. {} were sent but only {} were anticipated. The accurate job may be in a corrupted state.\n",
                self.files.len(),
                self.capacity
            ));
        }
        m
    }

    /// The not seen files (SendDeletedList) or the seen ones (SendBaseFileList).
    pub fn files_seen(&self, seen: bool) -> Vec<(Vec<u8>, Payload)> {
        self.order.iter().filter_map(|n| self.files.get(n).filter(|p| p.seen == seen).map(|p| (n.clone(), p.clone()))).collect()
    }
}

/// AccurateCheckFile() comparison of the catalog lstat with the file:
/// true when the file changed. `chksum_ok` computes and compares the file
/// checksum (options '5'/'1').
pub fn changed(
    opts: &str,
    cat: &crate::attribs::Stat,
    file: &crate::attribs::Stat,
    can_checksum: bool,
    chksum: &str,
    rerunning: bool,
    chksum_cmp: &mut dyn FnMut() -> Option<bool>,
    warn: &mut dyn FnMut(String),
    fname: &str,
) -> bool {
    let mut status = false;
    for c in opts.chars() {
        if status {
            break;
        }
        match c {
            'i' => status = cat.ino != file.ino,
            'p' => status = cat.mode != file.mode,
            'n' => status = cat.nlink != file.nlink,
            'u' => status = cat.uid != file.uid,
            'g' => status = cat.gid != file.gid,
            's' => status = cat.size != file.size,
            'a' => status = cat.atime != file.atime,
            'm' => status = cat.mtime != file.mtime,
            'c' => status = cat.ctime != file.ctime,
            'd' => status = cat.size > file.size,
            'A' => status = true,
            '5' | '1' => {
                if can_checksum {
                    if chksum.is_empty() && !rerunning {
                        warn(format!("Cannot verify checksum for {fname}\n"));
                        status = true;
                    } else {
                        status = !chksum_cmp().unwrap_or(false);
                    }
                }
            }
            _ => {}
        }
    }
    status
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_and_compare() {
        let mut l = FileList::new(2);
        l.add_record(b"/a\0P0A\0");
        l.add_record(b"/b\0P0A\0abc\x0012\0");
        l.add_record(b"/a\0P0A\0");
        assert_eq!(l.files.len(), 2);
        assert_eq!(l.files[&b"/b".to_vec()].chksum, "abc");
        assert_eq!(l.files[&b"/b".to_vec()].delta_seq, 12);
        assert_eq!(l.end_load_messages().len(), 1);
        let a = crate::attribs::Stat { mtime: 1, ..Default::default() };
        let b = crate::attribs::Stat { mtime: 2, ..Default::default() };
        let mut no = || None;
        assert!(changed("Cmcs", &a, &b, false, "", false, &mut no, &mut |_| {}, "f"));
        assert!(!changed("Cs", &a, &b, false, "", false, &mut no, &mut |_| {}, "f"));
    }
}
