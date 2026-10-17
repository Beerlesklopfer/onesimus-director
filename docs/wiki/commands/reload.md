# reload — Konfiguration neu laden

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L423) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Reload conf file

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `registered_unverified`. Funktionsparität ist nicht durch einen Paritätsfall nachgewiesen.

- Kein Paritätsfall (Reload ändert die laufende Konfiguration beider Directors).

**Lokaler Handler:** [crates/onesimus-dird/src/commands/admin.rs:1841](../../../crates/onesimus-dird/src/commands/admin.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`reload` liest alle Konfigurationsdateien des Directors neu ein, ohne den Dienst neu zu starten. Der Befehl verändert die laufende Konfiguration: neue, geänderte und gelöschte Ressourcen gelten sofort. Ist die neue Konfiguration fehlerhaft, bleibt die bisherige aktiv. Die genaue Fehlerursache steht dann nicht in der Antwort, sondern im Meldungsprotokoll des Directors (Messages-Ressource, zum Beispiel `/var/log/bareos/bareos.log`). Ein erfolgreicher `reload` legt für neue Client- und Storage-Ressourcen die Katalogeinträge an.

## Syntax

```text
reload
```

## Argumente

Der Befehl hat keine Argumente. Weitere Wörter hinter `reload` werden ignoriert (Paritätsfall `dcfg-reload-arg`).

## API-Modi

Unter `.api 0` und `.api 1` kommt dieselbe Textzeile `reloaded` oder `failed to reload`. Unter `.api 2` steht das Ergebnis in `result.reload.success` (`true`); ein Fehlschlag kommt als JSON-RPC-Fehler `"message": "failed"` mit `error.data.result.reload.success` = `false` und leerer Meldungsliste.

## Beispiele

Die Dateipfade sind auf das Standardverzeichnis `/etc/bareos` umgeschrieben; in den Paritätsläufen liegt die Konfiguration in einem Arbeitsverzeichnis.

### Konfiguration neu laden

Paritätsfall `dcfg-reload-ok`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> reload
reloaded
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Neuen Client nach dem Neuladen anzeigen

Vor dem Aufruf wurde die Datei `bareos-dir.d/client/dcfg-new.conf` angelegt (Paritätsfall `dcfg-reload-new`). Nach `reload` zeigt `show` auch Werte, die vom Standard abweichen und vorher nicht ausgegeben wurden, etwa `Catalog`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> reload
reloaded
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show client=dcfg-new
Client {
  Name = "dcfg-new"
  Address = "192.0.2.12"
  Password = "[…]"
  Catalog = "MyCatalog"
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Neu laden nach `configure add`

Mit [configure](configure.md) angelegte Ressourcen stehen in eigenen Dateien und bleiben beim Neuladen erhalten (Paritätsfall `dcfg-reload-after-add`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> configure add profile name=dcfg-prof jobacl=*all*
Created resource config file "/etc/bareos/bareos-dir.d/profile/dcfg-prof.conf":
Profile {
  Name = "dcfg-prof"
  JobAcl = *all*
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> reload
reloaded
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show profile=dcfg-prof
Profile {
  Name = "dcfg-prof"
  JobAcl = "*all*"
}

*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehlerhafte Konfiguration

Die Datei `bareos-dir.d/client/dcfg-broken.conf` enthält einen Client-Block ohne schließende Klammer (Paritätsfall `dcfg-reload-broken`). Der Director behält die alte Konfiguration; der neue Client existiert nicht.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> reload
failed to reload
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> show client=dcfg-broken
clients resource dcfg-broken not found.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

Im Meldungsprotokoll stehen bei einem fehlgeschlagenen `reload` die Ursache und danach immer diese zwei Zeilen; hier für einen Job ohne `FileSet` (Paritätsfall `c057-reload-job-without-fileset`, Zeitstempel durch `<T>` ersetzt):

```text
<T> bareos-dir JobId 0: Error: "fileset" directive in Job "c057-nofs" resource is required, but not found.
<T> bareos-dir JobId 0: Error: Please correct the configuration in /etc/bareos/bareos-dir.d/*/*.conf
<T> bareos-dir JobId 0: Error: Resetting to previous configuration.
```

**Abweichung vom Original:** Bei Syntaxfehlern (etwa der fehlenden Klammer oben) schreibt das Original zusätzlich die Lexer-Diagnose mit Zeile und Spalte ins Protokoll; Onesimus schreibt nur die beiden Schlusszeilen. (C-057)

### Neuer Katalog ist nicht erreichbar

Vorher wurde mit `configure add catalog name=dcfg-catalog dbname=bareos dbuser=bareos` ein Katalog angelegt, zu dem keine Verbindung möglich ist (Paritätsfall `dcfg-zz-reload-after-catalog`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> reload
reloaded
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

**Abweichung vom Original:** Das Original versucht beim Neuladen jeden Katalog zu öffnen, antwortet nach einigen Sekunden `failed to reload` und schreibt `Could not open Catalog "dcfg-catalog", database "bareos": …` ins Protokoll; Onesimus prüft die Verbindung nicht. (C-075)

### Neu laden unter `.api 2`

Paritätsfall `dcfg-reload-ok-api2`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> reload
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "reload": {
      "success": true
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehlschlag unter `.api 2`

Paritätsfall `c057-reload-failed-api2` (Client-Block ohne Abschluss).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 2
{
  "jsonrpc": "2.0",
  "id": null,
  "result": {
    "api": 2
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> reload
{
  "jsonrpc": "2.0",
  "id": null,
  "error": {
    "code": 1,
    "message": "failed",
    "data": {
      "result": {
        "reload": {
          "success": false
        }
      },
      "messages": {}
    }
  }
}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Konsole ohne Berechtigung

Paritätsfall `dcfg-reload-limited`.

```text
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]> reload
reload: is an invalid command.
parity-limited@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `failed to reload` | Die neue Konfiguration ist fehlerhaft, die alte bleibt aktiv. Die Ursache steht im Meldungsprotokoll, zum Beispiel `"Address" directive in Client "<name>" resource is required, but not found.`, `Attempt to define second Client resource named "<name>" is not permitted.` oder `AlwaysIncremental configured in job <name> which is not of job type "backup" …`. Datei korrigieren und `reload` wiederholen. |
| `reload: is an invalid command.` | Die Konsole darf `reload` nicht ausführen. |

## Siehe auch

- [configure](configure.md) — Ressourcen zur Laufzeit anlegen, ohne `reload`
- [show](show.md) — Ressourcen der laufenden Konfiguration anzeigen
- [messages](messages.md) — anstehende Meldungen des Directors abrufen
- [create](create.md) — Katalogeintrag für einen neuen Pool anlegen
