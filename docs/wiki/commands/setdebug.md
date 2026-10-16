# setdebug — Debug-Level setzen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L459) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Sets debug level

**Originalsyntax:**

```text
setdebug level=<nn> trace=0/1 timestamp=0/1 client=<client-name> | dir | storage=<storage-name> | all
```

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 7 von 7 Fällen byte-identisch; Modi: Text, API 1.
- Verglichen sind nur Dialog/Abbruch und `setdebug level=0 dir`; Debug-Pegel an echte FD/SD sind laut Testregeln nicht live erlaubt und nur durch Einheitentests belegt (NDMP: C-050).

**Offene Logeinträge:** `C-050` ([Kompatibilitätslog](../../COMPATIBILITY-LOG.md)).

**Paritätsfälle (7):** `c011-setdebug-cancel`, `c011-setdebug-badlevel-cancel`, `c011-setdebug-daemon-cancel`, `c011-setdebug-daemon-api1-cancel`, `c011-setdebug-daemon-badnumber`, `setdebug-dir`, `setdebug-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/debug.rs:33](../../../crates/onesimus-dird/src/commands/debug.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`setdebug` setzt den Debug-Level des Directors, eines Storage Daemons, eines File Daemons oder aller Daemons auf einmal. Zusätzlich lassen sich die Trace-Datei und Zeitstempel in der Debug-Ausgabe ein- und ausschalten. Der Befehl verändert den laufenden Daemon (nicht die Konfiguration); nach einem Neustart gilt wieder der Startwert.

## Syntax

```text
setdebug [level=<n>] [trace=0|1] [timestamp=0|1] dir | director
setdebug [level=<n>] [trace=0|1] [hangup=<n>] [timestamp=0|1] client[=<client-name>] | fd[=<client-name>]
setdebug [level=<n>] [trace=0|1] [timestamp=0|1] storage[=<storage-name>] | store[=<storage-name>] | sd[=<storage-name>]
setdebug [level=<n>] [trace=0|1] [hangup=<n>] [timestamp=0|1] all
setdebug [level=<n>] [trace=0|1] [hangup=<n>] [timestamp=0|1]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `level=<n>` | Ganzzahl ≥ 0 | Neuer Debug-Level; 0 schaltet die Debug-Ausgabe ab. Fehlt er, fragt der Director danach. |
| `trace=0\|1` | 0 oder 1 | Debug-Ausgabe in eine Trace-Datei im Arbeitsverzeichnis schreiben (1) oder nicht (0). Werte größer 1 gelten als 1. Ohne Angabe bleibt die Einstellung unverändert. |
| `timestamp=0\|1` | 0 oder 1 | Zeitstempel vor jede Debug-Zeile setzen. Ohne Angabe unverändert. |
| `hangup=<n>` | Ganzzahl | Nur für Client und `all`: Testhilfe des File Daemons, die die Verbindung nach `<n>` Dateien abbricht. Ohne Angabe unverändert. |
| `dir`, `director` | – | Ziel ist der Director selbst. |
| `client`, `fd` | optional `=<client-name>` | Ziel ist ein File Daemon. Ohne Wert oder bei unbekanntem Namen erscheint die Auswahlliste der Clients. |
| `storage`, `store`, `sd` | optional `=<storage-name>` | Ziel ist ein Storage Daemon. Ohne Wert oder bei unbekanntem Namen erscheint die Auswahlliste der Storages. |
| `all` | – | Director, alle Storage Daemons und alle File Daemons. |

Ohne Zielangabe fragt der Director, für welchen Daemon der Level gelten soll.

## Dialoge

