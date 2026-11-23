// SPDX-License-Identifier: AGPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
//! Glossar von daemon-crate: ein Begriff je Bedeutung.
//!
//! Zweck: die Begriffe des Daemon-Gerüsts ohne Bareos-Vorwissen. Kein Code.
//!
//! | Begriff | Bedeutung |
//! |---|---|
//! | **Daemon** | Ein Hintergrundprogramm (ofd, osd, odir), das Verbindungen annimmt und Jobs bearbeitet. |
//! | **Vordergrund** (`-f`) | Der Daemon bleibt am Terminal bzw. unter systemd am Prozess; kein `fork`. |
//! | **Hintergrund** (daemonize) | `fork`, neue Sitzung, Arbeitsverzeichnis wechseln; das Terminal ist danach frei ([`crate::background`]). |
//! | **PID-Datei** | Datei mit der Prozessnummer, damit `-k` oder ein Init-Skript den Daemon findet ([`crate::pidfile`]). |
//! | **Rechteabgabe** | Wechsel von root zu einem Dienstbenutzer (`-u`, `-g`) ([`crate::privileges`]). |
//! | **streng / nachsichtig** | Rechteabgabe, die bei jedem Fehler abbricht (odir, osd) bzw. Fehler übergeht (ofd heute). |
//! | **Abschaltsignal** | `SIGTERM` oder `SIGINT`: sauber beenden. |
//! | **Neuladesignal** | `SIGHUP`: Konfiguration neu lesen, ohne zu beenden. |
//! | **Signalmaske** | Welche Signale ein Thread nicht zugestellt bekommt; neue Threads erben sie ([`crate::signals`]). |
//! | **Konfigurationsprüfung** (`-t`) | Konfiguration lesen, Ergebnis melden, mit Exit-Code enden ([`crate::config_check`]). |
//! | **Exit-Code** | Rückgabewert des Prozesses; 41 und 42 verhindern einen Neustart durch systemd ([`crate::exit`]). |
//! | **Debug-Level** (`-d`) | Ausführlichkeit der Bareos-Debugausgabe; ab 1 schaltet das Logging auf `debug` ([`crate::logging`]). |
//! | **Accept-Schleife** | Wartet auf dem Port des Daemons und gibt jede Verbindung an einen eigenen Thread ([`crate::accept::serve`]). |
//! | **PSK-Identität** | Name, den eine Gegenstelle beim TLS-PSK nennt (`R_DIRECTOR\x1e<Name>`, `R_JOB\x1e<Job>`); er wählt den Schlüssel ([`crate::accept::PskIdentity`]). |
//! | **Hello** | Erste Zeile einer Gegenstelle, z. B. `Hello Director <Name> calling` ([`crate::handshake`]). |
//! | **OK-Hello** | Antwort des Daemons nach erfolgreichem CRAM-MD5 (`2000 OK Hello 54` FD, `3000 OK Hello` SD). |
//! | **Bandbreitengrenze** | Höchstzahl Bytes je Sekunde einer Datenverbindung (`setbandwidth`, [`crate::bwlimit`]). |
