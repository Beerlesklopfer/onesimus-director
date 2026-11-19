# Betrieb des Onesimus Directors (odir)

Nachschlageseite für Administratoren: wie `odir` als Dienst läuft, welche
Einstellungen es außerhalb von `/etc/bareos/bareos-dir.d` gibt und wo es sich
bewusst anders verhält als `bareos-dir`. Die Kommandozeilenparameter stehen in
[kommandozeile.md](kommandozeile.md).

`odir` ist ein Drop-in-Ersatz für `bareos-dir` 25: dieselbe Konfiguration,
derselbe Katalog, dieselben Clients und Storage-Daemons. Der Rückweg auf
`bareos-dir` muss jederzeit offen bleiben. Deshalb gibt es keine eigenen
Direktiven für Einstellungen, die `bareos-dir` nicht kennt; solche
Einstellungen laufen über Umgebungsvariablen (siehe unten).

## Dienst

| | |
| --- | --- |
| Unit | `odir.service` (`/usr/lib/systemd/system/odir.service`) |
| Benutzer | `bareos:bareos`, Arbeitsverzeichnis `/var/lib/bareos` |
| Start | `/usr/sbin/odir -c /etc/bareos/bareos-dir.d -f` |
| Reload | `systemctl reload odir`: erst `odir -t`, nur bei Erfolg SIGHUP |
| Neustart bei Fehler | nach 30 s, höchstens 10-mal am Tag (`systemctl reset-failed odir` setzt zurück) |
| Kein Neustart | Exit 41 (falsche Argumente) und 42 (Konfigurationsfehler) |
| Gegenseitiger Ausschluss | `Conflicts=bareos-director.service`: startet einer, stoppt der andere |
| Paket-Upgrade | die postinst startet den Dienst sofort neu (`dpkg -i`) |

Exit-Codes beim Start:

| Code | Bedeutung |
| --- | --- |
| 0 | regulär beendet (SIGTERM) |
| 1 | Laufzeitfehler, z. B. Katalog nicht erreichbar, Adresse nicht bindbar |
| 42 | Konfigurationsfehler, auch fehlende oder unlesbare Konfiguration |

Vor einem Neustart die Konfiguration prüfen (liest den Katalog nur, ändert nichts):

```sh
sudo -u bareos /usr/sbin/odir -t -c /etc/bareos/bareos-dir.d; echo $?
```

## Adressen und Ports

Wie `bareos-dir` 25:

- `DirAddresses` (Blockform): jede Zeile wird gebunden. `ip` und `ipv4` sind
  IPv4, `ipv6` ist IPv6. Werte ohne Anführungszeichen schreiben
  (`addr = ::1`, nicht `addr = "::1"`; `bareos-dir` lehnt die zitierte Form ab).
  Eine Zeile ohne `port` nimmt 9101, nicht `DirPort`. `DirAddresses` darf nicht
  mit `DirAddress` oder `DirPort` gemischt werden.
- `DirAddress`: die erste Adresse, auf die ein Name auflöst.
- ohne Angabe: `0.0.0.0` und `::`.
- Eine Adresse, die sich nicht binden lässt, wird dreimal im Abstand von 5 s
  versucht; danach endet der Start (kein Lauschen auf einer Teilmenge).
- gRPC (TLS) lauscht auf denselben Adressen auf `DirPort + 1000` (Vorgabe
  10101) bzw. auf `gRPC Port`.

Prüfen: `ss -ltnp | grep -E '9101|10101'` und die Zeilen `listening on …` im Journal.

## Katalog beim Start

1. Verbindung wie `CheckCatalog()`: jeder Katalog muss sich öffnen lassen
   (sechs Versuche, 5 s Abstand), sonst endet der Start mit Exit 1.
   Verbunden wird mit allen Angaben der Catalog-Ressource (Adresse oder
   Socket, Port, Benutzer, Passwort), genauso wie bei `odir -t`.
2. Schema-Erweiterung (gewollte Funktion): der Migrator spielt die
   Onesimus-Views und -Funktionen aus `/usr/lib/bareos/scripts/ddl/v2251_*.json`
   und `v2252_*.json` ein, wenn sie noch fehlen. Die Bareos-Tabelle `version`
   bleibt auf der Bareos-Version (2250), die Onesimus-Stufen stehen in der
   eigenen Tabelle `bareos_schema_version`; `bareos-dir` öffnet den Katalog
   also weiterhin. Die Migrationen sind additiv (kein DROP, kein TRUNCATE).
   Journal: `applied N schema migration(s)` bzw. `schema up to date`.
3. Jobs eines früheren Laufs, die noch auf `C` oder `R` stehen, enden als `f`
   (wie `CheckCatalog(UPDATE_AND_FIX)`); nur beim Start, nicht bei `reload`.
4. Fällt der Katalog im Betrieb aus, scheitern die Anfragen mit einer Meldung;
   ist er wieder da, verbindet der Director von selbst neu.

## Umgebungsvariablen

Vollständige Liste der Variablen, die `odir` im Betrieb liest. Gesetzt werden
sie mit einem systemd-Drop-in:

