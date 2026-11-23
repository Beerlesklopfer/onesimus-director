# P2b: Zweige (Branch) im Verbund odir + osd – Entwurf

Stand: 23.11.2026. Status: **Entwurf und Umsetzung** (Plan: [OSD-GERAETE-UND-VERTEILUNG.md](OSD-GERAETE-UND-VERTEILUNG.md), Abschnitt „Zweige“, Entscheidungen D-9 bis D-11, D-25). Bedienung: [osd § Zweige](../wiki/osd.md), [Betrieb odir § Zweige](../wiki/betrieb-odir.md).

## Ausgangslage im Code (gelesen am 23.11.2026)

**odir, ein Backup-Job heute** (`commands/job.rs::run_selected`, `backup_worker.rs`):

1. `run_selected` legt den `Job`-Satz an (`create_job_record`), berechnet Level und Basis (`get_level_since_time`), Pool, Fileset-Satz, baut `JobParams` mit `BackupSession` und stellt den Job in die Warteschlange des `JobManager` (Client-, Storage-, Job- und Director-Grenzen).
2. Der `JobManager` startet einen **Worker-Prozess je Job** (`job_worker::spawn_job_worker`). Der Worker spricht mit dem Director über Pipes (`PipeMessage`): `CatReq`, `UpdCat`, `Jmsg`, `JobEnd`; der Director schreibt die Katalogsätze (`catreq.rs`, Attribute als `File`-Sätze zur **JobId der Pipe-Nachricht**).
3. `backup_worker::run`: SD verbinden (gRPC zuerst), `Job`-Befehl (`start_sd_job`: Sitzung, Schlüssel), FD verbinden und einrichten, `use storage` (`reserve_write_device`), `storage`-Befehl an den FD, `run` an den SD, Nachrichtenfaden des SD (`message_thread`: `C`/`B` Katalog, `U` Attribute, `Jmsg`, `3099`), `backup` an den FD, Warten auf `2800 End Job` und `3099`.

**osd, ein Job heute:** `Job`-Befehl legt ein `Job`-Objekt mit eigener `VolSessionId` an; `use storage` merkt das `UseStorage`; `run` wartet auf den FD (`wait_fd`) und führt `run_job`. In `do_append_data` baut osd den **Producer** (Ring in `/dev/shm`, ein Geräteschreiber-Prozess, eine Postfach-Fadenschleife für `CatReq`/`Jmsg`/Status über den **Director-Kanal dieses Jobs**), verpackt jeden Datensatz in Blöcke und gibt Attribute erst frei, wenn ihr Block geschrieben ist.

## Gewählte Architektur: ein Stream, mehrere Senken, je Zweig ein ganzer Job

Die einfachste tragfähige Lösung nutzt, was es gibt: **Ein Zweig ist ein vollständiger Job in beiden Daemons**, nur ohne FD-Sitzung.

```
                 odir                                   osd
 Haupt-Job   Worker H ── FD (liest einmal) ── Daten ──► Job H ─┬─ Senke H: Ring H → Schreiber → Device 1 (Pool Inc)
                    └── SD-Steuerung (Job, use storage, Plan, run)
 Zweig-Job   Worker Z ── SD-Steuerung (Job, use storage, Attach) ► Job Z ─ Senke Z: Ring Z → Schreiber → Device 2 (Pool AI)
```

### osd

