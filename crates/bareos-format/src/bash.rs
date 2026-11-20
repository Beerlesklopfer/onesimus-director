//! Leerzeichen in Namen auf der Leitung: zwei verschiedene Kodierungen.
//!
//! Zweck: Bareos-Kommandos sind Zeilen aus `schlüssel=wert`-Paaren, durch
//! Leerzeichen getrennt. Ein Name mit Leerzeichen (Job, Client, Pool,
//! Device) muss deshalb kodiert werden. Im Repository gibt es dafür zwei
//! Verfahren mit zwei Namen:
//!
//! | Funktion | Kodierung | Bareos-Gegenstück | genutzt für |
//! |---|---|---|---|
//! | [`bash_spaces`] / [`unbash_spaces`] | `' '` ↔ `0x01` | `lib/util.cc` `BashSpaces`, `UnbashSpaces` | SD-/FD-Kommandos (Job-Kommando, `use storage`, `label`, Katalog-Anfragen) |
//! | [`underscore_spaces`] / [`ununderscore_spaces`] | `' '` ↔ `'_'` | keins (Onesimus) | Hello-Zeile des Directors in `bareos-bnet` |
//!
//! Die zweite Kodierung ist Defekt D13 des Gutachtens
//! (`docs/audits/ofd-struktur-2026-11-19.md`). Am Bareos-Quelltext
//! nachgeprüft am 20.11.2026 (Commit `18be77e6`, Zweig `bareos-25`):
//!
//! - `lib/util.cc` `BashSpaces` ersetzt `' '` durch `0x01`, `UnbashSpaces`
//!   umgekehrt. Eine Variante mit `'_'` gibt es in Bareos nicht.
//! - Hello des Directors an FD und SD: `dird/authenticate.cc` ruft
//!   `BashSpaces(dirname)` vor `fsend(hello, dirname)` (Zeile 73).
//! - Hello einer Konsole: `lib/bsock.cc` `ConsoleAuthenticateWithDirector`
//!   ruft `BashSpaces(bashed_name)` (Zeile 344).
//! - Gegenstelle: `lib/util.cc` `GetNameAndResourceTypeAndVersionFromHello`
//!   ersetzt `0x01` wieder durch `' '` (Zeile 238); ofd liest das Hello
//!   ebenso (`0x01` → `' '`).
//! - CRAM-MD5: `lib/cram_md5.cc` vergleicht den qualifizierten Namen in
//!   der `BashSpaces`-Form (Zeile 76).
//! - SD-Job-Kommando: `dird/msgchan.cc` `StartStorageDaemonJob` bashed
//!   `job_name`, `client_name`, `fileset_name`, `backup_format` (Zeilen
//!   308–328) und in `use storage` `pool_type`, `pool_name`, Storage-Name,
//!   `media_type`, `device_name` (Zeilen 153–167, 228–243).
//!
//! Folge: Ein Director-Name mit Leerzeichen geht über `bareos-bnet` als
//! `My_Dir` hinaus; ein Bareos-SD/-FD (und ofd) sucht dann einen Director
//! `My_Dir` und lehnt ab. Die Funktionen bleiben trotzdem getrennt und
//! unverändert, weil P0 kein Verhalten ändert; die Umstellung des Hello
//! auf [`bash_spaces`] ist ein eigener Schritt (P1, Entscheidung beim
//! Nutzer).
//!
//! Datenfluss: Director → (Kommandozeile) → FD/SD; FD/SD → (Antwort mit
//! Namen) → Director.
//!
//! Zusammengeführt (19.11.2026) aus `odir/src/catreq.rs`
//! (`bash_spaces`, `unbash_spaces`), `odir/src/sd_label.rs` (`bash_spaces`),
//! `odir/src/sd_copy.rs` (`bash`), `odir/src/job_messages.rs`
//! (`unbash_spaces`), `ofd/src/job.rs` (`unbash`) und
//! `bareos-bnet/src/cram_md5.rs` (`bash_spaces`, `unbash_spaces` mit `'_'`).
//! Am 20.11.2026 folgten die Inline-Kopien (`replace(' ', "\x01")` und
//! umgekehrt) in `bareos-bnet/src/hello.rs`, `odir/src/runscript.rs`,
//! `job_worker.rs` (`fd_restore_command`), `ua_storage_select.rs`,
//! `commands/label.rs`, `commands/status.rs`, `ofd/src/accept.rs` und
//! `ofd/src/runscript.rs`.

/// The byte `BashSpaces()` puts in place of a blank.
pub const BASHED_SPACE: char = '\u{1}';

/// `BashSpaces()`: every blank becomes `0x01`.
pub fn bash_spaces(text: &str) -> String {
    text.replace(' ', "\u{1}")
}

/// `UnbashSpaces()`: every `0x01` becomes a blank again.
pub fn unbash_spaces(text: &str) -> String {
    text.replace('\u{1}', " ")
}

/// Onesimus Hello form: every blank becomes `'_'`. Bareos uses
/// [`bash_spaces`] there (`dird/authenticate.cc`); defect D13, kept as is.
pub fn underscore_spaces(text: &str) -> String {
    text.replace(' ', "_")
}

/// Inverse of [`underscore_spaces`]: every `'_'` becomes a blank.
pub fn ununderscore_spaces(text: &str) -> String {
    text.replace('_', " ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goldens_of_the_replaced_functions() {
        // odir catreq/sd_label/sd_copy/job_messages, ofd job.rs unbash
        assert_eq!(bash_spaces("my pool  x"), "my\u{1}pool\u{1}\u{1}x");
        assert_eq!(unbash_spaces("my\u{1}pool"), "my pool");
        assert_eq!(unbash_spaces(&bash_spaces(" a b ")), " a b ");
        // bareos-bnet cram_md5 tests
        assert_eq!(underscore_spaces("my director"), "my_director");
        assert_eq!(ununderscore_spaces("my_director"), "my director");
        assert_eq!(underscore_spaces("no_spaces"), "no_spaces");
    }
}
