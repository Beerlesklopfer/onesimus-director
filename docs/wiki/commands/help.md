# help — Befehle und ihre Argumente anzeigen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L350) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Print help on specific command

**Originalsyntax:**

```text
help add autodisplay automount cancel configure create delete disable enable estimate exit gui label list llist messages memory mount prune purge quit query restore relabel release reload run status setbandwidth setdebug setip show sqlquery time trace truncate unmount umount update use var version wait
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 7 von 7 Fällen byte-identisch; Modi: Text, API 1, Konsolen parity-limited.

**Paritätsfälle (7):** `c028-help`, `c028-help-run`, `help`, `help-run`, `help-configure`, `help-unknown`, `help-limited`.

**Lokaler Handler:** [crates/odir/src/commands/basic.rs:130](../../../crates/odir/src/commands/basic.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`help` listet die Befehle, die die eigene Konsole benutzen darf, mit einer Kurzbeschreibung. `help <befehl>` zeigt zu einem Befehl zusätzlich die Argumente. Punktbefehle (`.api`, `.jobs`, …) erscheinen in der Liste nicht; für sie gibt es [`.help`](dot-help.md). Der Befehl liest nur.

## Syntax

```text
help
help <befehl>
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `<befehl>` | voller Befehlsname, Groß- und Kleinschreibung egal | Beschreibung und Argumente dieses Befehls zeigen. Abkürzungen gelten hier nicht (`help ver` findet nichts). Auch Punktbefehle und Befehle, die die Konsole nicht benutzen darf, werden gefunden. |

Mit mehr als einem Wort hinter `help` zeigt der Director die ganze Liste.

Die Liste ist nach der Befehlsliste der `CommandACL` gefiltert: Eine eingeschränkte Konsole sieht nur ihre erlaubten Befehle. Am Ende steht immer der Hinweis, dass ein einzelner Punkt (`.`) an einer Eingabeaufforderung den Befehl abbricht.

## API-Modi

Unter `.api 1` fallen Kopfzeilen und Schlusshinweis weg, und die Argumente von `help <befehl>` stehen in einer langen Zeile. Unter `.api 2` ist `result` ein Objekt mit einem Eintrag je Befehl; jeder Eintrag hat `command`, `description`, `arguments` (eine Zeile) und `permission` (ob die Konsole den Befehl benutzen darf). Für einen unbekannten Befehl ist das Ergebnis leer (`"result": {}`).

## Beispiele

### Alle Befehle

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> help
  Command            Description
  =======            ===========
  add                Add media to a pool
  autodisplay        Autodisplay console messages
  automount          Automount after label
  cancel             Cancel a job
  configure          Configure director resources
  create             Create DB Pool from resource
  delete             Delete volume, pool or job
  disable            Disable a job/client/schedule
  enable             Enable a job/client/schedule
  estimate           Performs FileSet estimate, listing gives full listing
  exit               Terminate Bconsole session
  export             Export volumes from normal slots to import/export slots
  gui                Switch between interactive (gui off) and non-interactive (gui on) mode
  help               Print help on specific command
  import             Import volumes from import/export slots to normal slots
  label              Label a tape
  list               List objects from catalog
  llist              Full or long list like list command
  messages           Display pending messages
  memory             Print current memory usage
  mount              Mount storage
  move               Move slots in an autochanger
  prune              Prune records from catalog
  purge              Purge records from catalog
  quit               Terminate Bconsole session
  query              Query catalog
  restore            Restore files
  relabel            Relabel a tape
  release            Release storage
  reload             Reload conf file
  rerun              Rerun a job
  resolve            Resolve a hostname
  run                Run a job
  status             Report status
  setbandwidth       Sets bandwidth
  setdebug           Sets debug level
  setdevice          Sets device parameter
  setip              Sets new client address -- if authorized
  show               Show resource records
  sqlquery           Use SQL to query catalog
  time               Print current time
  trace              Turn on/off trace to file
  truncate           Truncate purged volumes
  unmount            Unmount storage
  umount             Umount - for old-time Unix guys, see unmount
  update             Update volume, pool, slots, job or statistics
  use                Use specific catalog
  var                Does variable expansion
  version            Print Director version
  wait               Wait until no jobs are running
  whoami             Print the user name associated with this console

