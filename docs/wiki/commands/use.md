# use — Katalog wählen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L518) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Use specific catalog

**Originalsyntax:**

```text
use catalog=<catalog>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 7 von 7 Fällen byte-identisch; Modi: Text, Konsolen parity-limited, parity-operator.

**Paritätsfälle (7):** `c002-use-then-list`, `c002-use-noargs`, `c023-use`, `c023-use-var`, `c023-limited`, `c023-operator`, `use`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:297](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`use` legt fest, mit welchem Katalog (Catalog-Ressource) die Sitzung arbeitet. Alle folgenden Katalogbefehle wie `list`, `query` oder `sqlquery` benutzen ihn, bis die Sitzung endet oder ein anderer gewählt wird. Der Befehl schließt eine bereits offene Katalogverbindung und öffnet die neue. Die Konfiguration und andere Sitzungen bleiben unberührt.

## Syntax

```text
use catalog=<katalog>
use
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `catalog=<katalog>` | Name einer Catalog-Ressource | Diesen Katalog verwenden. |

Ohne `catalog=` wählt der Director aus: Gibt es nur einen erlaubten Katalog, nimmt er ihn mit dem Hinweis `Automatically selected Catalog: …`; bei mehreren fragt er nach (siehe Dialoge). Ein unbekannter Name (`catalog=Nope`), ein Name ohne Schlüsselwort (`use MyCatalog`) und `catalog` ohne Wert werden genauso behandelt wie fehlendes `catalog=`. Weitere Wörter werden ignoriert.

## Dialoge

Bei mehreren Katalogen, die die Konsole benutzen darf (`CatalogACL`), zeigt der Director die Liste `The defined Catalog resources are:` und fragt `Select Catalog resource (1-<n>): `. Antwort ist die Nummer; `.` bricht ab. Beispielausgabe nicht verfügbar: Der Testaufbau hat nur einen Katalog.

Im GUI-Modus ([`gui on`](gui.md)) entfällt die Frage: Ohne `catalog=` nimmt der Director den ersten Katalog der Konfiguration.

## API-Modi

Unter `.api 0` kommen zwei Zeilen: `Using Catalog "<katalog>"` beim Öffnen und `Using Catalog name=<katalog> DB=<datenbank>` als Bestätigung, bei automatischer Auswahl davor `Automatically selected Catalog: <katalog>`. Unter `.api 1` kommt nur die Bestätigungszeile. Unter `.api 2` ist das Ergebnis leer (`"result": {}`), auch wenn ein unbekannter Name angegeben wurde.

## Beispiele

### Katalog ausdrücklich wählen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> use catalog=MyCatalog
Using Catalog "MyCatalog"
Using Catalog name=MyCatalog DB=parity_1244604
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ohne Angabe (nur ein Katalog vorhanden)

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> use
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Using Catalog name=MyCatalog DB=parity_1244604
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannter Katalogname

Der Director fällt ohne Fehlermeldung auf die automatische Auswahl zurück (Paritätsfall `c023-use`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> use catalog=Nope
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Using Catalog name=MyCatalog DB=parity_1244604
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unter `.api 1`

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> .api 1
api: 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> use catalog=MyCatalog
Using Catalog name=MyCatalog DB=parity_1943382
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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> use catalog=nosuch
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
| `use: is an invalid command.` | Die `CommandACL` der Konsole erlaubt den Befehl nicht. |
| `Could not find a Catalog resource` | Nur im GUI-Modus: Die Konfiguration enthält keinen Katalog. |
| `You must specify a "use <catalog-name>" command before continuing.` | Nur im GUI-Modus: Die Konsole darf den ersten Katalog nicht benutzen. `use catalog=<katalog>` mit einem erlaubten Katalog eingeben. |

Die beiden GUI-Meldungen stammen aus dem Originalquelltext; Beispielausgabe nicht verfügbar, weil der Testaufbau genau einen erlaubten Katalog hat.

## Siehe auch

- [list](list.md) — Katalogeinträge auflisten
- [sqlquery](sqlquery.md) — SQL direkt im gewählten Katalog
- [.catalogs](dot-catalogs.md) — Namen der Kataloge
- [var](var.md) — `${Catalog}` zeigt den gewählten Katalog
