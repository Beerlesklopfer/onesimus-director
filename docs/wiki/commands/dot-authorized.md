# .authorized — Berechtigung der Konsole prüfen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L224) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Check for authorization

**Originalsyntax:**

```text
.authorized job=<job-name> | client=<client-name> | storage=<storage-name> | schedule=<schedule-name> | pool=<pool-name> | cmd=<command> | fileset=<fileset-name> | catalog=<catalog>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 3 von 3 Fällen byte-identisch; Modi: Text, Konsolen parity-limited.

**Paritätsfälle (3):** `dot-authorized-job`, `dot-authorized-job-denied`, `dot-authorized-cmd`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/dot.rs:220](../../../crates/onesimus-dird/src/commands/dot.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`.authorized` prüft, ob die eigene Konsole auf eine Ressource oder einen Befehl zugreifen darf, und antwortet `authorized` oder `not authorized`. Grundlage sind die ACLs der Console-Ressource und ihrer Profile (`JobACL`, `ClientACL`, `StorageACL`, `ScheduleACL`, `PoolACL`, `CommandACL`, `FileSetACL`, `CatalogACL`). Oberflächen wie bareos-webui nutzen den Befehl, um Schaltflächen nur bei vorhandener Berechtigung anzuzeigen. Der Befehl liest nur.

## Syntax

```text
.authorized job=<job> | client=<client> | storage=<storage> | schedule=<schedule> | pool=<pool> | cmd=<befehl> | fileset=<fileset> | catalog=<katalog> …
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `job=<job>` | Jobname | Prüft gegen `JobACL`. |
| `client=<client>` | Clientname | Prüft gegen `ClientACL`. |
| `storage=<storage>` | Storage-Name | Prüft gegen `StorageACL`. |
| `schedule=<schedule>` | Schedule-Name | Prüft gegen `ScheduleACL`. |
| `pool=<pool>` | Poolname | Prüft gegen `PoolACL`. |
| `cmd=<befehl>` | Befehlsname | Prüft gegen `CommandACL`. |
| `fileset=<fileset>` | FileSet-Name | Prüft gegen `FileSetACL`. |
| `catalog=<katalog>` | Katalogname | Prüft gegen `CatalogACL`. |

Die Schlüssel sind unabhängig von Groß-/Kleinschreibung. Ob der Name als Ressource existiert, wird nicht geprüft, nur die ACL. Mehrere Argumente sind erlaubt; das Ergebnis bestimmt das **letzte** bekannte Argument, frühere werden überschrieben. Ohne Argument, nur mit unbekannten Schlüsseln oder mit einem Schlüssel ohne Wert lautet die Antwort `not authorized`, auch für eine Konsole ohne Einschränkungen.

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Textzeile. Unter `.api 2` steht das Ergebnis in `result[".authorized"].authorized` (`true`); `not authorized` kommt als JSON-RPC-Fehler `"message": "failed"` mit `error.data.result[".authorized"].authorized` = `false`.

## Beispiele

Die Beispiele verwenden die eingeschränkte Konsole `parity-limited` der Paritätstests. Sie hat `JobACL = backup-bareos-fd`, `ClientACL = bareos-fd`, `PoolACL = Full, Scratch`, `StorageACL = File`, `CatalogACL = MyCatalog`, `FileSetACL = LinuxAll`, `ScheduleACL = WeeklyCycle` und eine `CommandACL` mit `list` und `.authorized`, aber ohne `run` und ohne `.api`.

### Erlaubter Job

Paritätsfall `dot-authorized-job`.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized job=backup-bareos-fd
authorized
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Gesperrter Job

Paritätsfall `dot-authorized-job-denied`.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized job=BackupCatalog
not authorized
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Client, Storage, Pool und Katalog

Paritätsfälle `dcfg-auth-client-bareos-fd`, `dcfg-auth-storage-File`, `dcfg-auth-pool-Full`, `dcfg-auth-pool-Incremental`, `dcfg-auth-catalog-MyCatalog`.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized client=bareos-fd
authorized
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized storage=File
authorized
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized pool=Full
authorized
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized pool=Incremental
not authorized
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized catalog=MyCatalog
authorized
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Schedule und FileSet

Nur das FileSet `LinuxAll` ist freigegeben (Paritätsfälle `dcfg-auth-schedule-WeeklyCycle`, `dcfg-auth-fileset-SelfTest`).

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized schedule=WeeklyCycle
authorized
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized fileset=SelfTest
not authorized
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Befehl prüfen

Paritätsfälle `dot-authorized-cmd`, `dcfg-auth-cmd-list`.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized cmd=run
not authorized
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized cmd=list
authorized
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Mehrere Argumente

Das letzte Argument entscheidet: im zweiten Aufruf ist der Job erlaubt, der Pool aber nicht (Paritätsfälle `dcfg-auth-two`, `dcfg-auth-two-denied`).

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized job=backup-bareos-fd client=bareos-fd
authorized
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized job=backup-bareos-fd pool=Incremental
not authorized
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole ohne Einschränkungen

Die Standardkonsole darf alles; ohne Argument antwortet aber auch sie `not authorized` (Paritätsfälle `dcfg-auth-job-admin`, `dcfg-auth-noarg`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized job=BackupCatalog
authorized
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized
not authorized
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Schlüssel

Paritätsfall `dcfg-auth-unknown`.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized foo=bar
not authorized
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Prüfung unter `.api 2`

Erlaubt (Paritätsfall `dcfg-auth-job-ok-api2`) und verweigert (`c046-dotauthorized-api2`, ohne Argument), beide mit der Standardkonsole. `parity-limited` kann `.api` nicht verwenden, weil der Befehl in ihrer `CommandACL` fehlt.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized job=BackupCatalog
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    ".authorized": {
      "authorized": true
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .authorized
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {
        ".authorized": {
          "authorized": false
        }
      },
      "messages": {}
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `not authorized` | Die ACL der Konsole erlaubt den Namen nicht, oder es fehlt ein gültiges Argument. |
| `.authorized: is an invalid command.` | `.authorized` steht nicht in der `CommandACL` der Konsole. |

## Siehe auch

- [whoami](whoami.md) — Namen der eigenen Konsole anzeigen
- [.consoles](dot-consoles.md) — Namen aller Console-Ressourcen
- [.profiles](dot-profiles.md) — Namen aller Profile
- [show](show.md) — `show console=<name>` zeigt die ACLs einer Konsole