When at a prompt, entering a period (.) cancels the command.

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ein Befehl mit Argumenten

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> help run
  Command            Description
  =======            ===========
  run                Run a job

Arguments:
	job=<job-name> client=<client-name> fileset=<fileset-name>
	level=<level> storage=<storage-name> where=<directory-prefix>
	when=<universal-time-specification> pool=<pool-name>
	pluginoptions=<plugin-options-string> accurate=<yes|no>
	comment=<text> spooldata=<yes|no> priority=<number>
	jobid=<jobid> catalog=<catalog> migrationjob=<job-name>
	backupclient=<client-name> backupformat=<format>
	nextpool=<pool-name> since=<universal-time-specification>
	verifyjob=<job-name> verifylist=<verify-list>
	migrationjob=<complete_name> [ yes ]

When at a prompt, entering a period (.) cancels the command.

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Befehl ohne Argumente

Groß- und Kleinschreibung spielt keine Rolle.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> help VERSION
  Command            Description
  =======            ===========
  version            Print Director version

Arguments:
	

When at a prompt, entering a period (.) cancels the command.

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Hilfe zu `help`

Die Argumentliste von `help` nennt die Befehle, zu denen es ausführliche Hilfe gibt.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> help help
  Command            Description
  =======            ===========
  help               Print help on specific command

Arguments:
	add autodisplay automount cancel configure create
	delete disable enable estimate exit gui label
	list llist messages memory mount prune purge
	quit query restore relabel release reload run
	status setbandwidth setdebug setip show sqlquery
	time trace truncate unmount umount update use
	var version wait

When at a prompt, entering a period (.) cancels the command.

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Befehl

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> help nosuch
  Command            Description
  =======            ===========

Can't find nosuch command.


When at a prompt, entering a period (.) cancels the command.

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Eingeschränkte Konsole

Die Konsole `parity-limited` sieht nur ihre erlaubten Befehle (Paritätsfall `help-limited`).

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> help
  Command            Description
  =======            ===========
  help               Print help on specific command
  list               List objects from catalog
  llist              Full or long list like list command
  status             Report status
  show               Show resource records
  time               Print current time
  version            Print Director version
  whoami             Print the user name associated with this console

When at a prompt, entering a period (.) cancels the command.

parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter `.api 1`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> help run
  run                Run a job

Arguments:
	job=<job-name> client=<client-name> fileset=<fileset-name> level=<level> storage=<storage-name> where=<directory-prefix> when=<universal-time-specification> pool=<pool-name> pluginoptions=<plugin-options-string> accurate=<yes|no> comment=<text> spooldata=<yes|no> priority=<number> jobid=<jobid> catalog=<catalog> migrationjob=<job-name> backupclient=<client-name> backupformat=<format> nextpool=<pool-name> since=<universal-time-specification> verifyjob=<job-name> verifylist=<verify-list> migrationjob=<complete_name> [ yes ]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> help
  add                Add media to a pool
  autodisplay        Autodisplay console messages
  automount          Automount after label
[…]
  wait               Wait until no jobs are running
  whoami             Print the user name associated with this console
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter `.api 2`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> help
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "add": {
      "command": "add",
      "description": "Add media to a pool",
      "arguments": "pool=<pool-name> storage=<storage-name> jobid=<jobid>",
      "permission": true
    },
[…]
    "whoami": {
      "command": "whoami",
      "description": "Print the user name associated with this console",
      "arguments": "",
      "permission": true
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> help nosuch
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {}
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `Can't find <befehl> command.` | Es gibt keinen Befehl mit genau diesem Namen. Den vollen Namen verwenden; die Liste zeigt `help` ohne Argument. |

## Siehe auch

- [.help](dot-help.md) — Befehlsliste im Maschinenformat, auch mit Punktbefehlen
- [whoami](whoami.md) — Name der eigenen Konsole
- [.authorized](dot-authorized.md) — Berechtigung für einen Befehl prüfen
- [Befehlsübersicht](README.md) — alle Befehle nach Themen
