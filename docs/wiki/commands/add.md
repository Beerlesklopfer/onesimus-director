# add — Volumes ohne Label im Katalog anlegen

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L309) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Add media to a pool

**Originalsyntax:**

```text
add pool=<pool-name> storage=<storage-name> jobid=<jobid>
```

**RunScript erlaubt:** nein. **Audit-Ereignis:** ja. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `partial`. Paritätsfälle vorhanden, vollständige Funktionsparität aber nicht nachgewiesen (siehe Befunde).

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 1, Konsolen parity-c030-client, parity-c030-job, parity-c030-pool, parity-c030-storage.
- Verglichen sind Dialog, ACL-Warnung und Abbruch; das tatsächliche Anlegen von Volumes und der Katalogzustand danach sind nicht verglichen.

**Paritätsfälle (9):** `c011-add-cancel`, `c011-add-pool-cancel`, `c011-add-pool-badnumber-cancel`, `c011-add-api1-cancel`, `c030-add-job-cancel`, `c030-add-storage-cancel`, `c030-add-client-cancel`, `c030-add-pool-cancel`, `add-noargs`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/extra.rs:880](../../../crates/onesimus-dird/src/commands/extra.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
## Zweck

`add` legt Media-Einträge für Volumes im Katalog an, ohne das Medium selbst zu beschriften. Das braucht man nur in Sonderfällen, etwa wenn Bänder schon ein Bareos-Label tragen oder wenn Volumes vorab im Katalog stehen sollen. Der Befehl verändert nur den Katalog; der Storage Daemon wird nicht angesprochen. Im Normalfall ist [label](label.md) der richtige Befehl, darauf weist auch der Director bei jedem Aufruf hin.

## Syntax

```text
add [ pool=<pool> ] [ storage=<storage> ]
```

## Argumente

| Argument | Werte | Bedeutung |
|---|---|---|
| `pool=<pool>` | Pool-Name aus dem Katalog | Pool, in dem die Volumes angelegt werden. Fehlt die Angabe, erscheint eine Pool-Auswahl. Darf die Konsole nur einen Pool benutzen, wird er automatisch gewählt. |
| `storage=<storage>` | Name einer Storage-Ressource | Bestimmt den Media Type und die Storage-Zuordnung der neuen Volumes. Fehlt die Angabe, erscheint eine Storage-Auswahl. Bricht man sie ab, wählt der Director den Media Type selbst (oder fragt danach, wenn es mehrere gibt). |

`jobid=<jobid>` steht zwar in der Bareos-Hilfe, wird von `add` aber nicht ausgewertet. Die übrigen Angaben (Anzahl, Name, Startnummer, Slot) fragt `add` immer im Dialog ab.

## Dialoge

1. **Pool** — `Select the Pool (1-<n>):`, wenn `pool=` fehlt.
2. **Storage** — `Select Storage resource (1-<n>):`, wenn `storage=` fehlt. Ein `.` an dieser Stelle bricht nicht ab, sondern lässt den Director den Media Type automatisch wählen (`Automatically selected Media Type: File`).
3. **Anzahl** — `Enter number of Volumes to create. 0=>fixed name. Max=<n>:`. `0` legt genau ein Volume mit einem festen Namen an, eine Zahl größer 0 eine Serie mit fortlaufender Nummer. `Max` ist die Zahl der Volumes, die der Pool noch aufnehmen darf (1000, wenn der Pool unbegrenzt ist). Hat der Pool sein Maximum schon erreicht, fragt der Director zuerst `Enter new maximum (zero for unlimited):`.
4. **Name** — bei `0` `Enter Volume name:`, sonst `Enter base volume name:`. An den Basisnamen wird eine vierstellige Nummer angehängt.
5. **Startnummer** — nur bei einer Serie: `Enter the starting number:` (mindestens 1).
6. **Slot und InChanger** — nur bei Autochanger-Storages: `Enter slot (0 for none):` und `InChanger? yes/no:`.

Ein `.` bricht jede Frage ab (außer der Storage-Auswahl, siehe oben). Nach dem Abbruch wird nichts angelegt.

## API-Modi

Unter `.api 0` laufen die Dialoge wie gezeigt. Unter `.api 1` schickt der Director die Auswahllisten ohne Nummern als einzelne Einträge (`Defined Pools:` gefolgt von `Incremental`, `Full`, `Differential`, `Scratch`) und wartet auf eine Auswahl; diese Form ist für Programme gedacht. Für `.api 2` gibt es keinen Paritätsfall.

## Beispiele

### Pool auswählen lassen und abbrechen

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> add
You probably don't want to be using this command since it
creates database records without labeling the Volumes.
You probably want to use the "label" command.

Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Defined Pools:
1: Incremental
2: Full
3: Differential
4: Scratch
Select the Pool (1-4):
> .
Selection aborted, nothing done.
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ein Volume mit festem Namen anlegen

Mit `0` als Anzahl legt `add` genau ein Volume mit dem eingegebenen Namen an.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> add pool=Full storage=File
You probably don't want to be using this command since it
creates database records without labeling the Volumes.
You probably want to use the "label" command.

Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Enter number of Volumes to create. 0=>fixed name. Max=97:
> 0
Enter Volume name:
> docsvol1
1 Volumes created in pool Full
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Eine Serie von Volumes anlegen

Mit `2` als Anzahl, dem Basisnamen `docsser` und der Startnummer `1` entstehen die Volumes `docsser0001` und `docsser0002`.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> add pool=Full storage=File
You probably don't want to be using this command since it
creates database records without labeling the Volumes.
You probably want to use the "label" command.

Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Enter number of Volumes to create. 0=>fixed name. Max=96:
> 2
Enter base volume name:
> docsser
Enter the starting number:
> 1
2 Volumes created in pool Full
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Storage-Auswahl abbrechen

Bricht man die Storage-Auswahl ab, wählt der Director den Media Type selbst und fragt weiter nach der Anzahl.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> add pool=Full
You probably don't want to be using this command since it
creates database records without labeling the Volumes.
You probably want to use the "label" command.

Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
The defined Storage resources are:
1: File
2: ParityNdmp
3: ParityNdmpNative
Select Storage resource (1-3):
> .
Selection aborted, nothing done.
Automatically selected Media Type: File
Enter number of Volumes to create. 0=>fixed name. Max=97:
>
```

### Konsole mit nur einem erlaubten Pool

Darf die Konsole nur den Pool `Full` benutzen, entfällt die Pool-Auswahl (Konsole `parity-c030-pool`).

```text
parity-c030-pool@bareos-dir[25.1.2+onesimus.0.2.0]> add
You probably don't want to be using this command since it
creates database records without labeling the Volumes.
You probably want to use the "label" command.

Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Automatically selected Pool: Full
The defined Storage resources are:
1: File
2: ParityNdmp
3: ParityNdmpNative
Select Storage resource (1-3):
> .
Selection aborted, nothing done.
Automatically selected Media Type: File
Enter number of Volumes to create. 0=>fixed name. Max=97:
>
```

### Zu große Anzahl

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> add pool=Full storage=File
You probably don't want to be using this command since it
creates database records without labeling the Volumes.
You probably want to use the "label" command.

Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Enter number of Volumes to create. 0=>fixed name. Max=94:
> 500
The number must be between 0 and 94
Enter number of Volumes to create. 0=>fixed name. Max=94:
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Ungültiger Volume-Name

Das Zeichen `|` trennt in Bareos Volume-Listen und ist im Namen nicht erlaubt.

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> add pool=Full storage=File
You probably don't want to be using this command since it
creates database records without labeling the Volumes.
You probably want to use the "label" command.

Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Enter number of Volumes to create. 0=>fixed name. Max=94:
> 0
Enter Volume name:
> a|b
Illegal character "|" in a volume name.
Enter Volume name:
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

### Startnummer 0

```text
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]> add pool=Full storage=File
You probably don't want to be using this command since it
creates database records without labeling the Volumes.
You probably want to use the "label" command.

