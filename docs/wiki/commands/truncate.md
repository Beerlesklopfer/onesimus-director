# truncate — Volumes im Status Purged leeren

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L487) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Truncate purged volumes

**Originalsyntax:**

```text
truncate volstatus=Purged [storage=<storage>] [pool=<pool>] [volume=<volume>] [drive=<drivenum>] [yes]
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 8 von 8 Fällen byte-identisch; Modi: Text.
- Truncate über SD-Relabel mit Auswertung von `3000 OK label.` fehlt (C-053); verglichen sind nur Guards und Abbruchpfade.

**Offene Logeinträge:** `C-053` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (8):** `c012-truncate-yes-only`, `c012-truncate-bad-volstatus`, `c012-truncate-unknown-arg`, `c012-truncate-bad-drive`, `truncate-noargs`, `w011-truncate-storage-cancel`, `w011-truncate-pool-cancel`, `w011-truncate-confirm-no`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:1360](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`truncate` gibt den Platz von Volumes frei, deren Jobs bereits entfernt wurden (Status `Purged`). Der Director listet die passenden Volumes, fragt nach und lässt sie dann vom Storage Daemon neu beschriften; dabei wird die Volume-Datei bzw. das Band auf die Kennung gekürzt. Im Katalog bleiben die Volumes erhalten. Der Befehl wirkt nur auf Storages mit nativem Protokoll (nicht NDMP) und nur auf Volumes eines einzigen Storages pro Aufruf.

## Syntax

```text
truncate volstatus=Purged [storage=<storage>] [pool=<pool>] [volume=<volume>] [drive=<n>] [yes]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `volstatus=Purged` | nur `Purged` | Pflicht. |
| `storage=<storage>` | Storage | Nur Volumes dieses Storages; unbekannter Name: Auswahl. |
| `pool=<pool>` | Pool | Nur Volumes dieses Pools; unbekannter Name: Auswahl. |
| `volume=<volume>` | Volume-Name | Nur dieses Volume. |
| `drive=<n>` | Laufwerksnummer ab 0 | Laufwerk im Storage Daemon. |
| `yes` | – | Ohne Rückfrage ausführen. |

Andere Schlüsselwörter (auch `client=` und `catalog=`) beendet der Director mit `Invalid parameter.` und der Kurzhilfe.

## Dialoge

Nach der Liste der Volumes fragt der Director `Truncate listed volumes (yes/no)?`. Nur `yes` führt die Aktion aus; `no` beendet ohne Meldung. Findet er keine passenden Volumes, meldet er `No results to list.` und fragt nicht. Unbekannte Pools oder Storages führen zu einer Auswahlliste, `.` bricht ab.

## API-Modi

Text in allen Modi gleich. Unter `.api 1` und `.api 2` wird nicht nachgefragt; ohne `yes` geschieht dann nichts.

## Beispiele

### Volumes auflisten, Rückfrage ablehnen

Das Volume wird zuerst mit [purge](purge.md) geleert, dann zum Truncate angeboten.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> purge volume=Full-47321

This command can be DANGEROUS!!!

