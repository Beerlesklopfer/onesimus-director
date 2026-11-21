# Dokumentationsregel für Crates

Entscheidung des Nutzers vom 19.11.2026. Sie gilt für jede neue Crate und für
jede Crate, die ein Umbau anfasst.

## Was jede Crate hat

- **Moduldoku** (`//!`) in jedem Modul: Zweck, Datenfluss, Bareos-Gegenstück
  (Datei und Funktion im C++-Quelltext, Zweig `bareos-25`).
- **Elementdoku** (`///`) an jedem Element, auch an privaten Funktionen,
  Feldern und Konstanten.
- **Glossar**: ein Modul `glossary` mit einer Tabelle, ein Begriff je
  Bedeutung. Doppelbedeutungen bekommen zwei Namen.
- **Diagramme**: Mermaid-Klassendiagramme über `aquamarine`
  (`#[cfg_attr(doc, aquamarine::aquamarine)]` vor dem Element, darunter ein
  Doc-Block mit ```` ```mermaid ````).
- **Modul- und Typbaum**, beim Bau erzeugt (siehe unten).

## Wie es erzwungen wird

In neuen Crates (`bareos-format`, `daemon-crate`, `onesimus-rig`):

```rust
#![deny(missing_docs)]
#![deny(clippy::missing_docs_in_private_items)]
```

In bestehenden Crates, die angefasst werden, gilt mindestens
`#![warn(clippy::missing_docs_in_private_items)]` für die angefassten Module;
die volle Regel für eine Crate wie odir (74.000 Zeilen) ist ein eigener
Arbeitsschritt.

Prüfung:

```sh
cargo clippy -p bareos-format -p daemon-crate -p onesimus-rig --all-targets
RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --document-private-items \
  -p bareos-format -p daemon-crate -p onesimus-rig
```

## Modulbaum

Jede dieser Crates hat ein `build.rs`, das `scripts/doc/module_tree.rs`
einbindet. Es läuft die Module ab `src/lib.rs` ab und schreibt den Baum nach
`$OUT_DIR/module_tree.md`; die Crate-Wurzel bindet ihn mit

```rust
#![doc = include_str!(concat!(env!("OUT_DIR"), "/module_tree.md"))]
```

in ihre Startseite ein. Der Generator liest Zeilen, er ist kein Rust-Parser:
Er sieht Deklarationen am Zeilenanfang und überspringt `#[cfg(test)]`-Module.

Eine typgeprüfte, vollständige Sicht liefert `cargo-modules`:

```sh
cargo install cargo-modules --locked
cargo modules structure --package bareos-format --lib
cargo modules dependencies --package daemon-crate --lib | dot -Tsvg > deps.svg
```

`cargo-modules` ist auf dem Entwicklungsrechner nicht installiert
(Stand 19.11.2026); deshalb der eigene Generator.
