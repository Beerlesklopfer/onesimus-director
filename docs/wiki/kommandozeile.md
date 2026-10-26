# Kommandozeilenparameter

Diese Referenz beschreibt die Startparameter der Programme im Repository. Die
[Befehlsreferenz](commands/README.md) erklärt dagegen Befehle wie `status` oder
`list`, die nach dem Verbindungsaufbau an den Director gesendet werden.
Stand: 18.10.2026, abgeglichen mit den `Cli`-Definitionen und ihrer Verwendung in
[oconsole](../../crates/oconsole/src/main.rs),
[odir](../../crates/odir/src/main.rs) und
[ofd](../../crates/ofd/src/main.rs).

Alle Programme unterstützen `-h` / `--help` und `-V` / `--version`.
Werte in `<…>` sind Platzhalter und werden durch eigene Angaben ersetzt.

## oconsole

```bash
oconsole [OPTIONEN]
```

Ohne `--command` oder `--mcp` startet die interaktive Konsole mit History und
Tab-Vervollständigung. Kommandozeilenwerte überschreiben die Konfigurationsdatei.
Ohne expliziten Pfad wird zuerst `/etc/bareos/oconsole.conf`, danach
`/etc/bareos/bconsole.conf` versucht; die erste lesbare Datei wird verwendet.

| Parameter | Erklärung und Standard |
| --- | --- |
| `-c`, `--config <DATEI>` | Konfigurationsdatei verwenden. |
| `-a`, `--address <HOST>` | Adresse des Directors; ohne Konfigurationswert `localhost`. |
| `-p`, `--port <PORT>` | BNET-Port; ohne Konfigurationswert `9101`. Beeinflusst auch den berechneten Standard-gRPC-Port. |
| `-n`, `--name <NAME>` | Loginname der Console-Ressource. Ohne Console-Ressource wird `*UserAgent*` mit dem Director-Passwort verwendet; der Name der Director-Ressource ist kein Console-Login. |
| `-w`, `--password <PASSWORT>` | Passwort des Console-Logins überschreiben. Alternativ aus der Konfigurationsdatei beziehen. |
| `-e`, `--command <BEFEHL>` | Einen vollständigen Konsolenbefehl ausführen und beenden. Befehle mit Leerzeichen in Anführungszeichen setzen. Fehler führen zu Exitcode `1`. |
| `--grpc` | gRPC mit TLS statt BNET verwenden. Kann auch in der Konfiguration aktiviert sein; es gibt keinen CLI-Schalter zum Abschalten. |
| `--grpc-port <PORT>` | gRPC-Port überschreiben. Standard: konfigurierter Wert, sonst BNET-Port + `1000` (normalerweise `10101`). |
| `--grpc-fingerprint <SHA256>` | Erwarteten SHA-256-Fingerabdruck des Director-Zertifikats angeben. Sonst wird `~/.config/oconsole/known_directors` verwendet. Beim ersten interaktiven Aufruf wird nach Vertrauen gefragt; Einzelbefehle und MCP benötigen einen bereits bekannten oder expliziten Fingerabdruck. |
| `--mcp` | MCP-Server über stdin/stdout starten. Interaktive Eingabe entfällt; stdout transportiert das MCP-Protokoll. |
| `--mcp-read-only` | Mit `--mcp` nur lesende Tools anbieten, auch über stdio. |
| `--mcp-http <IP:PORT>` | Mit `--mcp` Streamable HTTP am Pfad `/mcp` statt stdio anbieten, z. B. `127.0.0.1:9104`. HTTP bietet im Produktions-Build nur lesende Tools. |
| `--mcp-token-file <DATEI>` | Mit `--mcp-http` diese Bearer-Token-Datei verwenden. Fehlende Datei wird mit Modus `0600` angelegt. Standard: `$XDG_CONFIG_HOME/oconsole/mcp.token`, sonst `~/.config/oconsole/mcp.token`. |
| `--mcp-host <AUTHORITY>` | Mit `--mcp-http` einen zusätzlichen HTTP-Host akzeptieren, z. B. `mcp.example.org`. Mehrfach verwendbar. |
| `--mcp-allow-write` | Nur im Build mit Feature `mcp-development` verfügbar: Schreib- und Restore-Tools auch über HTTP freigeben. `--mcp-read-only` hat Vorrang. stdio bietet diese Tools bereits standardmäßig in jedem Build. |

