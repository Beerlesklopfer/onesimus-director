# var — Variablen auflösen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L520) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Does variable expansion

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 16 von 16 Fällen byte-identisch; Modi: Text, API 2, Konsolen parity-limited, parity-operator.
- Nacktes `var` gibt im Original undefinierte Speicherreste aus; der Fall `var` prüft deshalb nur die Katalogzeilen davor.

**Paritätsfälle (16):** `c023-var-blank`, `c023-var-date`, `c023-var-job`, `c023-var-jobname`, `c023-var-undefined`, `c023-var-env`, `c023-var-ops`, `c023-var-errors`, `c023-var-escapes`, `c023-var-loops`, `c023-var-catalog-arg`, `c023-var-api2`, `c023-use-var`, `c023-limited`, `c023-operator`, `var`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:199](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->

## Zweck

`var` löst Variablen in einem Text so auf, wie es der Director bei Volume-Namen (`LabelFormat`) und ähnlichen Angaben tut, und gibt das Ergebnis aus. Damit lässt sich ein `LabelFormat` ausprobieren, bevor man es in die Konfiguration schreibt. Der Befehl öffnet dafür den Katalog der Sitzung (beim ersten Mal mit den Zeilen `Automatically selected Catalog: …` und `Using Catalog "…"`).

Die Werte stammen aus dem Konsolenjob der Sitzung: `${Job}`, `${Client}`, `${Pool}`, `${Storage}` und `${MediaType}` sind die Vorgaben des ersten Jobs der Konfiguration, `${Type}` ist `Console`, `${JobId}` ist `0`.

## Syntax

```text
var <text>
```

## Argumente

Alles hinter dem ersten Leerzeichen nach `var` ist der Text, führende Leerzeichen fallen weg, alle anderen bleiben erhalten. `catalog=` wird nicht ausgewertet, sondern als Text ausgegeben.

**Variablen** (Name am Anfang abkürzbar, `${Di}` = `${Dir}`):

| Variable | Bedeutung |
|---|---|
| `${Year}`, `${Month}`, `${Day}`, `${Hour}`, `${Minute}`, `${Second}` | aktuelles Datum und Uhrzeit, zweistellig (Jahr vierstellig) |
| `${WeekDay}` | Wochentag als Zahl, `0` = Sonntag |
| `${Dir}` | Name des Directors |
| `${Job}` | Name des Jobs |
| `${JobName}` | eindeutiger Jobname (`-Console-.<datum>_<zeit>_<nr>`) |
| `${JobId}` | JobId (`0` in der Konsole) |
| `${Type}`, `${Level}` | Jobtyp und Level (`Console`, Level leer) |
| `${Client}`, `${Pool}`, `${Storage}`, `${MediaType}` | Client, Pool, Storage und Medientyp des Jobs |
| `${NumVols}` | Zahl der Volumes im Pool |
| `${Catalog}` | gewählter Katalog |
| `${<Counter>}` | aktueller Wert einer Counter-Ressource; `${<Counter>+}` erhöht den Zähler dabei (verändert ihn dauerhaft, bei `Catalog =` auch im Katalog) |
| `${<UMGEBUNG>}` | Umgebungsvariable des Director-Prozesses, z.B. `${LANG}` |

Statt `${Name}` geht auch `$Name`; der Name reicht dann bis zum nächsten Zeichen, das kein Buchstabe oder keine Ziffer ist (`x$Levelx` sucht die unbekannte Variable `Levelx`).

**Operationen** hinter dem Namen, durch `:` getrennt und verkettbar (`${Dir:U:#}`):

| Operation | Wirkung |
|---|---|
| `:u`, `:l` | in Groß- bzw. Kleinbuchstaben |
| `:#` | Länge |
| `:o<von>,<anzahl>` / `:o<von>-<bis>` | Teilstring ab Position `<von>` (ab 0) |
| `:s/<muster>/<ersatz>/[g][t]` | Ersetzen mit regulärem Ausdruck; `g` = alle, `t` = Muster als einfacher Text, `\1`, `\2` = Gruppen |
| `:y/<von>/<nach>/` | Zeichen übersetzen wie `tr` |
| `:p/<breite>/<füllzeichen>/<l\|r\|c>` | auf Breite auffüllen, Text links, rechts oder mittig |
| `:-<text>` | `<text>`, wenn die Variable leer oder unbekannt ist |
| `:+<text>` | `<text>`, wenn die Variable gesetzt ist, sonst leer |
| `:*<text>` | leer, wenn die Variable gesetzt ist, sonst `<text>` |

**Weitere Schreibweisen:** `\n`, `\\`, `\x41`, `\x{4142}`, `\101` (oktal) für Sonderzeichen; `${VAR[n]}` und `${VAR[#]}` für Elemente; `[ … ]{start,ende,schritt}` als Schleife über Indizes. Eine unbekannte Variable bleibt unverändert stehen.

## API-Modi

Unter `.api 0` und `.api 1` kommt das Ergebnis als eine Zeile (unter `.api 1` ohne die Katalogzeilen). Unter `.api 2` ist das Ergebnis leer (`"result": {}`); das aufgelöste Ergebnis wird dort nicht ausgegeben.

## Beispiele

### Namen des Directors

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var ${Dir}
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
bareos-dir
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Datum und Uhrzeit

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var Y=${Year} M=${Month} D=${Day} h=${Hour} m=${Minute} s=${Second} w=${WeekDay}
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Y=2026 M=10 D=16 h=16 m=46 s=28 w=5
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Jobvariablen und Abkürzungen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var ${Dir}|${Type}|${Level}|${JobId}|${NumVols}|${Catalog}
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
bareos-dir|Console| |0|0|MyCatalog
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var ${Job}|${Client}|${Pool}|${Storage}|${MediaType}
parity-admin|bareos-fd|Incremental|File|File
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var $Dir $Type x$Levelx
bareos-dir Console x$Levelx
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var ${Di}/${J}/${Med}/${Cat}
bareos-dir/parity-admin/File/MyCatalog
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Eindeutiger Jobname

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var ${JobName}
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
-Console-.2026-10-16_16.46.29_10
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Operationen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var ${Dir:u} ${Dir:l} ${Dir:#} ${Dir:o1,3} ${Dir:o1-3} ${Dir:o4,}
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
BAREOS-DIR bareos-dir 10 are are os-dir
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var ${Dir:s/dir/DIR/} ${Dir:s/r/R/g} ${Dir:s/x/y/t} ${Dir:y/a-z/A-Z/} ${Dir:s/(b)(a)/\2\1/}
bareos-DIR baReos-diR bareos-dir BAREOS-DIR abreos-dir
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var ${Dir:p/20/*/l}| ${Dir:p/20/*/r}| ${Dir:p/20/-=/c}|
bareos-dir**********| **********bareos-dir| -=-=-bareos-dir-=-=-|
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var ${Dir:+set} ${Dir:*x} ${Dir:-x} ${Dir:U:#}
set  bareos-dir 10
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Unbekannte Variablen und Vorgabewerte

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var ${Undefined}
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
${Undefined}
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var $Undefined end
$Undefined end
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var ${Undef:+set}|${Undef:*x}|${Undef:-dflt}
${Undef:+set}|${Undef:*x}|dflt
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Umgebungsvariablen, Elemente und Schleifen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var ${LANG} ${LC_ALL}
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
C C
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var ${LANG[0]} ${LANG[#]} ${LANG[1]} ${LANG[-1]}
C C ${LANG[1]} 1
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var [x${LANG[#]}]{0,1,2}
xCxx
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var [${LANG[#]},]
C,
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Sonderzeichen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var a\nb \\ \x41 \101 \q
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
a
b \ A A q
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var \x{4142} a\x00b
AB a
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Leerzeichen und Anführungszeichen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var hello world
hello world
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var    spaced   out
spaced   out
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var "${Dir}" 'x'
"bareos-dir" 'x'
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Fehlerhafter Ausdruck

Ist der Ausdruck ungültig, gibt der Director eine leere Zeile aus (Paritätsfall `c023-var-errors`).

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var ${Dir
Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"

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
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> var ${Dir}
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
| (leere Zeile) | Der Ausdruck ist ungültig, z.B. fehlende `}`, unbekannte Operation (`${Dir:q}`), Teilstring außerhalb des Textes (`${Dir:o9,1}`) oder ein einzelnes `$` oder `\` am Ende. Die genaue Ursache steht nur im Director-Log. |
| `var: is an invalid command.` | Die `CommandACL` der Konsole erlaubt den Befehl nicht. |

`var` ohne Text gibt bei Onesimus eine leere Zeile aus. Beim Original ist die Ausgabe in diesem Fall undefiniert: Es liest über das Ende der Eingabe hinaus und gibt Reste früherer Meldungen aus (Notiz im Paritätsfall `var`).

## Siehe auch

- [use](use.md) — Katalog für `${Catalog}` wählen
- [time](time.md) — Uhrzeit des Directors
- [show](show.md) — `LabelFormat` und Counter in der Konfiguration ansehen