It purges (deletes) all Files from a Job,
JobId, Client or Volume; or it purges (deletes)
all Jobs from a Client or Volume without regard
to retention periods. Normally you should use the
PRUNE command, which respects retention periods.
This command requires full access to all resources.
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Found 15 Jobs for media "Full-47321" in catalog.
Purge (yes/no)?
> yes
15 Jobss on Volume "Full-47321" purged from catalog.
There are no more Jobs associated with Volume "Full-47321". Marking it purged.

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> truncate volstatus=Purged volume=Full-47321
Using Catalog "MyCatalog"
+---------+------------+------+---------+-----------+---------------------+----------+-------------+-----------+---------------+---------+
| mediaid | volumename | pool | storage | mediatype | lastwritten         | volfiles | volbytes    | volstatus | actiononpurge | comment |
+---------+------------+------+---------+-----------+---------------------+----------+-------------+-----------+---------------+---------+
|  47,321 | Full-47321 | Full | File    | File      | 2026-07-10 21:10:05 |        0 | 158,201,165 | Purged    |             0 |         |
+---------+------------+------+---------+-----------+---------------------+----------+-------------+-----------+---------------+---------+
Truncate listed volumes (yes/no)?
> no
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nach Pool und Storage auswählen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> truncate volstatus=Purged pool=Full storage=File
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
+---------+------------+------+---------+-----------+---------------------+----------+-------------+-----------+---------------+---------+
| mediaid | volumename | pool | storage | mediatype | lastwritten         | volfiles | volbytes    | volstatus | actiononpurge | comment |
+---------+------------+------+---------+-----------+---------------------+----------+-------------+-----------+---------------+---------+
|  47,321 | Full-47321 | Full | File    | File      | 2026-07-10 21:10:05 |        0 | 158,201,165 | Purged    |             0 |         |
+---------+------------+------+---------+-----------+---------------------+----------+-------------+-----------+---------------+---------+
Truncate listed volumes (yes/no)?
> no
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Beispielausgabe nicht verfügbar: mit `yes` würde der echte Storage Daemon das Volume neu beschriften.

### Keine passenden Volumes

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> truncate volstatus=Purged volume=c053-nosuchvolume
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
No results to list.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Pool oder Storage

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> truncate volstatus=Purged pool=nosuch
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Could not find Pool "nosuch": ERR=Pool record not found in Catalog.
Defined Pools:
1: Incremental
2: Full
3: Differential
4: Scratch
Select the Pool (1-4):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> truncate volstatus=Purged storage=nosuch
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Could not find Storage "nosuch": ERR=Defined Storages:
1: ParityNdmpNative
2: ParityNdmp
3: File
Select the Storage (1-3):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehlerhafte Aufrufe

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> truncate
missing parameter
USAGE: truncate volstatus=Purged [storage=<storage>] [pool=<pool>] [volume=<volume>] [drive=<drivenum>] [yes]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> truncate volstatus=Full
Invalid parameter. 'volstatus' must be 'Purged'.
USAGE: truncate volstatus=Purged [storage=<storage>] [pool=<pool>] [volume=<volume>] [drive=<drivenum>] [yes]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> truncate volstatus=Purged drive=x
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Drive number must be integer but was : x

USAGE: truncate volstatus=Purged [storage=<storage>] [pool=<pool>] [volume=<volume>] [drive=<drivenum>] [yes]
Invalid parameter.
USAGE: truncate volstatus=Purged [storage=<storage>] [pool=<pool>] [volume=<volume>] [drive=<drivenum>] [yes]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `missing parameter` | Aufruf ohne Argumente. |
| `required parameter 'volstatus' missing` | `volstatus=` fehlt oder hat keinen Wert. |
| `Invalid parameter. 'volstatus' must be 'Purged'.` | Anderer Status als `Purged`. |
| `Drive number must be integer but was : <wert>` | `drive=` ist keine Zahl ohne Vorzeichen. |
| `Invalid parameter.` | Unbekanntes oder unzulässiges Schlüsselwort. |
| `No results to list.` | Kein Volume erfüllt die Bedingungen. |
| `Could not find Pool "<name>": ERR=Pool record not found in Catalog.` | Unbekannter Pool; es folgt die Pool-Auswahl. |
| `Could not find Storage "<name>": ERR=…` | Unbekannter Storage; es folgt die Storage-Auswahl. |
| `Selected volumes are from different storages. This is not supported. Please choose only volumes from a single storage.` | Die Liste umfasst mehrere Storages; mit `storage=` einschränken. |
| `Storage uses a non-native protocol. Truncate is only supported for native protocols.` | Der Storage spricht NDMP. |

## Siehe auch

- [purge](purge.md) — Jobs eines Volumes entfernen (Status Purged)
- [prune](prune.md) — abgelaufene Einträge entfernen
- [list](list.md) — Volumes mit Status anzeigen (`list volumes`)
- [label](label.md) — Volumes beschriften
