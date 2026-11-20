//! Glossar von osd-ring: ein Begriff je Bedeutung.
//!
//! Das Modul enthält keinen Code.
//!
//! | Begriff | Bedeutung |
//! |---|---|
//! | **Ring** | Eine Datei im gemeinsamen Speicher je Job (`osd-<Port>-<JobId>.ring`), die Erzeuger und Verbraucher einblenden ([`crate::Ring`]). |
//! | **Fach** (Slot) | Platz für genau einen Bareos-Block mit Länge und sieben Metawörtern ([`crate::SlotMut`], [`crate::SlotRef`]). |
//! | **Erzeuger** (Producer) | Die Netzsitzung des Jobs; sie füllt Fächer ([`crate::Ring::claim`]). |
//! | **Verbraucher** (Consumer) | Ein Geräteschreiber; er liest Fächer ([`crate::Ring::next`]) und gibt sie frei ([`crate::Ring::release`]). |
//! | **Schreibzeiger** (Cursor) | Zahl der veröffentlichten Fächer. |
//! | **Lesezeiger** (Sequence) | Je Verbraucher die Zahl der gelesenen Fächer. |
//! | **Freigabe** (Gating) | Ein Fach ist frei, wenn der kleinste Lesezeiger der noch zählenden Verbraucher es passiert hat. |
//! | **Postfach** (Mailbox) | Je Verbraucher ein Bereich für eine Anfrage an den Erzeuger und dessen Antwort ([`crate::Ring::call`], [`crate::Ring::serve_mail`]). |
//! | **Info-Bereich** | Bytes, die der Erzeuger vor dem Start der Verbraucher hinterlegt (Job-Angaben). |
//! | **Herzschlag** | Zähler je Verbraucher; steht er bei vollem Ring [`crate::LIVENESS`] lang, wird der Verbraucher ausgeschlossen. |
//! | **Weckwort** | 32-Bit-Wort im Kopf, auf das prozessübergreifend gewartet wird (`futex`, [`crate::sys`]). |
