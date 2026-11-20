//! Glossar von bareos-format: ein Begriff je Bedeutung.
//!
//! Zweck: Bareos benutzt manche Wörter für mehrere Dinge. Hier steht, was ein
//! Begriff in diesem Crate bedeutet. Das Modul enthält keinen Code.
//!
//! | Begriff | Bedeutung |
//! |---|---|
//! | **Stream-Kennung** | Satzart eines Datensatzes (`STREAM_*`, [`crate::stream`]). Nie eine Netzverbindung; die heißt hier **Verbindung**. |
//! | **Datensatz** (record) | Ein Kopf plus Nutzdaten im Datenstrom FD → SD bzw. im Volume ([`crate::record`]). |
//! | **Block** | Einheit, in der der SD Datensätze in ein Volume schreibt, mit Kopf `BB02` ([`crate::volume`]). |
//! | **Volume** | Eine Datei (oder ein Band) des SD mit Blöcken. |
//! | **Label** | Datensatz mit negativem FileIndex, der Volume, Sitzung oder Ende markiert. |
//! | **FileIndex** | Laufende Nummer einer Datei innerhalb eines Jobs (ab 1); negativ bei Labels. |
//! | **LStat** | Die Zeichenkette mit den `stat`-Feldern einer Datei ([`crate::lstat`]). |
//! | **LinkFI** | FileIndex der zuerst gesicherten Datei eines Hardlinks; 0, wenn keiner. |
//! | **Bareos-Base64** | Bareos-eigene Kodierung von Zahlen und Binärdaten ([`crate::base64`]); nicht RFC 4648 bei Zahlen. |
//! | **Dateityp** | Einordnung einer Datei durch die Dateisuche des FD (`FT_*`, [`crate::filetype`]). |
//! | **Meldungsart** | Schwere einer Meldung (`M_*`, [`crate::msg`]). |
//! | **BashSpaces** | Kodierung von Leerzeichen als `0x01` in Kommandozeilen ([`crate::bash::bash_spaces`]). |
//! | **Unterstrich-Kodierung** | Onesimus-Hello-Form `' '` → `'_'` ([`crate::bash::underscore_spaces`]); nicht Bareos (D13). |
//! | **Job-Kommando** | Die Anmeldezeile eines Jobs beim SD (`JobId=… job=…`, [`crate::sd_job::JobCommand`]). |
//! | **eindeutiger Jobname** | `job=`: Jobname mit Datum und Zähler, z. B. `VBackup.2026-11-19_05.06.21_04`. |
//! | **rechdr** | Kopf vor jedem Datensatz, den der SD beim Restore an den FD schickt ([`crate::record::RecHdr`]). |
//! | **Datenkopf** | Kopf `<FileIndex> <Stream> 0` vor jedem Datensatz vom FD an den SD ([`crate::record::data_header`]). |
//! | **regexwhere** | Liste `!von!nach!optionen` zum Umschreiben von Dateinamen beim Restore ([`crate::breg`]). |
//! | **Sitzung** (VolSessionId/-Time) | Kennung, unter der ein SD die Datensätze eines Jobs schreibt; nicht die TCP-Sitzung. |
