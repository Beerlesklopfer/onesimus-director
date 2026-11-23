// SPDX-License-Identifier: AGPL-3.0-only
// SPDX-FileCopyrightText: 2000-2012 Free Software Foundation Europe e.V.
// SPDX-FileCopyrightText: 2010-2017 Planets Communications B.V.
// SPDX-FileCopyrightText: 2012-2026 Bareos GmbH & Co. KG
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Dateitypen (`FT_*`): wie der FD eine gefundene Datei einordnet.
//!
//! Zweck: Der Attribut-Datensatz trägt neben der LStat-Zeichenkette den
//! Dateityp, den die Dateisuche des FD vergeben hat: gewöhnliche Datei,
//! Verzeichnisanfang/-ende, Symlink, gespeicherter Hardlink, nicht lesbar,
//! übersprungen, gelöscht (Accurate), Plugin-Objekt. Der Director wertet ihn
//! beim Eintragen in den Katalog aus, der FD beim Restore.
//!
//! Datenfluss: FD `find` → Attribut-Datensatz (`<FileIndex> <Typ> …`) → SD →
//! Director `catreq` und FD `restore`.
//!
//! Bareos-Gegenstück: `core/src/include/filetypes.h`.
//!
//! Zusammengeführt (19.11.2026) aus `ofd/src/attribs.rs`, `ofd/src/find.rs`
//! und `odir/src/catreq.rs`.

/// Hard link to a file already saved (`LinkFI` points to it).
pub const FT_LNKSAVED: i32 = 1;
/// Regular file, empty.
pub const FT_REGE: i32 = 2;
/// Regular file.
pub const FT_REG: i32 = 3;
/// Symbolic link.
pub const FT_LNK: i32 = 4;
/// Directory, sent after its contents.
pub const FT_DIREND: i32 = 5;
/// Special file (device, socket).
pub const FT_SPEC: i32 = 6;
/// No access (permission denied).
pub const FT_NOACCESS: i32 = 7;
/// Could not follow a link.
pub const FT_NOFOLLOW: i32 = 8;
/// Could not stat.
pub const FT_NOSTAT: i32 = 9;
/// Unchanged since the last backup (incremental).
pub const FT_NOCHG: i32 = 10;
/// Directory unchanged since the last backup.
pub const FT_DIRNOCHG: i32 = 11;
/// The archive file itself, not saved.
pub const FT_ISARCH: i32 = 12;
/// Directory not descended (no recursion).
pub const FT_NORECURSE: i32 = 13;
/// Different file system, not descended.
pub const FT_NOFSCHG: i32 = 14;
/// Could not open the directory.
pub const FT_NOOPEN: i32 = 15;
/// Raw block device.
pub const FT_RAW: i32 = 16;
/// Named pipe.
pub const FT_FIFO: i32 = 17;
/// Directory, sent before its contents.
pub const FT_DIRBEGIN: i32 = 18;
/// File system type not allowed.
pub const FT_INVALIDFS: i32 = 19;
/// Drive type not allowed (Windows).
pub const FT_INVALIDDT: i32 = 20;
/// Windows reparse point.
pub const FT_REPARSE: i32 = 21;
/// Deleted since the last backup (Accurate).
pub const FT_DELETED: i32 = 23;
/// Unchanged against a base job.
pub const FT_BASE: i32 = 24;
/// Restore object that must be restored first.
pub const FT_RESTORE_FIRST: i32 = 25;
/// Windows junction.
pub const FT_JUNCTION: i32 = 26;
/// Plugin configuration object.
pub const FT_PLUGIN_CONFIG: i32 = 27;
/// Plugin configuration object, filled.
pub const FT_PLUGIN_CONFIG_FILLED: i32 = 28;

/// Mask of the file type in the type field of an attribute record.
pub const FT_MASK: i32 = 0xFFFF;
/// Flag in the type field: a data stream id follows the attributes (filetypes.h:84).
pub const AR_DATA_STREAM: i32 = 1 << 16;

/// `IS_FT_OBJECT()`: a restore or plugin configuration object.
pub fn is_ft_object(t: i32) -> bool {
    t == FT_RESTORE_FIRST || t == FT_PLUGIN_CONFIG_FILLED || t == FT_PLUGIN_CONFIG
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_match_the_replaced_constants() {
        // ofd/src/attribs.rs, ofd/src/find.rs, odir/src/catreq.rs (19.11.2026)
        assert_eq!((FT_LNKSAVED, FT_REG, FT_LNK, FT_DIREND, FT_DIRBEGIN), (1, 3, 4, 5, 18));
        assert_eq!((FT_INVALIDFS, FT_INVALIDDT, FT_DELETED, FT_BASE), (19, 20, 23, 24));
        assert_eq!((FT_RESTORE_FIRST, FT_PLUGIN_CONFIG_FILLED, FT_MASK, AR_DATA_STREAM), (25, 28, 0xFFFF, 0x10000));
        assert!(is_ft_object(FT_PLUGIN_CONFIG) && !is_ft_object(FT_REG));
    }
}
