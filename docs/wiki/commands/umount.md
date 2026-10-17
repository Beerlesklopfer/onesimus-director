# umount — Gerät aushängen (anderer Name für unmount)

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L496) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Umount - for old-time Unix guys, see unmount

**Originalsyntax:**

```text
umount storage=<storage-name> [ drive=<drivenum> ] jobid=<jobid> | job=<job-name> | ujobid=<complete_name>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Paritätsfall nachgewiesen.

- Alias von unmount; kein Paritätsfall (siehe unmount).

**Offene Logeinträge:** `C-050` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:196](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`umount` ist derselbe Befehl wie [unmount](unmount.md), unter dem von Unix gewohnten Namen. Argumente, Dialoge, API-Modi und Meldungen sind identisch; die ausführliche Beschreibung steht bei [unmount](unmount.md).

## Syntax

```text
umount [ storage=<storage> | <storage> ] [ drive=<laufwerk> ] [ alldrives ]
umount jobid=<jobid> | job=<job-name> | ujobid=<eindeutiger-job-name>
```

## Argumente

Wie bei [unmount](unmount.md#argumente).

## Beispiele

### Storage auswählen lassen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> umount
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The defined Storage resources are:
1: File
2: ParityNdmp
3: ParityNdmpNative
Select Storage resource (1-3):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Kein laufender Job

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> umount jobid=999999
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
JobId 999999 is not running.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

Wie bei [unmount](unmount.md#fehlermeldungen).

## Siehe auch

- [unmount](unmount.md) — vollständige Beschreibung
- [mount](mount.md) — Gerät wieder einbinden
