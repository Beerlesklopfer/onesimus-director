# Backlog

## Postgres-Plugin für Verzeichnisbäume (BVFS)

Ein fertiges Postgres-Plugin-Crate (auf crates.io, Name noch eintragen) löst die
Verzeichnisbäume im Catalog direkt in PostgreSQL auf. Damit lässt sich im
Restore-Baum navigieren, wie es bconsole tut (`cd`, `ls`, `mark`).

- Crate-Name offen. Globale crates.io-Suche (13.10.2026, Begriffe: bareos, bacula,
  bvfs, path hierarchy, ltree, pg tree, restore tree …) ohne eindeutigen Treffer;
  entfernt ähnlich: `closure-tree`, `diesel_ltree`, `ltree2viz`.
- Einbinden statt der heutigen BVFS-Stubs in `crates/onesimus-dird/src/commands/dot.rs`
  (`.bvfs_update`, `.bvfs_lsdirs`, `.bvfs_lsfiles`, `.bvfs_versions` …)
- Danach als Tools im MCP-Modus von `oconsole` anbieten (Backup-Baum durchsuchen)
- Restore-Baum im interaktiven `oconsole` wie bei bconsole

## JSON-Ausgabe (`.api 2`) für alle GUI-Befehle

Der Director liefert JSON bisher nur bei `.help`. Die Onesimus-GUI braucht es
für `list`, `llist`, `show`, `.jobs`, `.clients`, … – gilt für BNET und gRPC.
