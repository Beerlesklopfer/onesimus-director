//! Das File-Device: Volumes als Dateien im `Archive Device`.
//!
//! Zweck: Ein Device vom Typ `File` legt jedes Volume als Datei
//! `<Archive Device>/<Volumename>` an. Dieses Modul öffnet, liest und
//! beschreibt sie: Volume-Label lesen und neu schreiben (Block 0), Blöcke
//! anhängen. Das Format selbst kommt aus `bareos-format`.
//!
//! Bareos-Gegenstück: `core/src/stored/backends/unix_file_device.cc`,
//! `stored/dev.cc` (`print_name`, `open`, `eod`), `stored/label.cc`
//! (`WriteNewVolumeLabelToDev`, `CreateVolumeLabel`, `ReadDevVolumeLabel`).

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;

use bareos_format::label::{VolumeLabel, VOL_LABEL, PRE_LABEL};
use bareos_format::volume::{BlockBuilder, RecordProgress, RecordRef};

/// The settings of a File device that osd uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileDevice {
    /// `Name` of the Device resource.
    pub name: String,
    /// `Archive Device`: the directory of the volumes.
    pub archive_device: PathBuf,
    /// `Media Type`.
    pub media_type: String,
    /// `Label Media`.
    pub label_media: bool,
    /// `Maximum Block Size` (bytes of a data block).
    pub max_block_size: u32,
    /// `Label Block Size` (buffer of the label block).
    pub label_block_size: u32,
    /// `Maximum Volume Size`; 0 = none.
    pub max_volume_size: u64,
}

/// What the label check of a volume file found (`ReadDevVolumeLabel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelState {
    /// No file or an empty file (`VOL_NO_LABEL`).
    NoLabel,
    /// A Bareos volume label.
    Labeled(Box<VolumeLabel>),
    /// Data that is not a Bareos volume (`VOL_LABEL_ERROR`, `VOL_IO_ERROR`).
    Broken(String),
}

impl FileDevice {
    /// The device of a `bareos-sd.d` Device resource.
    pub fn from_config(dev: &bareos_config::sd::SdDevice) -> FileDevice {
        FileDevice {
            name: dev.name.clone(),
            archive_device: PathBuf::from(&dev.archive_device),
            media_type: dev.media_type.clone(),
            label_media: dev.label_media,
            max_block_size: dev.maximum_block_size,
            label_block_size: dev.label_block_size,
            max_volume_size: dev.maximum_volume_size,
        }
    }

    /// `print_name()`: `"<Name>" (<Archive Device>)`.
    pub fn print_name(&self) -> String {
        format!("\"{}\" ({})", self.name, self.archive_device.display())
    }

    /// The file of `volume`.
    pub fn volume_path(&self, volume: &str) -> PathBuf {
        self.archive_device.join(volume)
    }

    /// Reads the label of `volume` (`ReadDevVolumeLabel` for a file).
    pub fn read_label(&self, volume: &str) -> LabelState {
        let mut file = match File::open(self.volume_path(volume)) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return LabelState::NoLabel,
            Err(e) => return LabelState::Broken(e.to_string()),
        };
        let mut buf = vec![0u8; bareos_format::volume::BLKHDR2_LENGTH];
        match file.read_exact(&mut buf) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                return match file.metadata().map(|m| m.len()) {
                    Ok(0) => LabelState::NoLabel,
                    _ => LabelState::Broken("short volume".into()),
                };
            }
            Err(e) => return LabelState::Broken(e.to_string()),
        }
        let header = match bareos_format::volume::BlockHeader::parse(&buf) {
            Ok(h) => h,
            Err(e) => return LabelState::Broken(e),
        };
        buf.resize(header.block_len as usize, 0);
        if let Err(e) = file.read_exact(&mut buf[bareos_format::volume::BLKHDR2_LENGTH..]) {
            return LabelState::Broken(e.to_string());
        }
        let contents = match bareos_format::volume::read_volume(&buf) {
            Ok(c) => c,
            Err(e) => return LabelState::Broken(e),
        };
        match contents.records.first() {
            Some(r) if r.file_index == VOL_LABEL || r.file_index == PRE_LABEL => match VolumeLabel::parse(&r.data) {
                Ok(l) => LabelState::Labeled(Box::new(l)),
                Err(e) => LabelState::Broken(e),
            },
            _ => LabelState::Broken("first record is not a volume label".into()),
        }
    }

    /// `WriteNewVolumeLabelToDev`: creates or truncates the file of
    /// `label.volume_name` and writes the label block (number 0) with the
    /// session `vol_session_id`/`vol_session_time`. Returns its length.
    pub fn write_label(&self, label: &VolumeLabel, vol_session_id: u32, vol_session_time: u32) -> Result<u64, String> {
        let data = label.serialize();
        let mut builder = BlockBuilder::new(self.label_block_size.max(1024) as usize);
        let rec = RecordRef { vol_session_id, vol_session_time, file_index: VOL_LABEL, stream: 0, data: &data };
        if !builder.write_record(&rec, &mut RecordProgress::default()) {
            return Err(format!("Cannot write Volume label to block for device {}", self.print_name()));
        }
        let block = builder.finish(0);
        let path = self.volume_path(&label.volume_name);
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| format!("Open device {} Volume \"{}\" failed: ERR={e}", self.print_name(), label.volume_name))?;
        file.write_all(&block).and_then(|_| file.sync_data()).map_err(|e| format!("Write label to {}: {e}", path.display()))?;
        Ok(block.len() as u64)
    }

    /// Opens `volume` for appending; returns the file positioned at its end
    /// and that position (`eod`).
    pub fn open_append(&self, volume: &str) -> Result<(File, u64), String> {
        let path = self.volume_path(volume);
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("Open device {} Volume \"{volume}\" failed: ERR={e}", self.print_name()))?;
        let end = file.seek(SeekFrom::End(0)).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok((file, end))
    }
}