- **Zwei neue typisierte Nachrichten** in `stored_service.proto` (`BranchFrame` im `StoredFrame`, Repo onesimus-proto, Branch `p2b-zweige`): `BranchPlan` (am Haupt-Job vor `run`: die Namen der Zweige), `BranchAttach` (am Zweig-Job statt `run`: der Name des Haupt-Jobs) und `BranchReply`. Im Prozess erscheinen sie als Nutzdaten mit Kennung (`onesimus_bnet_grpc::branch`), damit `BnetStream` und die Befehlsschleifen unverändert bleiben; über BNET gibt es sie nicht (bareos-sd/-dir kennen keine Zweige).
- **Senken statt eines Ringverbrauchers:** `do_append_data` hält eine Liste von `Sink` (der Job selbst, dann die Zweige). Jede Senke hat **eigenen Producer** (Ring, Geräteschreiber, Postfach), **eigenen Director-Kanal** (die Steuerverbindung des Zweig-Jobs), **eigene `VolSessionId`** im Satzkopf und eigene Attribute. Jeder Datensatz geht der Reihe nach an alle lebenden Senken. Damit entstehen je Zweig eigene Volumes, `JobMedia`-, `File`- und `Job`-Sätze ohne Änderung am Schreiber.
  - Abweichung vom Plan („ein Ring, alle Zweige lesen jedes Fach“): Je Senke ein Ring, weil jeder Zweig eigene Blockgrenzen, eigene Sitzungsnummer im Satzkopf, eigenes Volume und eigene `JobMedia`-Bereiche braucht. Ein gemeinsamer Ring würde das nicht hergeben, ohne den Schreiber umzubauen. Die Kosten sind ein Kopieren je Senke im Speicher (Blockbau), kein Netz- und kein Plattenzugriff.
  - Ringgröße (D-25): Jobs einer Zweig-Gruppe bekommen 512 Fächer, wenn `Ring Slots` nicht gesetzt ist.
- **Zweig-Sitzung:** `BranchAttach` hängt den Zweig-Job an den Haupt-Job (wartet bis `Branch Attach Wait`, Vorgabe 120 s, bis der Haupt-Job bekannt ist), meldet `3010`/Status `R` und wartet, bis die Senke fertig ist; dann meldet sie `3099 Job … end` mit Status, Dateien und Bytes des **Zweigs**.
- **Haupt-Job:** `do_append_data` wartet auf die geplanten Zweige (angehängt, oder gesehen und wieder weg: `use storage` gescheitert, abgebrochen; sonst bis `Branch Attach Wait`) und schließt dann die Menge. Ein Zweig, der zu spät kommt, wird abgewiesen.
- **Fehlerisolation (D-11):** Scheitert eine Senke (Gerät fehlt, Schreiber stirbt, Volume nicht beschreibbar, Abbruch des Zweig-Jobs), endet nur deren Job mit `E` bzw. `A`; die anderen laufen weiter. Scheitert die Senke des Haupt-Jobs, bekommen die Zweige trotzdem den Stream (der Haupt-Job endet `E`). Scheitern alle, endet der FD-Strom mit Fehler.
- **Abbruch:** `cancel` auf einen Zweig-Job setzt dessen Flag; die Senke sieht es beim nächsten Datensatz, schließt ihr Volume mit EOS und endet `A`. `cancel` auf den Haupt-Job beendet den Strom für alle (odir bricht auch die Zweig-Jobs ab).

### odir

