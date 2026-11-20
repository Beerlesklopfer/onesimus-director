# configure — Ressourcen zur Laufzeit anlegen und exportieren

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L319) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Configure director resources

**Dynamische Syntax:** `add <resource-type> <resource-directives> | export client=<client>`. Der [Originalgenerator](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/dird_conf.cc#L918) leitet Pflichtfelder, optionale Felder und Typen aus Ressourcendirektiven ab.

Ressourcentypen für `add`: `client`, `jobdefs`, `job`, `storage`, `catalog`, `schedule`, `fileset`, `pool`, `messages`, `counter`, `profile`, `console`, `user`.

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 18.10.2026 (`development` `579da25`): 44 von 44 Fällen byte-identisch; Modi: Text, API 1, API 2.
- C-019, C-062, C-075 behoben; auch die Onesimus-Erweiterungen `configure update/delete` prüfen seit 20.11.2026 (`c116264`) die Ressourcen-ACL (Hinweis C-062).
- `configure add` ist für Ressourcen ohne Katalogbezug verglichen, die Wirkung über `show` im selben Fall; `reload` danach ist nicht abgedeckt.

**Paritätsfälle (44):** `c019-configure-bare`, `c019-configure-add-only`, `c019-configure-export-only`, `c019-configure-invalid-sub`, `c019-export-no-client-arg`, `c019-export-unknown-client`, `c019-export-twice`, `c019-add-invalid-type`, `c019-add-director`, `c019-add-device`, `c019-add-unknown-directive`, `c019-add-forbidden-char`, `c019-add-missing-name`, `c019-add-missing-value`, `c019-add-missing-required`, `c019-add-console`, `c019-add-console-type-name`, `c019-add-duplicate-existing`, `c019-add-profile`, `c019-add-schedule`, `c019-add-schedule-invalid`, `c019-add-messages`, `c019-add-invalid-name`, `c019-add-unknown-profile`, `c019-add-empty-password`, `c019-add-counter-invalid`, `c019-add-user`, `c019-add-fileset`, `c019-add-case-keys`, `c046-configure-noarg-api2`, `c046-configure-export-client-api2`, `c046-configure-export-nosuch-api2`, `c046-configure-export-noarg-api2`, `c046-configure-add-noarg-api2`, `c046-configure-add-incomplete-api2`, `c046-configure-add-exists-api2`, `c046-configure-add-director-api2`, `c046-configure-add-nosuch-api2`, `c046-configure-badsub-api2`, `c046-configure-export-novalue-api2`, `c046-configure-noarg-api1`, `c047-tree-prefix-case`, `configure-export`, `configure-bad`.

**Lokaler Handler:** [crates/odir/src/commands/configure.rs:53](../../../crates/odir/src/commands/configure.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`configure` legt zur Laufzeit neue Ressourcen in der Director-Konfiguration an (`configure add`) oder erzeugt für einen Client die passende Director-Ressource des File Daemons (`configure export`). Der Befehl verändert die Konfiguration: `add` schreibt eine neue Datei `bareos-dir.d/<typ>/<name>.conf` und lädt die Ressource sofort in den laufenden Director, ohne `reload`. `export` schreibt eine Datei unter `bareos-dir-export/client/<client>/bareos-fd.d/director/`, die man auf den Client kopiert. Vorhandene Ressourcen ändert oder löscht das Bareos-`configure` nicht; dafür bearbeitet man die Dateien und ruft [reload](reload.md) auf. odir hat dafür die Erweiterungen `configure update` und `configure delete`, beide über das Konfigurations-CRUD ([unten](#configure-update--ressource-ändern-onesimus-erweiterung)).

## Syntax

```text
configure add <ressourcentyp> name=<name> [<direktive>=<wert> …]
configure add <ressourcentyp>=<name> [<direktive>=<wert> …]
configure export client=<client>
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `add` | – | Neue Ressource anlegen. |
| `<ressourcentyp>` | `client`, `jobdefs`, `job`, `storage`, `catalog`, `schedule`, `fileset`, `pool`, `messages`, `counter`, `profile`, `console`, `user` | Art der neuen Ressource, Groß-/Kleinschreibung egal. `director` ist nicht erlaubt (es gibt nur einen Director), `device` gehört zum Storage Daemon und wird abgelehnt. |
| `name=<name>` | Ressourcenname | Pflicht für jeden Typ. Statt `client name=x` geht auch `client=x`. |
| `<direktive>=<wert>` | je nach Typ | Jede Direktive des Ressourcentyps, geschrieben ohne Leerzeichen und klein (`maximumvolumejobs=1` für `Maximum Volume Jobs`). Die vollständige Liste je Typ mit Werttyp zeigt `help configure`. Eine Direktive darf mehrfach vorkommen, wenn die Ressource das erlaubt (zum Beispiel `run=` bei `schedule`). Werte mit Leerzeichen stehen in Anführungszeichen. Die Zeichen `@` und `;` sind in Werten verboten. |
| `export` | – | Director-Ressource für den File Daemon eines Clients erzeugen. |
| `client=<client>` | Name eines konfigurierten Clients | Client, für den exportiert wird. |

Pflichtdirektiven laut `help configure` (alle anderen sind optional):

| Typ | Pflicht |
|---|---|
| `client` | `name`, `address`, `password` |
| `jobdefs`, `job` | `name`, `type`, `messages`, `pool` (bei `job` dürfen sie aus `jobdefs=` kommen) |
| `storage` | `name`, `address`, `password`, `device`, `mediatype` |
| `catalog` | `name`, `dbname`, `dbuser` |
| `console` | `name`, `password` |
| `schedule`, `fileset`, `pool`, `messages`, `counter`, `profile`, `user` | `name` |

Beim Anlegen eines Clients exportiert `configure add client` automatisch auch dessen File-Daemon-Ressource, wie `configure export`.

Die Konfiguration muss in Verzeichnisform vorliegen (`bareos-dir.d/<typ>/*.conf`), und das Verzeichnis für den Typ muss bereits existieren; `configure` legt es nicht an.

## configure add console — enge Form (Onesimus-Abweichung)

Console, Profile und User tragen die ACLs der äußeren Stufe ([ACL-ZWEISTUFIG.md](../../plans/ACL-ZWEISTUFIG.md)). odir lehnt deshalb `configure add profile` und `configure add user` immer ab (F-1, 19.11.2026) und nimmt `configure add console` seit 20.11.2026 nur in dieser Form an:

```text
configure add console name=<name> password=<passwort> profile=<P1> [profile=<P2> …] [description=<text>]
```

| Regel | Ablehnung (`configure add console: refused, …`) |
|---|---|
| Erlaubt sind genau `name`, `password`, ein oder mehrere `profile` und optional `description`; Schlüssel wie Bareos-Schlüsselwörter (Groß-/Kleinschreibung, Leerzeichen, `_` egal). `profile=a,b` zählt wie `profile=a profile=b`. | jede `…acl`-Direktive (`directive "jobacl" not allowed: a new console has no own ACL lines …`), jede andere Direktive (`tlsenable`, `usepamauthentication`, …: `only name, password, profile and description`) |
| Pflicht: Name, nicht leeres Passwort, mindestens ein Profil. | `missing name=<name>`, `missing password=<password>`, `give at least one profile=<profile>` |
| Profile müssen existieren. | `unknown Profile "<p>"` |
| Name: gültiger Ressourcenname, noch keine Console, nicht `*UserAgent*`; die Kurzform `console=<name>` gibt es hier nicht. | `invalid name: …`, `Resource "Console" with name "<n>" already exists`, `the name *UserAgent* is reserved …`, `give the name as name=<name>` |
| Werte ohne Steuerzeichen, `"`, `\`, `;`, `@` (sie könnten den Wert in der Datei beenden). | `directive "<k>": control characters, … are not allowed` |
| **Profile ≤ eigene Rechte:** Die Profile dürfen nicht mehr erlauben als die anlegende Console selbst (Regel unten). | `the profiles grant <Typ>Acl "<name>", which Console "<c>" may not use` bzw. `… names beyond a fixed list (*all* or a pattern) …` |
| Eine gesperrte Console (Katalog-ACL nicht lesbar, F-4) legt nichts an. | `Console "<c>" is locked (catalog ACL not readable)` |
| Profile mit `.acl` nur, wenn die anlegende Console keine Katalog-ACL-Einträge hat. | `the profiles grant .acl, and Console "<c>" has catalog ACL entries the new console could remove` |

Die neue Console hat keine eigenen ACL-Zeilen und keine Katalog-Einträge; ihre effektive ACL ist die Datei-ACL ihrer Profile (erster Treffer über die Profile in der angegebenen Reihenfolge). Weiter einschränken kann man sie danach mit [.acl set](dot-acl.md). Geschrieben wird wie bei jedem `configure add` (`bareos-dir.d/console/<name>.conf`, sofort im laufenden Director, ohne `reload`); zusätzlich prüft odir nach dem Einlesen, dass die Console genau die verlangten Direktiven und Profile hat, sonst wird Datei und Ressource wieder entfernt.

**Prüfung „Profile ≤ eigene Rechte“** (`acl::grant_error`), je ACL-Typ, gegen die effektiven Rechte der anlegenden Console (Datei-ACL ∩ Katalog-ACL, dieselbe Auswertung wie bei jeder Berechtigungsprüfung):

- **CommandAcl:** für jeden Befehlsnamen, den eine Prüfung treffen kann (Bareos-Befehlstabelle, registrierte Befehle und Aliasse): erlauben die Profile ihn, muss ihn auch die anlegende Console dürfen. Punktbefehle wie `.api` (in Bareos Regexe) werden so genau entschieden.
- **Alle anderen Typen** (Job, Client, Storage, Schedule, Pool, FileSet, Catalog, Where, PluginOptions; auch künftige Namen zählen): Listen aus Namen, `!`-Einträgen und `*all*` werden genau verglichen. Regexe machen den Vergleich vorsichtig: ein erlaubender Regex in einem Profil zählt wie `*all*`, ein `!`-Regex der anlegenden Console wie `!*all*`, ein erlaubender Regex der anlegenden Console wird nicht mitgezählt. Im Zweifel wird abgelehnt.
- `!`-Einträge in den Profilen schränken nur ein; ein Typ ohne Einträge erlaubt nichts.

Unter `.api 2` steht der Erfolg wie bei jedem `configure add` in `result.configure.add` (`resource`, `name`, `filename`, `content`), eine Ablehnung als JSON-RPC-Fehler mit `error.data.messages.error[]` und `result.configure` leer. Das Passwort steht nie in der Antwort: Text und `content` zeigen `Password = "***"`; in Audit-Zeile und gRPC-Antwortfeld `command` ist `password=` maskiert.

```text
admin@bareos-dir> configure add console name=gui-anna password=[…] profile=operator description="GUI Anna"
Created resource config file "/etc/bareos/bareos-dir.d/console/gui-anna.conf":
Console {
  Name = "gui-anna"
  Password = "***"
  Description = "GUI Anna"
  Profile = operator
}
pdc-mcp@bareos-dir> configure add console name=x password=[…] profile=webui-admin
configure add console: refused, the profiles grant CommandAcl ".acl", which Console "pdc-mcp" may not use.
```

`configure delete console` bleibt abgelehnt; Consoles löscht man im Editor.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Textausgabe. Unter `.api 2` steht das Ergebnis in `result.configure.add` mit den Feldern `resource`, `name`, `filename` und `content` (der geschriebene Ressourcentext) beziehungsweise in `result.configure.export` mit `clientname`, `component`, `resource`, `name`, `filename` und `content`. Fehler kommen als JSON-RPC-Fehler `"message": "failed"`, der Meldungstext steht in `error.data.messages.error`.

## Beispiele

Die Dateipfade in den Beispielen sind auf das Standardverzeichnis `/etc/bareos` umgeschrieben; in den Paritätsläufen liegt die Konfiguration in einem Arbeitsverzeichnis. Passwörter sind durch `[…]` ersetzt.

### Console anlegen

In odir abgelehnt (ACL-Direktiven, siehe [enge Form](#configure-add-console--enge-form-onesimus-abweichung)); das Beispiel zeigt das Bareos-Original. Legt eine Console mit Beschreibung und ACLs an und zeigt sie danach mit `show` (Paritätsfall `c019-add-console`). `show` gibt das Passwort als MD5-Wert aus.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add console name=c019e password=[…] description="Parity console" jobacl=*all* commandacl=status,show
Created resource config file "/etc/bareos/bareos-dir.d/console/c019e.conf":
Console {
  Name = "c019e"
  Password = "[…]"
  Description = "Parity console"
  JobAcl = *all*
  CommandAcl = status,show
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show console=c019e
Console {
  Name = "c019e"
  Description = "Parity console"
  Password = "[…]"
  JobAcl = "*all*"
  CommandAcl = "status", "show"
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Console in Kurzform anlegen

`console=<name>` ersetzt `console name=<name>` (Paritätsfall `c019-add-console-type-name`). In odir abgelehnt (kein Profil, Kurzform); Bareos-Original.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add console=c019f password=[…]
Created resource config file "/etc/bareos/bareos-dir.d/console/c019f.conf":
Console {
  Name = "c019f"
  Password = "[…]"
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Client anlegen

Beim Client wird zuerst die File-Daemon-Ressource exportiert, dann die Client-Ressource angelegt (Paritätsfall `dcfg-add-client`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add client name=dcfg-client address=192.0.2.10 password=[…] description="Testclient"
Exported resource file "/etc/bareos/bareos-dir-export/client/dcfg-client/bareos-fd.d/director/bareos-dir.conf":
Director {
  Name = "bareos-dir"
  Password = "[…]"
}
Created resource config file "/etc/bareos/bareos-dir.d/client/dcfg-client.conf":
Client {
  Name = "dcfg-client"
  Address = "192.0.2.10"
  Password = "[…]"
  Description = "Testclient"
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show client=dcfg-client
Client {
  Name = "dcfg-client"
  Description = "Testclient"
  Address = "192.0.2.10"
  Password = "[…]"
  Catalog = "MyCatalog"
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### JobDefs anlegen

Paritätsfall `dcfg-add-jobdefs`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add jobdefs name=dcfg-jobdefs type=backup messages=Standard pool=Full client=bareos-fd fileset=SelfTest
Created resource config file "/etc/bareos/bareos-dir.d/jobdefs/dcfg-jobdefs.conf":
JobDefs {
  Name = "dcfg-jobdefs"
  Type = backup
  Messages = "Standard"
  Pool = "Full"
  Client = "bareos-fd"
  FileSet = "SelfTest"
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Job auf Basis von JobDefs anlegen

Fehlende Pflichtdirektiven kommen aus der JobDefs-Ressource (Paritätsfall `dcfg-add-job`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add job name=dcfg-job jobdefs=DefaultJob client=bareos-fd
Created resource config file "/etc/bareos/bareos-dir.d/job/dcfg-job.conf":
Job {
  Name = "dcfg-job"
  JobDefs = "DefaultJob"
  Client = "bareos-fd"
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show job=dcfg-job
Job {
  Name = "dcfg-job"
  VirtualFullBackupPool = ""
  NextPool = ""
  Client = "bareos-fd"
  JobToVerify = ""
  Catalog = ""
  JobDefs = "DefaultJob"
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Job vollständig anlegen

Paritätsfall `dcfg-add-job-full`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add job name=dcfg-job2 type=backup client=bareos-fd fileset=SelfTest storage=File pool=Full messages=Standard
Created resource config file "/etc/bareos/bareos-dir.d/job/dcfg-job2.conf":
Job {
  Name = "dcfg-job2"
  Type = backup
  Client = "bareos-fd"
  FileSet = "SelfTest"
  Storage = File
  Pool = "Full"
  Messages = "Standard"
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Storage anlegen

Paritätsfall `dcfg-add-storage`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add storage name=dcfg-storage address=localhost password=[…] device=FileStorage mediatype=File
Created resource config file "/etc/bareos/bareos-dir.d/storage/dcfg-storage.conf":
Storage {
  Name = "dcfg-storage"
  Address = "localhost"
  Password = "[…]"
  Device = FileStorage
  MediaType = "File"
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Pool anlegen

Der Pool steht danach in der laufenden Konfiguration; den Katalogeintrag des Pools erzeugt bei Bedarf [create](create.md) (Paritätsfall `dcfg-add-pool`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add pool name=dcfg-pool pooltype=Backup maximumvolumejobs=1 labelformat=dcfg-
Created resource config file "/etc/bareos/bareos-dir.d/pool/dcfg-pool.conf":
Pool {
  Name = "dcfg-pool"
  PoolType = Backup
  MaximumVolumeJobs = 1
  LabelFormat = "dcfg-"
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show pool=dcfg-pool
Pool {
  Name = "dcfg-pool"
  LabelFormat = "dcfg-"
  LabelType = "bareos"
  ActionOnPurge = None
  MaximumVolumeJobs = 1
  NextPool = ""
  RecyclePool = ""
  ScratchPool = ""
  Catalog = ""
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Catalog anlegen

`configure add catalog` prüft die Datenbankverbindung nicht. Ist der neue Katalog nicht erreichbar, scheitert erst der nächste [reload](reload.md) im Original (Paritätsfall `dcfg-add-catalog`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add catalog name=dcfg-catalog dbname=bareos dbuser=bareos
Created resource config file "/etc/bareos/bareos-dir.d/catalog/dcfg-catalog.conf":
Catalog {
  Name = "dcfg-catalog"
  DbName = "bareos"
  DbUser = "bareos"
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show catalog=dcfg-catalog
Catalog {
  Name = "dcfg-catalog"
  DbUser = "bareos"
  DbName = "bareos"
  MultipleConnections = No
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Schedule mit mehreren Run-Zeilen anlegen

Paritätsfall `dcfg-add-schedule`. `show` gibt die Zeitangaben normalisiert aus.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add schedule name=dcfg-schedule run="Full 1st sun at 23:05" run="Incremental mon-sat at 23:05"
Created resource config file "/etc/bareos/bareos-dir.d/schedule/dcfg-schedule.conf":
Schedule {
  Name = "dcfg-schedule"
  Run = Full 1st sun at 23:05
  Run = Incremental mon-sat at 23:05
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show schedule=dcfg-schedule
Schedule {
  Name = "dcfg-schedule"
  Run = Full 1st Sun at 23:05
  Run = Incremental Mon-Sat at 23:05
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### FileSet anlegen

Paritätsfall `c019-add-fileset`. Die Include-Blöcke eines FileSets lassen sich mit `configure` nicht angeben; sie werden in der Datei ergänzt.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add fileset name=c019r description=x
Created resource config file "/etc/bareos/bareos-dir.d/fileset/c019r.conf":
FileSet {
  Name = "c019r"
  Description = "x"
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Messages anlegen

Paritätsfall `c019-add-messages`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add messages name=c019j description=test
Created resource config file "/etc/bareos/bareos-dir.d/messages/c019j.conf":
Messages {
  Name = "c019j"
  Description = "test"
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Counter anlegen

Voraussetzung ist das Verzeichnis `bareos-dir.d/counter/` (Paritätsfall `dcfg-add-counter`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add counter name=dcfg-counter minimum=1 maximum=100
Created resource config file "/etc/bareos/bareos-dir.d/counter/dcfg-counter.conf":
Counter {
  Name = "dcfg-counter"
  Minimum = 1
  Maximum = 100
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Profile anlegen

Schlüsselwörter dürfen groß geschrieben werden; in der Datei stehen sie in der kanonischen Schreibweise (Paritätsfall `c019-add-case-keys`). In odir abgelehnt (F-1, Profile nur im Editor); Bareos-Original.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add Profile Name=c019s JobACL=a
Created resource config file "/etc/bareos/bareos-dir.d/profile/c019s.conf":
Profile {
  Name = "c019s"
  JobAcl = a
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### User anlegen

Voraussetzung ist das Verzeichnis `bareos-dir.d/user/` (Paritätsfall `dcfg-add-user`). In odir abgelehnt (F-1, User nur im Editor); Bareos-Original.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add user name=dcfg-user profile=operator
Created resource config file "/etc/bareos/bareos-dir.d/user/dcfg-user.conf":
User {
  Name = "dcfg-user"
  Profile = operator
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show user=dcfg-user
User {
  Name = "dcfg-user"
  Profile = "operator"
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ressource anlegen unter `.api 2`

Paritätsfall `dcfg-add-console-api2` (Bareos-Original). odir nimmt diesen Aufruf in der [engen Form](#configure-add-console--enge-form-onesimus-abweichung) an, `content` zeigt dort `Password = "***"`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add console name=dcfg-con2 password=[…] profile=operator
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "configure": {
      "add": {
        "resource": "Console",
        "name": "dcfg-con2",
        "filename": "/etc/bareos/bareos-dir.d/console/dcfg-con2.conf",
        "content": "Console {\n  Name = \"dcfg-con2\"\n  Password = \"[…]\"\n  Profile = operator\n}\n"
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### File-Daemon-Ressource exportieren

Paritätsfall `configure-export`. Die erzeugte Datei kommt auf dem Client nach `/etc/bareos/bareos-fd.d/director/`. Ein zweiter Aufruf überschreibt die Datei mit demselben Inhalt (`c019-export-twice`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure export client=bareos-fd
Exported resource file "/etc/bareos/bareos-dir-export/client/bareos-fd/bareos-fd.d/director/bareos-dir.conf":
Director {
  Name = "bareos-dir"
  Password = "[…]"
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Export unter `.api 2`

Paritätsfall `c046-configure-export-client-api2`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure export client=bareos-fd
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "configure": {
      "export": {
        "clientname": "bareos-fd",
        "component": "bareos-fd",
        "resource": "director",
        "name": "bareos-dir",
        "filename": "/etc/bareos/bareos-dir-export/client/bareos-fd/bareos-fd.d/director/bareos-dir.conf",
        "content": "Director {\n  Name = \"bareos-dir\"\n  Password = \"[…]\"\n}\n"
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Aufruf ohne Unterbefehl

`configure`, `configure add` und `configure export` ohne weiteres Wort zeigen die Kurzhilfe (Paritätsfälle `c019-configure-bare`, `c019-configure-add-only`, `c019-configure-export-only`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure
usage:
  configure add <resourcetype> <key1>=<value1> ...
  configure export client=<clientname>
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehler unter `.api 2`

Paritätsfall `c046-configure-add-exists-api2`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add client name=bareos-fd address=localhost password=[…]
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {
        "configure": {}
      },
      "messages": {
        "error": [
          "Resource \"Client\" with name \"bareos-fd\" already exists.\n"
        ]
      }
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehlende Pflichtdirektive bei einem Job

Paritätsfall `dcfg-add-job-missing`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add job name=dcfg-job3 client=bareos-fd
failed to create config resource "dcfg-job3"
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ungültiger Wert

Paritätsfall `c019-add-schedule-invalid`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add schedule name=c019i enabled=maybe
configure error: Expect YES, NO, TRUE, or FALSE, got: maybe
configure error: Parser Error occurred.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole ohne Berechtigung

Eine Konsole, deren `CommandACL` `configure` nicht enthält, bekommt die allgemeine Ablehnung (Paritätsfall `dcfg-add-limited`).

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> configure add console name=dcfg-x password=[…]
configure: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## configure update — Ressource ändern (Onesimus-Erweiterung)

```text
configure update <typ>=<name> <direktive>=<wert> [<direktive>=<wert> …]
```

Ändert Direktiven einer vorhandenen Ressource in ihrer Datei und lädt die Konfiguration neu. Der Befehl geht durch das Konfigurations-CRUD ([.config](dot-config.md), [CONFIG-CRUD-GRPC.md](../../plans/CONFIG-CRUD-GRPC.md)): Er liest den Rohtext der Ressource mit seiner SHA-256, ändert darin nur die genannten Direktiven und schreibt ihn mit dieser Prüfsumme zurück. Kommentare, Reihenfolge, Einrückung, Zeilenenden und alle anderen Direktiven bleiben Byte für Byte; die geänderte Gesamtkonfiguration wird vorher wie `odir -t` geprüft, geschrieben wird atomar, ein fehlgeschlagener Reload stellt den alten Stand wieder her. Die Ressourcen-ACL der Konsole gilt (wie `show`).

| Argument | Bedeutung |
|---|---|
| `<typ>=<name>` | Erstes Argument mit Wert: `client`, `job`, `jobdefs`, `schedule` oder `fileset`. `pool`, `storage` und `console` werden abgelehnt (noch nicht im CRUD). |
| `<direktive>=<wert>` | Schlüssel wie Bareos-Schlüsselwörter: Groß-/Kleinschreibung, Leerzeichen und `_` egal (`Description`, `description`; `fileretention`, `file_retention`). Nur Direktiven der Ressource selbst; Direktiven in verschachtelten Blöcken (Job `RunScript { }`, FileSet `Include { }`/`Options { }`) werden abgelehnt. Eine vorhandene Direktive bekommt den neuen Wert, eine fehlende kommt als neue Zeile vor die schließende Klammer (eingerückt wie die übrigen). Unbekannte Schlüssel werden eingetragen und von der Prüfung abgelehnt. `name` ist nicht änderbar. |
| mehrfach derselbe Schlüssel | Ersetzt **alle** vorhandenen Zeilen der Direktive durch die angegebenen Werte, an der Stelle der ersten (Run-Zeilen eines Schedules). |
| `<direktive>=` | Ohne Wert: entfernt alle Zeilen der Direktive (bei `;`-getrennten Direktiven auf einer Zeile nur diese). |
| `newclient=`, `newstorage=`, `newpool=` | Frühere Schreibweisen bei `job`: wie `client=`, `storage=`, `pool=`. |

Werte ohne Leerzeichen und ohne `{ } = ; # " , \ @` werden ohne Anführungszeichen geschrieben, alle anderen in Anführungszeichen. Ausnahme: `Run` eines Schedules steht immer ohne Anführungszeichen (wie in Bareos) und wird wie von StoreRun geprüft; `Run` eines Jobs ist eine Zeichenkette und wird normal gesetzt. Steuerzeichen und `"` sind in Werten verboten.

Ausgabe: `<Typ> '<name>' updated: <direktive>=<wert>, …` (Passwörter als `password=***`); ändert sich nichts, `<Typ> '<name>': no change (…)`. Über gRPC entspricht das `ConfigureUpdate`, über MCP dem Werkzeug `configure_update`.

### Beschreibung und Run-Zeilen eines Schedules ändern

```text
*configure update schedule=WeeklyCycle description="Weekly full, daily incremental"
Schedule 'WeeklyCycle' updated: description=Weekly full, daily incremental
*configure update schedule=WeeklyCycle run="Full sun at 23:30" run="Incremental mon-sat at 23:30"
Schedule 'WeeklyCycle' updated: run=Full sun at 23:30, run=Incremental mon-sat at 23:30
```

Aus

```text
Schedule {
  Name = "WeeklyCycle"
  Description = "old"  # shown in the GUI
  Run = Full 1st sat at 21:00
  Run = Differential 2nd-5th sat at 21:00
}
```

wird

```text
Schedule {
  Name = "WeeklyCycle"
  Description = "Weekly full, daily incremental"  # shown in the GUI
  Run = Full sun at 23:30
  Run = Incremental mon-sat at 23:30
}
```

### Client-Adresse ändern

```text
*configure update client=fd address=fd.example port=9112
Client 'fd' updated: address=fd.example, port=9112
```

Alle übrigen Direktiven und Kommentare der Client-Datei bleiben (behebt K-10: die frühere Fassung schrieb die Datei mit vier Direktiven neu).

### Fehler

| Meldung | Ursache |
|---|---|
| `configure update: configuration editing is not available (no configuration path)` | Director ohne Konfigurationspfad (nur in Tests). |
| `<Typ> "<name>" not allowed by the console's ACL` | Ressourcen-ACL der Konsole. |
| `<Typ> "<name>" not found` | Ressource gibt es nicht. |
| `configuration invalid: …` mit dem Text der Prüfung | Die geänderte Konfiguration besteht `odir -t` nicht (z. B. `Keyword "bogus" not permitted`, Verweis auf eine unbekannte Ressource); die Datei bleibt unverändert. |
| `Run = <wert>: <fehler>` | Run-Zeile eines Schedules ungültig (z. B. `Job type field: Fulll in run record not found`). |
| `configure update: … nested block …` | Direktive in einem verschachtelten Block. |
| `configure update: <typ> resources cannot be changed with configure update yet` | `pool`, `storage`, `console`. |

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `usage:` mit den zwei Aufrufformen | Unterbefehl oder Ressourcentyp fehlt. |
| `invalid subcommand <wort>.` | Unterbefehl ist weder `add` noch `export`. |
| `invalid resource type <typ>.` | Unbekannter Ressourcentyp oder `device`. |
| `Only one Director resource allowed.` | `configure add director` ist nicht möglich. |
| `Resource "<Typ>": missing name parameter.` | `name=` fehlt. |
| `Resource "<Typ>" does not permit directive "<direktive>".` | Direktive gibt es für diesen Typ nicht; Schreibweise mit `help configure` prüfen. |
| `Missing value for directive "<direktive>"` | Direktive ohne `=<wert>` angegeben. |
| `Could not add directive "<direktive>": character ';' (end of directive) is forbidden.` | Wert enthält `;` (ebenso `'@' (include)` für `@`). |
| `Resource "<Typ>" with name "<name>" already exists.` | Name ist schon vergeben. |
| `Resource config directory "<pfad>/" does not exist.` | Das Verzeichnis `bareos-dir.d/<typ>/` fehlt; anlegen und erneut versuchen. |
| `configure error: Illegal character "<z>" in name.` gefolgt von `configure error: Name not specified for resource` | Name enthält ein unzulässiges Zeichen. |
| `configure error: Empty Password not allowed in Resource "<name>"` | `password=` ohne Wert. |
| `configure error: SaveResource failed` / `configure error: ParseAllTokens failed.` | Eine Pflichtdirektive fehlt (zum Beispiel `address` beim Client). |
| `configure error: Expect YES, NO, TRUE, or FALSE, got: <wert>` / `configure error: Parser Error occurred.` | Wert hat den falschen Typ. |
| `configure error: Could not find config Resource "<Typ>" referenced on line <n> : <zeile>` | Verweis auf eine nicht vorhandene Ressource (zum Beispiel `profile=nosuch`). |
| `failed to create config resource "<name>"` | Job/JobDefs ist auch mit den JobDefs-Werten unvollständig (fehlt `type`, `messages` oder `pool`). |
| `usage: configure export client=<clientname>` | `client=` fehlt, hat keinen Wert oder nennt keinen bekannten Client. |
| `configure: is an invalid command.` | Die Konsole darf `configure` nicht ausführen. |
| `configure add profile: refused. …` / `configure add user: refused. …` | Profile und User nur im Editor (F-1). |
| `configure add console: refused, …` | Die enge Form ist verletzt oder die Profile übersteigen die eigenen Rechte ([enge Form](#configure-add-console--enge-form-onesimus-abweichung)). |

## Siehe auch

- [reload](reload.md) — Konfiguration nach Änderungen an den Dateien neu laden
- [show](show.md) — Ressourcen der laufenden Konfiguration anzeigen
- [help](help.md) — `help configure` listet alle Direktiven je Ressourcentyp
- [create](create.md) — Katalogeintrag für einen neuen Pool anlegen
- [.consoles](dot-consoles.md), [.profiles](dot-profiles.md), [.users](dot-users.md) — Namen der angelegten Konsolen, Profile und Benutzer