MCP und `-e` als getrennte Betriebsarten verwenden. Weitere Details zu Tools,
Transport und Authentifizierung: [MCP](mcp.md), [gRPC](grpc-api.md).

```bash
# Interaktive Konsole mit expliziter Konfiguration
oconsole -c /etc/bareos/oconsole.conf

# Einzelabfrage; der gesamte Director-Befehl ist ein Argument
oconsole -e 'list jobs limit=10'

# gRPC auf einem anderen Host (Passwort aus der Konfiguration)
oconsole --grpc -a pdc -n admin --grpc-port 10101

# Lesender MCP-Server über stdio
oconsole --mcp --mcp-read-only

# MCP über HTTP mit eigener Token-Datei
oconsole --mcp --mcp-http 127.0.0.1:9104 --mcp-token-file /tmp/oconsole-mcp.token
```

## odir

```bash
odir [OPTIONEN]
```

Ohne `-f` läuft der Director als Hintergrundprozess. Der systemd-Dienst startet
mit `-f`; systemd übernimmt dort die Prozessüberwachung.

| Parameter | Erklärung und Standard |
| --- | --- |
| `-c`, `--config <PFAD>` | Konfigurationsdatei oder Verzeichnis; Standard `/etc/bareos/bareos-dir.d`. |
| `-d`, `--debug-level <ZAHL>` | Debug-Level; Standard `0`. `999` zeigt die Debug-Level-Referenz und beendet das Programm. |
| `--dt`, `--debug-timestamps` | Zeitstempel in BNET-Debug-Ausgaben einschalten. |
| `-f`, `--foreground` | Im Vordergrund laufen. |
| `-u`, `--user <BENUTZER>` | Vor Katalogverbindung und Worker-Start Benutzerrechte abgeben; für einen Benutzerwechsel als root starten. |
| `-g`, `--group <GRUPPE>` | Zielgruppe beim Rechtewechsel verwenden. |
| `-t`, `--test-config` | Konfiguration, Ressourcen und Katalogverbindung prüfen, danach mit Prüfstatus beenden. Startet keinen normalen Director-Betrieb. |
| `-k`, `--kill` | Laufenden Director stoppen und beenden. PID aus `-p`, andernfalls Prozesssuche über `/proc`. |
| `-p`, `--pid-file <DATEI>` | PID-Datei schreiben; auch bei `-f`. Ohne Angabe keine PID-Datei. Bei `-k` als PID-Quelle verwenden. |
| `-P`, `--port <PORT>` | BNET-Listen-Port aus der Konfiguration überschreiben; üblich `9101`. Nicht mit dem kleingeschriebenen `-p` verwechseln. |
| `-b`, `--bind-address <ADRESSE>` | BNET-Bind-Adresse; Standard `0.0.0.0`. |
| `--name <NAME>` | Director-Namen aus der Konfiguration überschreiben. |
| `-m`, `--max-jobs <ANZAHL>` | `Maximum Concurrent Jobs` des Directors überschreiben. |
| `--tls-cert <DATEI>` | PEM-Zertifikat für die BNET-TLS-Konfiguration; zusammen mit `--tls-key` verwenden. |
| `--tls-key <DATEI>` | PEM-Schlüssel für die BNET-TLS-Konfiguration. |
| `--tls-ca <DATEI>` | Optionale PEM-CA für diese TLS-Konfiguration. Diese Optionen konfigurieren nicht das separat verwaltete gRPC-Zertifikat. |
| `--xc [AUSWAHL]`, `--export-config [AUSWAHL]` | Konfigurationsauszug ausgeben und beenden. Aktuell nur ausgewählte Felder von Director, Clients, Jobs, Storages und Pools; die optionale Auswahl wird noch nicht ausgewertet. Kein vollständiger Konfigurationsexport. |
| `--xs`, `--export-schema` | Aktuelles Katalogschema aus `/usr/lib/bareos/scripts/ddl` als JSON ausgeben und beenden. Kein Schema der Konfigurationsdirektiven. |