```sh
sudo systemctl edit odir
#   [Service]
#   Environment=ONESIMUS_DEFAULT_MEDIA_WAIT=3600
sudo systemctl restart odir
```

| Variable | Wirkung | Werte | Vorgabe |
| --- | --- | --- | --- |
| `ONESIMUS_DEFAULT_MEDIA_WAIT` | Wartezeit auf ein Volume für Jobs ohne `Max Wait Time`; danach wird der Job abgebrochen | ganze Sekunden > 0 | 600 |
| `PGTZ` | Zeitzone der Katalogsitzung (wie bei libpq); sonst die Einstellung von Rolle/Datenbank, sonst `log_timezone` des Servers | Zeitzonenname | – |
| `RUST_LOG` | Filter der Log-Ausgabe (tracing `EnvFilter`), ersetzt die Vorgabe | z. B. `info`, `debug,hyper=info` | `info` (mit `-d`: `debug`) |

Nur für Tests, nicht für den Betrieb: `ONESIMUS_CATREQ_DB`,
`OVFS_EQUIV_TEMPLATE`, `OVFS_EQUIV_URL`, `OVFS_FALLBACK_URL`,
`UPDATE_GRPC_API_DOC`. Beim Bau: `SOURCE_DATE_EPOCH` (Build-Datum in der
Versionsangabe, für reproduzierbare Pakete).

### ONESIMUS_DEFAULT_MEDIA_WAIT

- Steuert, wie lange ein Job ohne `Max Wait Time` auf ein beschreibbares
  Volume wartet (z. B. bei vollem Pool oder fehlendem Medium). Danach bricht
  der Watchdog den Job ab; die Meldung nennt den „Onesimus default“ und geht
  wie jede Job-Meldung an die Messages-Ziele (auch Mail).
- Einheit: ganze Sekunden. Vorgabe: 600 (Onesimus-Entscheidung C-085).
- Ungültige Werte (leer, 0, negativ, keine Zahl) fallen auf 600 zurück; im
  Journal steht dann eine Warnung.
- Es gibt keinen Wert für „unbegrenzt“.
- `Max Wait Time` im Job (oder in seinen JobDefs) hat immer Vorrang.
- Wird einmal beim Start gelesen: wirksam erst nach `systemctl restart odir`
  (nicht nach `reload`).
- Wirksamen Wert prüfen:

  ```sh
  sudo journalctl -u odir -b | grep "volume wait"
  # volume wait for jobs without Max Wait Time: 3600 s (ONESIMUS_DEFAULT_MEDIA_WAIT)
  systemctl show odir -p Environment
  ```

**Unterschied zu bareos-dir:** `bareos-dir` wartet ohne `Max Wait Time`
unbegrenzt. `odir` begrenzt die Wartezeit, damit ein nächtlicher Job bei vollem
Pool nicht tagelang hängt. Eine eigene Direktive dafür gibt es bewusst nicht:
`bareos-dir` würde die Konfiguration mit einer unbekannten Direktive nicht mehr
lesen, und der Rückweg wäre versperrt. Wer das Verhalten von `bareos-dir` auch
unter `odir` will, setzt `Max Wait Time` im Job; das verstehen beide Directors.

## Meldungen per Mail und an den Operator

`Mail`, `MailOnError`, `MailOnSuccess` und `Operator` der Messages-Ressource
werden wie in `bareos-dir` bedient: Mail sammelt die Meldungen eines Jobs und
ruft am Job-Ende einmal `MailCommand` auf (Meldungen auf stdin, Job-Codes wie
`%r`, `%j`, `%e` ersetzt); `Operator` ruft `OperatorCommand` je Meldung sofort
auf. Ohne `MailCommand` nimmt `odir` wie `bareos-dir` `/usr/lib/sendmail -F
BAREOS <Empfänger>`. Daemon-Meldungen ohne Job gehen am Ende jedes Jobs, jeder
Konsolensitzung und beim Beenden hinaus. Fehler des Mail-Programms stehen als
„Message delivery ERROR“ im Log.

## AutoPrune am Job-Ende

Nach jedem Job prunt der Director Jobs und Files des Clients nach `Job
Retention` und `File Retention` (Pool-Retention vor Client-Retention), wenn
`AutoPrune = yes` gesetzt ist (wie `bareos-dir`). Der erste Job nach dem
Wechsel von einer Version ohne AutoPrune prunt den ganzen Rückstand; vorher den
Katalog sichern.

Strenger als `bareos-dir` (Entscheidung des Nutzers vom 19.11.2026): Die
jüngste erfolgreiche Kette je Client und FileSet-Name im Katalog wird nie
gepurgt, auch wenn sie älter als die Retention ist:

- Kette: der letzte Full mit JobStatus `T`/`W`, die letzte Differential danach
  (mit Dateien) und die Incrementals danach, wie `AccurateGetJobids` (Level
  Incremental); dazu Base-Jobs der Kette. Das FileSet zählt nach Namen über
  alle FileSetIds (eine geänderte Include-Liste ergibt eine neue FileSetId).
