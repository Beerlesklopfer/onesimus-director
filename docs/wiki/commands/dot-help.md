# .help — Befehle und Argumente maschinenlesbar auflisten

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L243) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Print parsable information about a command

**Originalsyntax:**

```text
.help [ all ] [ item=cmd ] [ full ]
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 8 von 8 Fällen byte-identisch; Modi: Text, API 1, API 2, Konsolen parity-limited.

**Paritätsfälle (8):** `c020-compact-help-run`, `c028-dot-help`, `dot-help`, `dot-help-all`, `dot-help-item`, `dot-help-full`, `dot-help-limited`, `dot-help-json`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/basic.rs:161](../../../crates/onesimus-dird/src/commands/basic.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.help` liefert die Befehlstabelle des Directors in einer Form, die Programme leicht auswerten können: Name, Kurzbeschreibung und Argumentliste je Befehl. bconsole und oconsole bauen daraus die Tab-Vervollständigung, die WebUI prüft damit, welche Befehle eine Konsole benutzen darf. Für Menschen ist [help](help.md) gedacht. Der Befehl verändert nichts.

Es erscheinen nur Befehle, die die `Command ACL` der Konsole erlaubt. Die Liste zeigt die Befehle des Bareos-Originals; die Onesimus-Erweiterungen (`.sqlcount`, `.ovfs_*` usw.) fehlen darin.

## Syntax

```text
.help [ full ]
.help all
.help item=<befehl>
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| *(keins)*, `full` | | alle Befehle, je Zeile `<name> <beschreibung> -- <argumente>` |
| `all` | | nur die Namen der normalen Befehle (ohne Punktbefehle), einer pro Zeile |
| `item=<befehl>` | Befehlsname, z. B. `list` oder `.api` | nur die Argumentliste dieses Befehls |

Ein unbekannter Befehlsname bei `item=` ergibt eine leere Antwort.

## API-Modi

Unter `.api 0` und `.api 1` kommt derselbe Text. Unter `.api 2` ist `result` ein Objekt, dessen Schlüssel die Befehlsnamen sind; jeder Eintrag hat `command`, `description`, `arguments` und `permission` (darf die Konsole den Befehl benutzen). `.help all` liefert dort ebenfalls diese Objekte (ohne Punktbefehle), `.help item=<befehl>` ein Objekt mit genau einem Schlüssel, ein unbekannter Name `"result": {}`.

## Beispiele

### Alle Befehle

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .help
. no op --
.actiononpurge List possible actions on purge --
.api Switch between different api modes -- [ 0 | 1 | 2 | off | on | json ] [compact=<yes|no>]
.authorized Check for authorization -- job=<job-name> | client=<client-name> | storage=<storage-name> | schedule=<schedule-name> | pool=<pool-name> | cmd=<command> | fileset=<fileset-name> | catalog=<catalog>
.catalogs List all catalog resources --
.clients List all client resources -- [enabled | disabled]
.consoles List all console resources --
.users List all user resources --
.defaults Get default settings -- job=<job-name> | client=<client-name> | storage=<storage-name> | pool=<pool-name>
.filesets List all filesets --
.help Print parsable information about a command -- [ all ] [ item=cmd ] [ full ]
.jobdefs List all job defaults resources --
[…]
version Print Director version --
wait Wait until no jobs are running -- jobname=<name> | jobid=<jobid> | ujobid=<complete_name> | mount [timeout=<number>]
whoami Print the user name associated with this console --
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Nur die Namen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .help all
add
autodisplay
automount
cancel
configure
create
delete
disable
[…]
version
wait
whoami
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Argumente eines Befehls

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .help item=.api
[ 0 | 1 | 2 | off | on | json ] [compact=<yes|no>]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Befehl

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .help item=nosuch
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .help item=run
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "run": {
      "command": "run",
      "description": "Run a job",
      "arguments": "job=<job-name> client=<client-name> fileset=<fileset-name> level=<level> storage=<storage-name> where=<directory-prefix> when=<universal-time-specification> pool=<pool-name> pluginoptions=<plugin-options-string> accurate=<yes|no> comment=<text> spooldata=<yes|no> priority=<number> jobid=<jobid> catalog=<catalog> migrationjob=<job-name> backupclient=<client-name> backupformat=<format> nextpool=<pool-name> since=<universal-time-specification> verifyjob=<job-name> verifylist=<verify-list> migrationjob=<complete_name> [ yes ]",
      "permission": true
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Kompaktes JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api json compact=yes
{"jsonrpc":"2.0","id":null,"result":{"api":2}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .help item=run
{"jsonrpc":"2.0","id":null,"result":{"run":{"command":"run","description":"Run a job","arguments":"job=<job-name> client=<client-name> fileset=<fileset-name> level=<level> storage=<storage-name> where=<directory-prefix> when=<universal-time-specification> pool=<pool-name> pluginoptions=<plugin-options-string> accurate=<yes|no> comment=<text> spooldata=<yes|no> priority=<number> jobid=<jobid> catalog=<catalog> migrationjob=<job-name> backupclient=<client-name> backupformat=<format> nextpool=<pool-name> since=<universal-time-specification> verifyjob=<job-name> verifylist=<verify-list> migrationjob=<complete_name> [ yes ]","permission":true}}}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Alle Befehle als JSON

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .help
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    ".": {
      "command": ".",
      "description": "no op",
      "arguments": "",
      "permission": true
    },
    ".actiononpurge": {
      "command": ".actiononpurge",
      "description": "List possible actions on purge",
      "arguments": "",
      "permission": true
    },
    ".api": {
      "command": ".api",
[…]
      "description": "Print the user name associated with this console",
      "arguments": "",
      "permission": true
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mit eingeschränkter Konsole

Die Konsole `parity-limited` sieht nur ihre freigegebenen Befehle.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .help all
help
list
llist
status
show
time
version
whoami
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.help: is an invalid command.` | Die `Command ACL` der Konsole erlaubt `.help` nicht. |

## Siehe auch

- [help](help.md) — Hilfe für Menschen
- [.api](dot-api.md) — Ausgabemodus umschalten
- [.authorized](dot-authorized.md) — prüfen, ob die Konsole einen Befehl oder eine Ressource benutzen darf
