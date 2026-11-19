//! Glossar des Testgerüsts: ein Begriff je Bedeutung.
//!
//! Zweck: Die Namen im Gerüst sind Bareos-Begriffe. Dieses Modul erklärt sie
//! einmal, damit Code und Doku sie ohne Vorwissen lesbar verwenden. Es enthält
//! keinen Code.
//!
//! | Begriff | Bedeutung im Gerüst |
//! |---|---|
//! | **Gerüst** (rig) | Ein Satz aus Director, File Daemon, Storage Daemon und Katalog, der für einen Test gestartet und danach abgeräumt wird ([`crate::Rig`]). |
//! | **Director** | Der Steuer-Daemon (`odir`, Bareos: `bareos-dir`). Er plant Jobs, spricht mit FD und SD und schreibt den Katalog. |
//! | **File Daemon (FD)** | Der Daemon auf dem gesicherten Rechner (`ofd`, Bareos: `bareos-fd`). Er liest beim Backup die Dateien und schreibt sie beim Restore zurück. |
//! | **Storage Daemon (SD)** | Der Daemon am Speichergerät (`bareos-sd`, später `osd`). Er schreibt die Daten des FD in Volumes. |
//! | **Katalog** | Die PostgreSQL-Datenbank des Directors mit Jobs, Dateien und Volumes. Im Gerüst ein eigener Cluster ([`crate::catalog::Cluster`]). |
//! | **Konsole** | Die Sitzung, über die ein Mensch (hier der Test) dem Director Befehle gibt; Bareos: `bconsole` ([`crate::console::Console`]). |
//! | **Job** | Ein Auftrag des Directors: Backup, Restore, Verify oder Estimate. Er hat eine **JobId** (Nummer im Katalog). |
//! | **Job-Status** | Ein Buchstabe im Katalog (`Job.JobStatus`): `T` fertig ohne Fehler, `E` fertig mit Fehler, `f` fatal, `A` abgebrochen, `R` läuft, `C` wartet, `D` fertig mit Abweichungen (Verify). |
//! | **FileSet** | Die Auswahl, welche Dateien ein Backup umfasst, samt Optionen (Prüfsumme, Kompression, Sparse). |
//! | **Volume** | Eine Datei im Speicherverzeichnis des SD, in die ein Backup geschrieben wird. |
//! | **Pool** | Eine Menge von Volumes mit gemeinsamen Regeln; hier ein Pool mit automatisch benannten Datei-Volumes. |
//! | **Verify VolumeToCatalog** | Prüfjob: liest das Volume und vergleicht die Attribute mit dem Katalog. |
//! | **Estimate** | Fragt den FD, welche Dateien ein Backup umfassen würde, ohne zu sichern. |
//! | **Abbruch** (cancel) | Ein laufender Job wird über die Konsole beendet; erwarteter Endstatus `A`. |
//! | **Sparse-Datei** | Datei mit Löchern: Bereiche, die nie geschrieben wurden und als Nullen gelesen werden. |
//! | **Hardlink** | Zwei Namen für dieselbe Datei (gleiche Inode). Bareos sichert den Inhalt einmal und stellt den zweiten Namen als Link wieder her. |
//! | **Austauschbarer Daemon** | Ein Programm, dessen Pfad eine Umgebungsvariable wählt (`ONESIMUS_RIG_SD`, `ONESIMUS_RIG_ODIR`, `ONESIMUS_RIG_OFD`). |
//! | **Erwarteter Defekt** | Ein im Gutachten gemessener Fehler (z. B. D1), den ein eigener `#[should_panic]`-Test festhält, bis er behoben ist. |