Folgende Optionen akzeptiert der Parser bereits, der Director wertet sie aktuell
aber noch nicht aus:

| Parameter | Vorgesehene Bedeutung; derzeit ohne Wirkung |
| --- | --- |
| `-s`, `--no-signals` | Signalbehandlung abschalten. |
| `-v`, `--verbose` | Ausführlichere Meldungen; mehrfach akzeptiert (`-vv`). |
| `-w`, `--working-directory <PFAD>` | Arbeitsverzeichnis überschreiben. Aktuell gilt weiterhin der Konfigurationswert. |
| `-r`, `--restore-only` | Nur Restore-Jobs annehmen. Aktuell keine Einschränkung dadurch. |

```bash
# Vor einem Neustart Konfiguration und Katalogzugang prüfen
odir -t -c /etc/bareos/bareos-dir.d

# Vordergrundbetrieb, z. B. zur Diagnose
odir -f -c /etc/bareos/bareos-dir.d -d 100 --dt

# Eigene PID-Datei beim Start und beim späteren Stoppen verwenden
odir -f -p /tmp/odir.pid
odir -k -p /tmp/odir.pid

# Debug-Level-Erklärung anzeigen
odir -d 999
```

## ofd

Cargo-Paket, Binary und eingebaute Hilfe heißen `ofd`. Dies ist der
eigene Rust File Daemon des Projekts. Der Konfigurationspfad bleibt für die
Bareos-Kompatibilität `/etc/bareos/bareos-fd.d`.

| Parameter | Erklärung und Standard |
| --- | --- |
| `--discover-podman` | Read-only conmon-Prozesserkennung nach Host-UID; JSON ausgeben und beenden. Startet keinen Daemon und gewährt keinen Podman-Zugriff. |
| `--conmon-executable <DATEI>` | Mit `--discover-podman` eine vertrauenswürdige root-eigene conmon-Installation auswählen, mehrfach verwendbar. Standard: vorhandene bekannte Installationspfade. |
| `--grpc-listen <IP:PORT>` | Zusätzlichen TLS-gRPC-Listener starten; BNET bleibt parallel als Fallback aktiv. Benötigt `--grpc-cert` und `--grpc-key`. |
| `--grpc-cert <DATEI>` | PEM-Serverzertifikat für den gRPC-Listener. Nur zusammen mit `--grpc-listen`. |
| `--grpc-key <DATEI>` | PEM-Serverschlüssel für den gRPC-Listener. Nur zusammen mit `--grpc-listen`. |
| `-c`, `--config <PFAD>` | Konfigurationsdatei oder Verzeichnis; Standard `/etc/bareos/bareos-fd.d`. |
| `-d`, `--debug-level <ZAHL>` | Debug-Level; Standard `0`. |
| `-f`, `--foreground` | Im Vordergrund laufen; sonst daemonisieren. |
| `-u`, `--user <BENUTZER>` | Benutzerrechte vor dem Serverstart wechseln. |
| `-g`, `--group <GRUPPE>` | Gruppe vor dem Serverstart wechseln. |
| `-t`, `--test-config` | Konfiguration einlesen und bei Erfolg beenden. Anders als beim Director keine Katalogprüfung. |
| `-s`, `--no-signals` | Akzeptiert, aktuell nicht ausgewertet. |
| `-v`, `--verbose` | Mehrfach akzeptiert, aktuell nicht ausgewertet. |

```bash
./target/release/ofd -t -c /etc/bareos/bareos-fd.d
./target/release/ofd -f -d 100
```

Steuertransport und Konfigurationsbeispiele: [File Daemon und gRPC-Fallback](file-daemon.md).