Fehlt `level=`, fragt der Director `Enter new debug level:`. Erwartet wird eine nicht negative Ganzzahl; andere Eingaben werden abgelehnt und die Frage wiederholt. `.` bricht ab.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdebug
Enter new debug level: 
> x
Expected a positive integer, got: x
Enter new debug level: 
> -1
Expected a positive integer, got: -1
Enter new debug level: 
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Fehlt das Ziel, folgt die Auswahl des Daemons (1 bis 4). Eine Zahl außerhalb des Bereichs wird abgelehnt, `.` bricht ab.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdebug level=0
Available daemons are: 
1: Director
2: Storage
3: Client
4: All
Select daemon type to set debug level (1-4): 
> 5
Please enter a number between 1 and 4
Select daemon type to set debug level (1-4): 
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Bei `client` oder `storage` ohne gültigen Namen erscheint die Ressourcenauswahl. Bricht man sie mit `.` ab, kommt anschließend die Auswahl des Daemons.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdebug level=0 client
The defined Client resources are:
1: bareos-fd
2: parity-ndmp-fd
Select Client (File daemon) resource (1-2): 
> .
Selection aborted, nothing done.
Available daemons are: 
1: Director
2: Storage
3: Client
4: All
Select daemon type to set debug level (1-4): 
> 
```

## API-Modi

Unter `.api 1` kommt dieselbe Textzeile wie im Standardmodus. Unter `.api 2` liefert `setdebug … dir` nur ein leeres `result`. Die Auswahl des Daemons kommt unter `.api 1` als Auswahlliste mit Einzeleinträgen (`Director`, `Storage`, `Client`, `All`), auf die oconsole keinen Eingabeprompt anbietet.

## Beispiele

### Debug-Level des Directors setzen

Der Director bestätigt mit den aktuellen Werten und dem Pfad der Trace-Datei.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdebug level=0 dir
level=0 trace=0 hangup=0 timestamp=0 tracefilename=/home/INTRANET.BERNAU.FAMILY/Joerg.Bernau/Dokumente/baReos-dird/target/parity/conf/work/bareos-dir.trace
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Trace-Datei und Zeitstempel einschalten und wieder aus

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdebug level=50 trace=1 timestamp=1 dir
level=50 trace=1 hangup=0 timestamp=1 tracefilename=/tmp/claude-10001/-home-INTRANET-BERNAU-FAMILY-Joerg-Bernau-Dokumente-baReos-dird/08def236-83d2-48b8-887a-df7f3e834b5d/scratchpad/docs-status/run/conf/work/bareos-dir.trace
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdebug level=0 trace=0 timestamp=0 director
level=0 trace=0 hangup=0 timestamp=0 tracefilename=/tmp/claude-10001/-home-INTRANET-BERNAU-FAMILY-Joerg-Bernau-Dokumente-baReos-dird/08def236-83d2-48b8-887a-df7f3e834b5d/scratchpad/docs-status/run/conf/work/bareos-dir.trace
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Alles im Dialog

Ohne Argumente fragt der Director erst nach dem Level, dann nach dem Daemon.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdebug
Enter new debug level: 
> 0
Available daemons are: 
1: Director
2: Storage
3: Client
4: All
Select daemon type to set debug level (1-4): 
> 1
level=0 trace=0 hangup=0 timestamp=0 tracefilename=/tmp/claude-10001/-home-INTRANET-BERNAU-FAMILY-Joerg-Bernau-Dokumente-baReos-dird/08def236-83d2-48b8-887a-df7f3e834b5d/scratchpad/docs-status/run/conf/work/bareos-dir.trace
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Client

Bei einem unbekannten Namen erscheint ohne Fehlermeldung die Auswahlliste.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdebug level=0 client=nosuch
The defined Client resources are:
1: bareos-fd
2: parity-ndmp-fd
Select Client (File daemon) resource (1-2): 
> .
Selection aborted, nothing done.
Available daemons are: 
1: Director
2: Storage
3: Client
4: All
Select daemon type to set debug level (1-4): 
> 
```

### Storage mit NDMP-Protokoll

Für einen Storage, der nicht das native Bareos-Protokoll spricht, setzt der Director keinen Level.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdebug level=0
Available daemons are: 
1: Director
2: Storage
3: Client
4: All
Select daemon type to set debug level (1-4): 
> 2
The defined Storage resources are:
1: File
2: ParityNdmp
3: ParityNdmpNative
Select Storage resource (1-3): 
> 2
Storage has non-native protocol.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Debug-Level eines File Daemons oder Storage Daemons

Beispielausgabe nicht verfügbar: der Befehl würde den Debug-Level des produktiven File Daemons bzw. Storage Daemons ändern.

### Alle Daemons

Beispielausgabe nicht verfügbar: `setdebug … all` ändert den Debug-Level der produktiven File und Storage Daemons.

### Unter .api 2

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> setdebug level=0 dir
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Eingeschränkte Konsole

Ohne Recht auf `setdebug` lehnt der Director den Befehl ab.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> setdebug level=0 dir
setdebug: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Expected a positive integer, got: <eingabe>` | Bei der Frage nach dem Level wurde keine nicht negative Ganzzahl eingegeben. |
| `Please enter a number between 1 and 4` | Ungültige Nummer bei der Auswahl des Daemons. |
| `Selection aborted, nothing done.` | Auswahl mit `.` abgebrochen. |
| `Storage has non-native protocol.` | Der gewählte Storage ist ein NDMP-Storage; dort lässt sich kein Debug-Level setzen. |
| `setdebug: is an invalid command.` | Die Konsole darf `setdebug` nicht ausführen (Command ACL). |

## Siehe auch

- [trace](trace.md) — Trace-Datei des Directors ein- und ausschalten
- [status](status.md) — Status von Director, Client und Storage, beim Client mit aktuellem Debug-Level
- [setbandwidth](setbandwidth.md) — Bandbreitenlimit für Client, Storage oder laufende Jobs setzen
