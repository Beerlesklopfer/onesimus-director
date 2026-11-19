//! Datensätze (Records) im Volume: vorbereitet, noch ohne Code.
//!
//! Zweck: Der SD zerlegt den Datenstrom des FD in Datensätze und schreibt sie
//! in Blöcke ([`crate::volume`]). Jeder Datensatz hat einen Kopf und Nutzdaten.
//! Dieses Modul nimmt später Typen, Parser und Writer dafür auf. Heute gibt es
//! im Repository keinen Code für Datensätze im Volume, deshalb steht hier nur
//! die Struktur mit Quellenangabe – kein ungetesteter Format-Code.
//!
//! Struktur laut Bareos `core/src/stored/record.h` und `block.h`, Zweig
//! `bareos-25` (gelesen 19.11.2026):
//!
//! | Version | Länge | Felder (serialisiert, in dieser Reihenfolge) |
//! |---|---|---|
//! | 1 (`RECHDR1_LENGTH`) | 20 | `VolSessionId` u32, `VolSessionTime` u32, `FileIndex` i32, `Stream` i32, `data_len` u32 |
//! | 2 (`RECHDR2_LENGTH`, geschrieben: `WRITE_RECHDR_LENGTH`) | 12 | `FileIndex` i32, `Stream` i32, `data_len` u32 |
//!
//! In Version 2 stehen `VolSessionId` und `VolSessionTime` im Blockkopf
//! (`BB02`) statt im Datensatzkopf.
//!
//! Label-Datensätze haben einen negativen `FileIndex` (`enum : int32_t` in
//! `record.h`): `PRE_LABEL` −1, `VOL_LABEL` −2, `EOM_LABEL` −3, `SOS_LABEL` −4,
//! `EOS_LABEL` −5, `EOT_LABEL` −6, `SOB_LABEL` −7, `EOB_LABEL` −8. Die
//! Byte-Kodierung der Label-Inhalte (`ser_volume_label`, `UnserVolumeLabel`,
//! `stored/label.cc`) ist noch nicht gelesen.
//!
//! Abgrenzung: Der Kopf, den der FD über das Netz an den SD schickt
//! (`<FileIndex> <Stream> <Länge>` als Text) und die Antwort `rechdr …` beim
//! Restore sind ein anderes Format; sie liegen heute in
//! `ofd/src/backup.rs` (`header`) und `ofd/src/restore.rs` (`parse_rechdr`)
//! und ziehen hierher um, wenn osd sie braucht (offener Punkt).
//!
//! Bareos-Gegenstück: `core/src/stored/record.cc` (`WriteRecordToBlock`,
//! `ReadRecordFromBlock`), `core/src/stored/record.h`.
