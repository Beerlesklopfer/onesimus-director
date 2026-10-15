# Backlog

## KRITISCH: Scheduler startet keine Jobs

`scheduler.rs`: `dispatch_ready_jobs()` erhöht nur einen Zähler
(`TODO(R6-full): Fork worker process`). Zusätzlich vergleicht
`fill_queue_from_config()` Stunde/Minute in UTC statt Ortszeit, ignoriert
Tages-/Wochen-/Monatsangaben der Run-Direktive ("1st Sat", "mon-fri") und
dedupliziert nicht. Geplante Backups laufen mit onesimus-dird derzeit nicht.
Folge: `status director` liefert `scheduled: []`.


## Postgres-Plugin für Verzeichnisbäume (OVFS)

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

## JSON-Ausgabe: Rest

Erledigt: Umschlag/Rahmung, Punkt-Befehle, list/llist, .defaults job=,
.help, time, version, reload, status director, .messages/messages.
Offen: `.help` Usage von configure (aus Direktiventabellen), `.defaults` für
client/storage/pool und Restore-Jobs (where/replace) — brauchen die rohen
Direktiven aus dem show-Port; Reihenfolge der Ressourcen wie C++ (Ladereihenfolge).
`status director`: terminated-Liste (Jobs seit Start) wird nicht geführt.

## `llist pools`: prunablevolumes/-jobs/-bytes

Bareos 25.1 gibt bei `llist pools` zusätzlich eine Prune-Zusammenfassung pro
Pool aus (ua_output.cc `PoolPruneSummary`, Auswertung der Aufbewahrungsfristen).
onesimus-dird liefert die Felder noch nicht.

## Quellbasierter Director-Vollständigkeitsabgleich

[Originalbefehle und Status](wiki/commands/bareos-reference.md) · [LLM-JSON](wiki/commands/bareos-director-commands.json).

88/88 Originalnamen sind registriert; dies ist keine Funktionsparität. Belegt sind unter anderem BVFS- und Steuerungs-Stubs, fehlende interaktive Restore-Dateiauswahl, `query`/`move`-Semantikabweichungen, unvollständige ACL-Prüfung und zu großzügige MCP-Punktbefehlsfreigaben. Die Referenz enthält die Quellenbelege und unterscheidet diese Befunde von noch ungeprüften Handlern.
