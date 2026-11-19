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
//! Die zweite Kodierung ist der Defektverdacht D13 des Gutachtens: Bareos
//! schickt im Hello `BashSpaces`, also `0x01`. Sie ist hier nur benannt und
//! unverändert übernommen, nicht behoben.
//!
//! Datenfluss: Director → (Kommandozeile) → FD/SD; FD/SD → (Antwort mit
//! Namen) → Director.
//!
//! Zusammengeführt (19.11.2026) aus `odir/src/catreq.rs`
//! (`bash_spaces`, `unbash_spaces`), `odir/src/sd_label.rs` (`bash_spaces`),
//! `odir/src/sd_copy.rs` (`bash`), `odir/src/job_messages.rs`
//! (`unbash_spaces`), `ofd/src/job.rs` (`unbash`) und
//! `bareos-bnet/src/cram_md5.rs` (`bash_spaces`, `unbash_spaces` mit `'_'`).

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

/// Onesimus Hello form: every blank becomes `'_'` (not Bareos, see D13).
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