- **Konfiguration:** Overlay `<overlay>/odir.d/job/<Haupt-Job>.conf` mit `Branch = <Job-Name>` (mehrfach). Der Zweig-Job ist eine normale Job-Ressource (Type Backup) mit eigenem Pool, Storage, Fristen und Level-Schema. Prüfung beim Laden: Zweig existiert, ist ein Backup-Job, ist nicht der Haupt-Job und hat selbst keine Zweige. Die Bareos-Dateien bleiben Standard.
- **Start (`run_selected`):** Hat der Job `Branch`-Einträge, legt odir für jeden Zweig einen `Job`-Satz an (eigene JobId, eigener Name `<Zweig>.<Zeit>_<n>`), berechnet Level und Basis (siehe unten) und stellt den Zweig-Job **mit** dem Haupt-Job in die Warteschlange. Ein Zweig-Job zählt nicht gegen die Grenzen von Client und Director (er liest den Client nicht und gehört zum laufenden Haupt-Job); Storage- und Job-Grenzen gelten weiter.
- **Zweig-Worker (`branch_worker`):** wie `backup_worker` ohne FD: SD verbinden (nur gRPC), `Job`-Befehl, `use storage`, `BranchAttach`, dann der vorhandene Nachrichtenfaden (`CatReq`, `UpdCat`, `Jmsg`, `3099`) und das Ende (`JobEnd`, `BackupEnd`) wie bei jedem Backup-Job. Damit laufen Katalog, Meldungen, Report, Mail, `status`, `list`, `cancel`, Fristen je Job unverändert.
- **Haupt-Worker:** schickt nach dem `Job`-Befehl `BranchPlan` an den SD, sonst unverändert.
- **Katalog:** Standardtabellen unverändert. Die Zuordnung Zweig → Haupt-Job steht in **`onesimus.job_branch`** (Migration v2257, Muster v2256): `branch_job_id`, `main_job_id`, `created_at`. odir schreibt sie beim Anlegen des Zweigs.
- **Attribute einmal je Job, Files je Zweig:** Der FD liefert die Attribute einmal; osd schickt sie je Senke mit dem Namen des Zweig-Jobs an dessen Steuerverbindung, odir schreibt sie auf die JobId dieses Zweig-Workers. Damit hat **jeder Zweig-Job seine eigenen `File`-Sätze**; Restore, Prune und Purge je Job bleiben unabhängig.
- **Level und Basis (D-10):** Der Haupt-Job entscheidet das Level des Streams: Braucht ein Zweig nach seinen Regeln Full (kein früherer Full des Zweig-Jobs für Client und FileSet), wird der Stream Full und damit das Level aller. Sonst gilt das Level des Haupt-Jobs für alle; die **Basis** (`since`-Zeit) des Streams ist die **älteste** von Haupt-Job und Zweigen (ein verpasster Lauf eines Zweigs lässt den Haupt-Job ab der älteren Basis lesen). Der Zweig mit der jüngeren Basis speichert dann etwas mehr als nötig.
- **Cancel:** `cancel` eines Haupt-Jobs bricht seine Zweig-Jobs mit ab; `cancel` eines Zweig-Jobs bricht nur ihn ab.

## Grenzen dieser Stufe (im Plan und BACKLOG geführt)

- **Accurate:** Die Liste der bekannten Dateien kommt aus der Kette des Haupt-Jobs. Löschungen eines Zweigs mit abweichender Kette werden daran gemessen.
- **Warten auf Medien:** Wartet ein Zweig auf ein Volume (Mount-Anforderung), hält er den Strom an (der langsamste Zweig bestimmt den Takt, D-13/D-25; später Zeitlimit und Spooling).
- **Nur gRPC:** Mit BNET zum SD scheitert ein Zweig-Job mit klarer Meldung; der Haupt-Job läuft ohne Zweige weiter.
- **Konsolidierung (VirtualFull des AI-Zweigs):** Der Zweig-Job ist ein normaler Backup-Job. Der VirtualFull aus seinen Jobs läuft im Rig mit ausdrücklicher `jobid=`-Liste. **Die automatische Kettenwahl (Accurate, VirtualFull ohne Liste, `restore`) wählt wie in Bareos nach Client und FileSet und mischt Haupt-Job und Zweige** (im Rig beobachtet: „Consolidating JobIds 2,4,3“ mit dem Incremental des Haupt-Jobs). Frage 3 unten.
- **Zweig-Job allein starten:** Er ist eine normale Job-Ressource und lässt sich mit `run` allein starten; ein Zeitplan dafür ist nicht sinnvoll (er schreibt dann wie jeder Backup-Job aus eigener FD-Sitzung).

## Fragen an den Besitzer

1. **Level der Zweige:** Die Umsetzung gibt allen Zweigen das Level des Streams (Haupt-Level, bei Bedarf auf Full angehoben). Soll ein Zweig mit abweichendem Rhythmus (z. B. Zweig nur sonntags Full) später ein eigenes Level-Schema mit eigener Basis bekommen, wird der Stream das Maximum bilden müssen (Folge-Punkt im BACKLOG)?
2. **Ring je Senke statt gemeinsamer Ring:** Abweichung vom Plantext (Begründung oben). Einverstanden?
3. **Kettenwahl:** Soll die Wahl der Kette (Accurate, VirtualFull, Restore „letzte Sicherung“) für Jobs aus `onesimus.job_branch` nach Pool getrennt werden (Haupt-Jobs sehen nie Zweige, ein Zweig nur die Jobs seines Pools)? Das weicht von Bareos ab (nur für Zweig-Gruppen) und löst die Mischung; ohne sie bleibt die ausdrückliche `jobid=`-Liste.
