# wait — auf Jobs warten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L523) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Wait until no jobs are running

**Originalsyntax:**

```text
wait jobname=<name> | jobid=<jobid> | ujobid=<complete_name> | mount [timeout=<number>]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 5 von 5 Fällen byte-identisch; Modi: Text, API 1, API 2.
- Verglichen sind nur `wait` ohne laufende Jobs und `wait mount timeout=`; das Warten auf echte Jobenden ist nur durch Einheitentests belegt (C-048).

**Offene Logeinträge:** `C-048` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (5):** `c035-wait-mount-timeout`, `c035-wait-mount-timeout-api1`, `c035-wait-mount-timeout-api2`, `wait-idle`, `wait-jobid`.

**Lokaler Handler:** [crates/odir/src/commands/wait.rs:46](../../../crates/odir/src/commands/wait.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`wait` blockiert die Sitzung, bis Jobs fertig sind. Ohne Argument wartet der Befehl, bis kein Job mehr läuft; mit einer Jobangabe wartet er auf diesen einen Job und meldet danach dessen Endstatus; mit `mount` wartet er, bis ein Job auf ein Volume wartet. Der Befehl verändert nichts und ist vor allem für Skripte gedacht, etwa nach `run … yes` vor der Auswertung.

## Syntax

```text
wait
wait jobid=<jobid> | job=<jobname> | jobname=<jobname> | ujobid=<eindeutiger-jobname>
wait mount [timeout=<sekunden>]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| (keins) | — | Warten, bis kein Job mehr läuft. Läuft keiner, kehrt der Befehl nach etwa 0,2 Sekunden ohne Ausgabe zurück. |
| `jobid=<jobid>` | Zahl | Auf diesen Job warten. Die Zahl muss nicht zu einem laufenden Job gehören; der Status wird dann sofort aus dem Katalog gelesen. |
| `job=<jobname>`, `jobname=<jobname>` | Jobname, Anfang genügt | Auf den laufenden Job warten, dessen Name so beginnt. |
| `ujobid=<name>` | eindeutiger Jobname (`backup-fd.2026-10-13_21.00.00_09`) | Auf diesen laufenden Job warten. |
| `mount` | — | Warten, bis ein laufender Job auf ein Volume oder einen Mount wartet. |
| `timeout=<sekunden>` | Zahl | Nur mit `mount`: höchstens so lange warten, dann `Wait on mount timed out`. |

Ausgewertet wird das erste Argument, das `jobid`, `job`, `jobname`, `ujobid` oder `mount` heißt; alles danach wird ignoriert. Mit einer Jobangabe oder `mount` öffnet der Befehl den Katalog (beim ersten Mal mit den Zeilen `Automatically selected Catalog: …` und `Using Catalog "…"`).

Nach dem Warten auf einen Job meldet der Director `JobId=<n>` und `JobStatus=<text> (<code>)`. Zusätzlich gibt es einen Ausgangswert `ExitStatus` (nur im JSON sichtbar): `0` = OK (Status `T` oder `W`), `1` = Warnung (`e`, nicht fataler Fehler), `2` = kritisch (`E`, `f`, `A`), `3` = unbekannt (alle anderen Codes).

## API-Modi

Unter `.api 0` und `.api 1` kommt der Text wie beschrieben (unter `.api 1` ohne die Katalogzeilen). Unter `.api 2` steht das Ergebnis eines Jobs im Objekt `result.job` mit `jobid`, `jobstatuslong`, `jobstatus` und `exitstatus`. Fehler kommen als JSON-Fehler. Die Zeitüberschreitung bei `mount` ist unter `.api 2` ein leeres Ergebnis (`"result": {}`).

## Beispiele

### Warten, bis kein Job mehr läuft

Läuft kein Job, kehrt der Befehl sofort ohne Ausgabe zurück.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> wait
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Status eines Jobs abfragen

JobId 1 läuft nicht und steht nicht im Katalog; deshalb ist der Status unbekannt. Für einen tatsächlich laufenden Job: Beispielausgabe nicht verfügbar, weil dafür ein echter Job gestartet werden müsste.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> wait jobid=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
JobId=1
JobStatus=Unknown term code (?)
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Status als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> wait jobid=99999
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "job": {
      "jobid": 99999,
      "jobstatuslong": "Unknown term code",
      "jobstatus": "?",
      "exitstatus": 3
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Auf eine Mount-Anforderung warten

Mit `timeout=1` gibt der Director nach einer Sekunde auf.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> wait mount timeout=1
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Wait on mount timed out
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Job nicht gefunden

Jobnamen ohne laufenden Job, `jobid=0`, `jobid=abc`, ein fehlender Wert oder nur `timeout=` ergeben dieselbe Meldung (Paritätsfälle `ds-wait-text`, `c046-wait-*`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> wait job=nosuch
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
ERR: Job was not found
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> wait timeout=5
ERR: Job was not found
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Job nicht gefunden, JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> wait job=nosuch
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {},
      "messages": {
        "error": [
          "ERR: Job was not found\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `ERR: Job was not found` | Kein laufender Job passt zu `job=`/`jobname=`/`ujobid=`, die JobId ist `0` oder keine Zahl, oder es fehlt jede Jobangabe (z.B. nur `timeout=`). Mit [`status director`](status.md) laufende Jobs ansehen. |
| `Wait on mount timed out` | Warnung: Bei `wait mount timeout=…` hat innerhalb der Zeit kein Job auf ein Volume gewartet. |
| `ERR: Can't open db` | Der Katalog ließ sich nicht öffnen. Beispielausgabe nicht verfügbar. |

## Siehe auch

- [status](status.md) — laufende Jobs ansehen
- [run](run.md) — Job starten
- [cancel](cancel.md) — Job abbrechen
- [mount](mount.md) — Volume einlegen, wenn ein Job wartet
- [.jobstatus](dot-jobstatus.md) — Bedeutung der Statuscodes