- Gilt unabhängig davon, ob Job, FileSet oder Client noch konfiguriert sind
  (umbenannter Job, entfernter Client).
- Weder die Jobs noch ihre File-Zeilen werden gepurgt: `prune jobs`, `prune
  files`, AutoPrune, `purge jobs`/`purge files` überspringen sie
  (`PurgedFiles` bleibt 0; der Restore mit Dateiauswahl bleibt möglich).
- Ein Volume mit Daten (JobMedia) eines Jobs dieser Ketten wird nicht gepruned,
  gepurgt, recycelt, `truncate`d oder gelöscht: weder durch `Prune Volumes`,
  `Recycle Oldest Volume`/`Purge Oldest Volume` bei der Volume-Suche eines Jobs
  noch durch `prune volume`, `purge volume`, `truncate` oder `delete volume` auf
  der Konsole (dort hilft auch `yes` nicht).
- Ist die Kette nicht ermittelbar (Katalogfehler), wird nichts gepurgt.
- Meldungen nennen, was geschützt wurde, z. B. `Keeping the latest Full chain of
  client "fd1" FileSet "fs" (JobIds 3,4,5); its jobs are not pruned.` oder
  `Volume "Full-0007" holds the latest Full chain of client "fd1" FileSet "fs"
  (JobIds 3); not pruned.`

Auswirkung auf die Volume-Wahl: Findet ein Job außer solchen Volumes kein
freies, wartet er auf ein Volume wie bei einem vollen Pool (Meldung an den Job:
„the job waits for a volume as with a full pool“). Abhilfe: ein Volume labeln
oder hinzufügen bzw. `Maximum Volumes` erhöhen. Ein Volume wird erst wieder frei,
wenn eine neuere Kette die geschützte ablöst (nächster Full). Ausweg für den
Einzelfall bleibt `delete jobid=…` auf der Konsole.

## Abweichungen von Bareos

Bewusst entschieden (Stand 19.11.2026):

| Thema | bareos-dir | odir | Grund |
| --- | --- | --- | --- |
| Schema-Erweiterung | keine | Views und Funktionen beim Start (v2251/v2252), `version` bleibt 2250 | Logik in der Datenbank änderbar ohne neuen Code |
| Wartezeit auf ein Volume | unbegrenzt ohne `Max Wait Time` | 600 s, über `ONESIMUS_DEFAULT_MEDIA_WAIT` | kein hängender Nachtlauf; keine Direktive wegen Rückweg |
| Gescheiterter Batch-Insert der File-Zeilen | Block wird ignoriert, Job `T`, Dateien fehlen im Katalog | Job wird fatal (`f`), „Attribute create error“ | kein stilles Fehlen von Dateien im Restore-Baum |
| Purge (PurgeFiles/PurgeJobs) | ohne Transaktion, Fehler ignoriert | eine Transaktion, Fehler werden gemeldet | kein halb gelöschter Job |
| `prune jobs`, Behalten-Liste nicht ermittelbar | Fehler ignoriert, löscht trotzdem | bricht vor dem Löschen ab | die letzte Full-Kette darf nie verloren gehen |
| Schutz der jüngsten Kette | nur bei `prune jobs`/AutoPrune, nur für konfigurierte Job/FileSet/Client, FileSet nach FileSetId; File-Zeilen fallen nach `File Retention` weg; Volumes werden ohne Rücksicht gepruned/recycelt | je Client und FileSet-Name immer, auch nicht konfiguriert; Jobs und File-Zeilen bleiben; Volumes mit diesen Jobs werden weder gepruned, gepurgt, recycelt, `truncate`d noch gelöscht; Katalogfehler purgt nichts | Entscheidung des Nutzers 19.11.2026: der letzte Stand jedes Clients muss restaurierbar bleiben; ein Job wartet dafür ggf. auf ein Volume |
| ACLs in Dateien ändern | `configure add console/profile/user` legt Dateien mit ACLs an | `configure add console/profile/user` und `.acl set/delete/clear/copy` werden abgelehnt; ACL-Ressourcen nur im Editor | Entscheidung des Nutzers 19.11.2026 (zweistufige ACLs): sonst kann sich eine Console mit `configure` selbst mehr Rechte geben |
| gRPC | gibt es nicht | TLS auf `DirPort + 1000`, gleiche Adressen wie BNET | Schnittstelle der GUI |
| Mail-Sammlung | Dateien `<WorkingDirectory>/*.mail` | im Speicher | – |
| Paketversion | – | bleibt `25.1.2+onesimus.0.2.0`, Installation mit `dpkg -i` | Entscheidung des Nutzers; neue Builds an Prüfsumme und Build-Datum erkennbar |

Vor dem Rückweg auf `bareos-dir` müssen Onesimus-eigene Direktiven (`gRPC Port`,
`gRPC TLS Certificate`, `gRPC TLS Key`, `mDNS Advertise`) aus der Konfiguration
entfernt werden; `bareos-dir` kennt sie nicht.
