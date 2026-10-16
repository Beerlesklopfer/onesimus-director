# .catalog — Abkürzung von .catalogs

## Zweck

Wer in der Konsole `.catalog` eingibt, bekommt die Liste der Kataloge, genau wie bei [`.catalogs`](dot-catalogs.md). Der Director akzeptiert wie das Bareos-Original jeden Anfang eines Befehlsnamens und nimmt den ersten passenden Eintrag der Befehlstabelle; `.catalog` ist der Anfang von `.catalogs`. Alle Wörter dahinter werden ignoriert. Der Befehl liest nur die Konfiguration und verändert nichts.

Onesimus enthält außerdem eine eigene Schemaverwaltung unter dem Namen `.catalog` (Unterbefehle `schema`, `version`, `tables`, `describe`, `ddl`, `validate`, `migrate`, `diff`, `codegen` und die abgesicherten Befehle `create`, `drop`, `reset`; Kompatibilitätslog C-040). Sie ist aus einer Konsolensitzung derzeit **nicht erreichbar**, weil die Eingabe vorher als `.catalogs` erkannt wird. Die Absicherung von `drop` und `reset` (ausdrückliche Freigabe in der `Command ACL`, `yes confirm=<datenbank>`, bei vorhandenen Jobs zusätzlich `force`) gilt für jeden Weg, der den Handler doch erreicht.

## Syntax

```text
.catalog [ <beliebige wörter> ]
```

## Argumente

Alle Argumente werden ignoriert; es gilt, was bei [.catalogs](dot-catalogs.md) steht.

## API-Modi

Wie bei [.catalogs](dot-catalogs.md).

## Beispiele

### Kataloge auflisten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .catalog
MyCatalog
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unterbefehle werden ignoriert

Auch `.catalog tables` oder `.catalog reset` liefern nur die Katalogliste und ändern nichts.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .catalog reset
MyCatalog
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

Wie bei [.catalogs](dot-catalogs.md).

## Siehe auch

- [.catalogs](dot-catalogs.md) — der ausgeführte Befehl
- [.sqltables](dot-sqltables.md), [.sqlschema](dot-sqlschema.md) — Tabellen und Spalten des Katalogs (Onesimus-Erweiterungen)
- [.sql](dot-sql.md) — SQL-Abfrage im Katalog
