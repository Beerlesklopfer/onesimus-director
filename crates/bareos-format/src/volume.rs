//! Volume-Blockformat: vorbereitet, noch ohne Code.
//!
//! Zweck: Ein Datei-Volume des SD ist eine Folge von Blöcken; jeder Block hat
//! einen Kopf und enthält Datensätze ([`crate::record`]). Der neue osd braucht
//! Parser und Writer dafür. Heute gibt es im Repository keinen Code für das
//! Blockformat (kein `BB02`, kein Volume-Label), deshalb steht hier nur die
//! Struktur mit Quellenangabe – kein ungetesteter Format-Code.
//!
//! Struktur laut Bareos `core/src/stored/block.h`, Zweig `bareos-25`
//! (gelesen 19.11.2026):
//!
//! | Kennung | Länge | Felder (serialisiert, in dieser Reihenfolge) |
//! |---|---|---|
//! | `BB01` (`BLKHDR1_LENGTH`) | 16 | `CheckSum` u32, `block_len` u32, `BlockNumber` u32, `Id` `"BB01"` |
//! | `BB02` (`BLKHDR2_LENGTH`, geschrieben: `WRITE_BLKHDR_LENGTH`) | 24 | wie `BB01` mit `Id` `"BB02"`, dazu `VolSessionId` u32, `VolSessionTime` u32 |
//!
//! Weitere Konstanten: `BLKHDR_CS_LENGTH` 4, `DEFAULT_BLOCK_SIZE` 64 512
//! (512 × 126), `MAX_BLOCK_LENGTH` 20 000 000, `BLOCK_VER` 2.
//! Volume-Label: `BareosId` `"Bareos 2.0 immortal\n"`, `BareosTapeVersion` 20,
//! serialisiert höchstens 1024 Byte (`SER_LENGTH_Volume_Label`). Offen und vor
//! dem ersten Code zu lesen: Byte-Reihenfolge der Felder (`ser.h`), welche
//! Bytes die Prüfsumme abdeckt (`block.cc` `BlockCheckSum`), die Kodierung
//! des Labels (`label.cc`).
//!
//! Bareos-Gegenstück: `core/src/stored/block.cc` (`WriteBlockToDevice`,
//! `ReadBlockFromDevice`, `UnserBlockHeader`), `core/src/stored/label.cc`
//! (`WriteNewVolumeLabelToDev`, `ReadDevVolumeLabel`).