Automatically selected Catalog: MyCatalog
Using Catalog "MyCatalog"
Enter number of Volumes to create. 0=>fixed name. Max=94:
> 3
Enter base volume name:
> docsz
Enter the starting number:
> 0
Start number must be greater than zero.
Enter the starting number:
> .
*UserAgent*@bareos-dir[25.1.2+onesimus.0.2.0]>
```

## Fehlermeldungen

| Meldung | Ursache und Abhilfe |
|---|---|
| `You probably don't want to be using this command since it …` | Hinweis bei jedem Aufruf, kein Fehler. Zum Beschriften neuer Medien [label](label.md) benutzen. |
| `The number must be between 0 and <max>` | Die Anzahl ist größer als der freie Platz im Pool. Kleinere Zahl eingeben oder `Maximum Volumes` des Pools erhöhen. |
| `Pool already has maximum volumes=<n>` | Der Pool ist voll; der Director fragt nach einem neuen Maximum. |
| `Illegal character "<z>" in a volume name.` | Der Name enthält ein nicht erlaubtes Zeichen. Erlaubt sind Buchstaben, Ziffern und `:.-_/`. |
| `Volume name too long.` / `Volume name must be at least one character long.` | Der Name ist zu lang oder leer. |
| `Start number must be greater than zero.` | Die Startnummer einer Serie muss mindestens 1 sein. |
| `Expected a positive integer, got: <eingabe>` | Bei einer Auswahl oder Anzahl wurde keine Zahl eingegeben. |
| `Selection aborted, nothing done.` | Der Dialog wurde mit `.` abgebrochen. |

## Siehe auch

- [label](label.md) — Volume beschriften und im Katalog anlegen
- [create](create.md) — Pool-Eintrag im Katalog anlegen
- [update](update.md) — Eigenschaften von Volumes ändern
- [delete](delete.md) — Volumes aus dem Katalog löschen
- [list](list.md) — Volumes anzeigen (`list volumes pool=<pool>`)
