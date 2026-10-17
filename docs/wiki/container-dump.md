# Container-Dump vor der Sicherung (Podman und Docker)

`scripts/onesimus-container-dump.sh` sichert vor dem Backup-Job, was sich aus
Container-Umgebungen nicht einfach aus einer Registry wiederherstellen lässt.
Die rohen Speicherverzeichnisse (`/var/lib/docker/overlay2`,
`/var/lib/containers/storage`) werden bewusst **nicht** gesichert: Sie ändern
sich laufend, sind groß und nach einem Restore oft inkonsistent.

## Was gesichert wird

Ausgabe nach `DUMP_DIR` (Standard `/var/backups/containers`):

| Pfad | Inhalt |
|---|---|
| `<engine>/inventory/images.txt` | alle Images mit Digests – damit sie aus der Registry exakt wieder gezogen werden können |
| `<engine>/inventory/containers.txt`, `containers-inspect.json` | alle Container mit vollständiger Konfiguration (Mounts, Ports, Umgebung, Image) |
| `<engine>/inventory/volumes.txt`, `networks.txt` | Volumes und Netzwerke |
| `<engine>/images/<name>.tar.gz` | **lokal gebaute** Images (kein RepoDigest einer echten Registry; Podman-Digests `localhost/…` zählen als lokal), je Image-ID ein Archiv mit allen Tags |

`<engine>` ist `docker`, `podman` (rootful) oder `podman-<benutzer>` für
rootless Podman (Benutzer über `PODMAN_USERS`). Ein Archiv wird nur neu
geschrieben, wenn sich die Image-ID geändert hat; Archive entfernter Images
werden gelöscht. Das Podman-Infrastruktur-Image `localhost/podman-pause` wird
übersprungen.

Persistente Daten (Volumes, Bind-Mounts wie `/var/lib/nextcloud/data`) gehören
direkt ins FileSet; Datenbanken in Containern brauchen zusätzlich einen
konsistenten Dump (z. B. `mysqldump`/`pg_dump` im Container).

## Einrichtung am Client

1. Skript auf den Client kopieren, z. B. nach `/usr/local/lib/bareos/onesimus-container-dump.sh`
   (Besitzer root, Modus 755). Falls der File Daemon `Allowed Script Dir` setzt,
   muss das Verzeichnis dort enthalten sein.
2. Im Job eine RunScript-Ressource ergänzen:

   ```
   RunScript {
     RunsWhen = Before
     RunsOnClient = Yes
     FailJobOnError = No
     Command = "/usr/local/lib/bareos/onesimus-container-dump.sh /var/backups/containers"
   }
   ```

   Für rootless Podman: `Command = "/usr/bin/env PODMAN_USERS=jbernau /usr/local/lib/bareos/onesimus-container-dump.sh /var/backups/containers"`.
3. `/var/backups/containers` ins FileSet aufnehmen und `reload`.

## Wiederherstellen

```
zcat /var/backups/containers/podman/images/<name>.tar.gz | podman load
zcat /var/backups/containers/docker/images/<name>.tar.gz | docker load
```

Registry-Images nach `inventory/images.txt` neu ziehen (Digest angeben), Container
nach `containers-inspect.json` bzw. den Compose-/Quadlet-Dateien neu anlegen.

## Rückgabewert

0 = alles gesichert (fehlende Engines sind kein Fehler), 1 = mindestens ein
Image oder Inventar konnte nicht geschrieben werden. Mit `FailJobOnError = No`
endet der Job dann mit Warnung statt mit Fehler.