/// `edit_uint64_with_commas`: `12,592,463`.
pub fn with_commas(v: u64) -> String {
    let digits = v.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `edit_uint64_with_suffix`: `3.147 M` for 3 147 xxx.
pub fn with_suffix(v: u64) -> String {
    let suffix = ["", "K", "M", "G", "T", "P", "E", "Z", "Y"];
    let commas = with_commas(v);
    let groups = commas.matches(',').count();
    let mut mbuf = commas.replacen(',', ".", 1);
    if groups > 0 {
        // drop the remaining groups, keep 5 characters ("123.4")
        if let Some(cut) = mbuf.find(',') {
            mbuf.truncate(cut);
        }
        mbuf.truncate(5);
    }
    format!("{mbuf} {}", suffix[groups.min(suffix.len() - 1)])
}

/// The host name for volume labels (`gethostname`).
pub fn host_name() -> String {
    nix::unistd::gethostname().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A new volume label as `CreateVolumeLabel` makes it: `LabelProg` is the
/// daemon name (the Storage resource name, Bareos `my_name`).
pub fn new_label(volume: &str, pool: &str, media_type: &str, daemon: &str) -> VolumeLabel {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let version = onesimus_daemon::version::VERSION;
    let date = onesimus_daemon::version::BUILD_DATE;
    VolumeLabel::new(
        volume,
        pool,
        media_type,
        &host_name(),
        daemon,
        &format!("Ver. {} {}", &version[..version.len().min(26)], &date[..date.len().min(17)]),
        &format!("Build {date}"),
        now.as_micros() as i64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn number_editing_like_lib_edit_cc() {
        assert_eq!(with_commas(12_592_463), "12,592,463");
        assert_eq!(with_commas(230), "230");
        assert_eq!(with_suffix(3_147_617), "3.147 M");
        assert_eq!(with_suffix(999), "999 ");
        assert_eq!(with_suffix(1_073_741_824), "1.073 G");
    }

    #[test]
    fn label_is_written_and_read_back() {
        let dir = std::env::temp_dir().join(format!("osd-device-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dev = FileDevice {
            name: "Dev".into(),
            archive_device: dir.clone(),
            media_type: "File".into(),
            label_media: true,
            max_block_size: 1 << 20,
            label_block_size: 64512,
            max_volume_size: 0,
        };
        assert_eq!(dev.read_label("V1"), LabelState::NoLabel);
        let label = new_label("V1", "Pool", "File", "osd-sd");
        let len = dev.write_label(&label, 0, 0).unwrap();
        assert_eq!(len, std::fs::metadata(dev.volume_path("V1")).unwrap().len());
        match dev.read_label("V1") {
            LabelState::Labeled(l) => assert_eq!((l.volume_name.as_str(), l.label_prog.as_str()), ("V1", "osd-sd")),
            other => panic!("{other:?}"),
        }
        let (_, end) = dev.open_append("V1").unwrap();
        assert_eq!(end, len);
        std::fs::write(dev.volume_path("junk"), b"not a volume at all, really not").unwrap();
        assert!(matches!(dev.read_label("junk"), LabelState::Broken(_)));
        std::fs::remove_dir_all(&dir).ok();
    }
}
