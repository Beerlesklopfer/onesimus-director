# .acl — Konsolen-ACLs anzeigen und ändern (Onesimus)

## Zweck

`.acl` zeigt die wirksamen ACLs einer Console-Ressource und ändert sie im laufenden Director. Die Änderung wird sofort wirksam (die nächste ACL-Prüfung sieht sie) und wie bei [configure add](configure.md) in die Ressourcendatei der Konsole geschrieben, sodass sie einen [reload](reload.md) oder Neustart übersteht.

**Onesimus-Erweiterung, nicht im Original (C-074, Entscheidung in [#4](https://github.com/Beerlesklopfer/onesimus-director/issues/4): bleibt als dokumentierte Erweiterung).** Das Bareos-Original kennt den Befehl nicht und antwortet `.acl: is an invalid command.`; dort ändert man ACLs in den Dateien unter `bareos-dir.d/console/` und `bareos-dir.d/profile/` und lädt sie mit `reload`.

## Berechtigungen

- `.acl` selbst muss in der `Command ACL` der Konsole erlaubt sein.
- `list` ohne `configure`-Recht zeigt nur die eigene Konsole; die ACLs anderer Konsolen sieht nur eine Konsole, die `configure` darf.
- `set`, `delete`, `clear` und `copy` brauchen wie `configure add` das Recht `configure`.

## Syntax

```text
.acl [list] [console=<name>]
.acl set console=<name> <typ>acl=<eintrag>[,<eintrag> …] …
.acl delete console=<name> <typ>acl[=<eintrag>[,<eintrag> …]] …
.acl clear console=<name>
.acl copy from=<console> to=<console>
```

`<typ>` ist einer von `job`, `client`, `storage`, `schedule`, `pool`, `command`, `fileset`, `catalog`, `where`, `pluginoptions` (also `jobacl=`, `commandacl=` usw., Groß-/Kleinschreibung egal).

## Argumente

| Argument | Bedeutung |
|---|---|
| `list` (oder kein Unterbefehl) | Wirksame ACLs anzeigen: je ACL-Typ zuerst die Einträge der Konsole, dann die ihrer Profile in Profilreihenfolge, mit `(Profile <name>)` markiert. Das ist die Reihenfolge, in der der Director prüft — der erste passende Eintrag entscheidet. `(none)` heißt: nichts erlaubt. Ohne `console=` die eigene Konsole bzw. mit `configure`-Recht alle Konsolen. |
| `set` | Ersetzt die eigenen Einträge der Konsole für jeden angegebenen Typ. Andere Typen und die Profile bleiben. |
| `delete` | Entfernt die angegebenen Einträge; `<typ>acl` ohne Wert entfernt alle eigenen Einträge dieses Typs. Ein nicht vorhandener Eintrag ist ein Fehler, dann ändert sich nichts. |
| `clear` | Entfernt alle ACL-Direktiven der Konsole. `Profile` bleibt. |
| `copy` | Gibt `to` die ACL- und `Profile`-Direktiven von `from`, sodass beide dieselben wirksamen ACLs haben. |
| `console=<name>` | Zielkonsole (bei `set`, `delete`, `clear` Pflicht). |
| `from=`, `to=` | Quell- und Zielkonsole bei `copy`. |

Die Datei wird wie bei `configure add` über `<name>.conf.tmp` und Umbenennen neu geschrieben (Dateirechte bleiben). Geänderte Einträge stehen in Anführungszeichen (`JobAcl = "a", "b"`), die übrigen Direktiven bleiben wie sie waren; Kommentare in der Datei gehen dabei verloren. Eine Konsole, die in einer Datei zusammen mit anderen Ressourcen steht (z. B. eine einzige `bareos-dir.conf`), ändert `.acl` nicht und meldet das; dann bleibt auch die laufende Konfiguration unverändert. Die Standardkonsole `*UserAgent*` hat keine ACLs.

## API-Modi

Unter `.api 0` und `.api 1` kommt derselbe Text. Unter `.api 2` liefert `list` die Konsolen unter `acl` als Liste von Objekten (`console`, `profiles` und je ACL-Typ eine Liste aus `value` und `source`, `"Console"` oder `"Profile <name>"`); die ändernden Unterbefehle liefern `acl` mit `action`, `console`, `filename`, `content` (geschriebener Ressourcentext) und `effective` (wirksame ACLs danach).

## Beispiele

Die Beispiele stammen aus einem Onesimus-Director mit der Paritätskonfiguration (privater Port, Kopie der Konfiguration); Pfade sind mit `[…]` gekürzt.

### Wirksame ACLs einer Konsole mit Profil

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl list console=parity-operator
Console "parity-operator":
  Profile = operator
  JobAcl = *all* (Profile operator)
  ClientAcl = *all* (Profile operator)
  StorageAcl = *all* (Profile operator)
  ScheduleAcl = *all* (Profile operator)
  PoolAcl = *all* (Profile operator)
  CommandAcl = !.bvfs_clear_cache (Profile operator), !.exit (Profile operator), !.sql (Profile operator), !configure (Profile operator), !create (Profile operator), !delete (Profile operator), !purge (Profile operator), !prune (Profile operator), !sqlquery (Profile operator), !umount (Profile operator), !unmount (Profile operator), *all* (Profile operator)
  FileSetAcl = *all* (Profile operator)
  CatalogAcl = *all* (Profile operator)
  WhereAcl = *all* (Profile operator)
  PluginOptionsAcl = *all* (Profile operator)
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original antwortet `.acl: is an invalid command.` (C-074, Onesimus-Erweiterung; Paritätsfälle `c074-dot-acl-list`, `c074-dot-acl-list-api2`, `c074-dot-acl-usage` als erwartete Abweichung).

### ACLs setzen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl set console=parity-limited jobacl=backup-bareos-fd,RestoreFiles poolacl=Full
Updated resource config file "[…]/bareos-dir.d/console/parity-limited.conf":
Console {
  Name = parity-limited
  Password = "parity-limited-pw"
  CommandACL = list, llist, show, help, .help, status, version, .jobs, .clients, .pools, .storages, .filesets, .catalogs, .schedule, .authorized, .defaults, whoami, time
  JobAcl = "backup-bareos-fd", "RestoreFiles"
  ClientACL = bareos-fd
  PoolAcl = "Full"
  StorageACL = File
  CatalogACL = MyCatalog
  FileSetACL = LinuxAll
  ScheduleACL = WeeklyCycle
}
Console "parity-limited":
  Profile = (none)
  JobAcl = backup-bareos-fd, RestoreFiles
  ClientAcl = bareos-fd
  StorageAcl = File
  ScheduleAcl = WeeklyCycle
  PoolAcl = Full
  CommandAcl = list, llist, show, help, .help, status, version, .jobs, .clients, .pools, .storages, .filesets, .catalogs, .schedule, .authorized, .defaults, whoami, time
  FileSetAcl = LinuxAll
  CatalogAcl = MyCatalog
  WhereAcl = (none)
  PluginOptionsAcl = (none)
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Einträge löschen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl delete console=parity-limited jobacl=RestoreFiles
Updated resource config file "[…]/bareos-dir.d/console/parity-limited.conf":
Console {
  Name = parity-limited
  Password = "parity-limited-pw"
  CommandACL = list, llist, show, help, .help, status, version, .jobs, .clients, .pools, .storages, .filesets, .catalogs, .schedule, .authorized, .defaults, whoami, time
  JobAcl = "backup-bareos-fd"
[…]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Alle ACLs leeren und von einer anderen Konsole kopieren

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl clear console=parity-limited
Updated resource config file "[…]/bareos-dir.d/console/parity-limited.conf":
Console {
  Name = parity-limited
  Password = "parity-limited-pw"
}
Console "parity-limited":
  Profile = (none)
  JobAcl = (none)
  ClientAcl = (none)
  StorageAcl = (none)
  ScheduleAcl = (none)
  PoolAcl = (none)
  CommandAcl = (none)
  FileSetAcl = (none)
  CatalogAcl = (none)
  WhereAcl = (none)
  PluginOptionsAcl = (none)
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .acl copy from=parity-operator to=parity-limited
Updated resource config file "[…]/bareos-dir.d/console/parity-limited.conf":
Console {
  Name = parity-limited
  Password = "parity-limited-pw"
  Profile = "operator"
}
Console "parity-limited":
  Profile = operator
  JobAcl = *all* (Profile operator)
[…]
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole ohne `configure`-Recht

```text
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]> .acl list console=admin
.acl: permission denied, listing the ACLs of Console "admin" requires the CommandACL configure.
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]> .acl set console=parity-operator jobacl=x
.acl set: permission denied, changing ACLs requires the CommandACL configure.
parity-operator@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `.acl: is an invalid command.` | `.acl` ist in der `Command ACL` nicht erlaubt (im Original: den Befehl gibt es nicht). |
| `.acl: usage:` mit Kurzhilfe | Unbekannter Unterbefehl. |
| `.acl <unterbefehl>: permission denied, changing ACLs requires the CommandACL configure.` | Ändern ohne `configure`-Recht. |
| `.acl: permission denied, listing the ACLs of Console "…" requires the CommandACL configure.` | Fremde Konsole ohne `configure`-Recht angefragt. |
| `.acl <unterbefehl>: console=<name> required.` / `.acl copy: from=<console> to=<console> required.` | Pflichtargument fehlt. |
| `Console "…" not found.` | Es gibt keine Console-Ressource dieses Namens. |
| `.acl set: specify at least one <type>acl=<entries>.` | `set` ohne ACL-Argument (ebenso `delete`). |
| `<Typ> entry "…" not found in Console "…".` / `Console "…" has no <Typ> entries.` | `delete` mit einem Eintrag, den die Konsole selbst nicht hat (Profil-Einträge ändert `.acl` nicht). |
| `Console "…" is defined in "…" together with other resources; .acl only changes consoles that have their own resource file.` | Die Konsole steht nicht in einer eigenen Datei; Datei aufteilen oder von Hand ändern. |

## Siehe auch

- [show](show.md) — `show console=<name>` und `show profile=<name>`
- [.authorized](dot-authorized.md) — eine einzelne Berechtigung prüfen
- [.consoles](dot-consoles.md), [.profiles](dot-profiles.md) — Namen der Konsolen und Profile
- [configure](configure.md) — neue Konsolen anlegen
- [reload](reload.md) — Konfiguration neu laden
