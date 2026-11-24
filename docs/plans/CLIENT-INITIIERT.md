# Client-initiierte Verbindung (`Connection From Client To Director`)

Stand 24.11.2026. Gehört zu [S-46](../STUBS.md) und zum offenen Punkt „Run On Incoming Connect
Interval“ ([C-059](../COMPATIBILITY-LOG.md)). Der Plan fasst zusammen, was aus dem Quelltext von
Bareos 25 (`bareos-25`, gelesen am 24.11.2026) belegt ist und was noch fehlt, bevor etwas gebaut wird.

## Zweck

Ein Client hinter NAT oder einer Firewall kann den Director nicht von außen erreichen. Er verbindet
sich selbst zum Director und hält die Verbindung offen; der Director benutzt sie für den nächsten Job
dieses Clients, statt selbst zu verbinden. Für Onesimus Home ist das der Normalfall.

## Belegt (Bareos 25)

- **Hello des FD:** `Hello Client <Name> FdProtocolVersion=<n> calling\n` (`filed/dir_cmd.cc`). Der
  Director nimmt auch `Hello Client <Name> calling` ohne Protokollversion an (`dird/socket_server.cc`,
  `HandleConnectionRequest`); erlaubte Länge 15 bis `sizeof(name)+25`, sonst Fehler und `sleep(5)`.
- **Antwort des Directors:** `1000 OK: <Name> Version: <v> (<Datum>)\n`; Ablehnung
  `1999 You are not authorized.\n` mit `sleep(5)`; ohne Client-Resource oder bei nicht erlaubter
  Verbindung keine Antwort, nur eine Warnung. Der Director macht zuerst TLS als Server
  (`TryTlsHandshakeAsAServer`), dann `AuthenticateInboundConnection` mit dem Passwort der
  Client-Resource.
- **Registrierung:** `HandleFiledConnection` (`dird/fd_cmds.cc`): Client-Resource suchen
  (`R_CLIENT`), `IsConnectFromClientAllowed`, `AuthenticateFileDaemon`, dann in den Verbindungspool
  (`lib/connection_pool.h`): ein Vektor mit Name, Protokollversion und Verbindungszeit, mehrere
  Verbindungen je Name möglich. Es gibt keinen eigenen Herzschlag; die Prüfung ist
  `WaitDataIntr(0)` und `recv()` bei jedem Zugriff, `recv() <= 0` heißt tot. Bei `Run On Incoming
  Connect Interval` folgt sofort `RunOnIncomingConnectInterval(client).Run()`.
- **Nutzung durch einen Job:** `ConnectToFileDaemon` nimmt zuerst eine wartende Verbindung
  (`UseWaitingClient(jcr, client, 0)`), sonst verbindet es selbst und versucht danach noch einmal
  mit Wartezeit (`max_retry_time`). Der Pool gibt die neueste lebende Verbindung ab (von hinten).
  Danach schickt der Director die erste Jobnachricht (`JobId=… Job=… SDid=… SDtime=…
  Authorization=…`, FD antwortet `2000 OK Job …`).
- **FD-Seite:** Je Director-Resource mit dem Schalter (`conn_from_fd_to_dir`) ein Thread; Wiederholung
  nach Fehler fest nach 60 s; nach einem Job sofort eine neue Verbindung (immer eine Leerlauf-
  Verbindung je Director). Der FD wartet mit `WaitDataIntr(60)` auf Daten des Directors und führt dann
  die Befehle des Directors aus; `Hello Director` steht nicht in der Befehlstabelle des FD.

## Nicht belegt (vor dem Bau klären)

1. **Rollen der CRAM-MD5-Anmeldung** beider Seiten (wer Challenger ist, wer zweiseitig antwortet) und
   welches Passwort der FD benutzt (`AuthenticateInboundConnection` in `lib/`,
   `filed/authenticate.cc`).
2. **`UseWaitingClient`**: setzt es `jcr->authenticated`? Davon hängt ab, ob der Director auf dem
   übernommenen Socket noch `Hello Director` sendet (`AuthenticateWithFileDaemon` überspringt es bei
   `jcr->authenticated`; der FD würde `Hello Director` mit `2999 Invalid command` ablehnen).
3. **`dird/run_on_incoming_connect_interval.*`**: Direktivname, Einheit, Verhältnis zum Scheduler.
4. **Direktivnamen** und Vorgaben in beiden Konfigurationen (Client-Resource des Directors,
   Director-Resource des FD), Heartbeat-Vorgabe.
5. **Wann `CleanupConnectionPool` läuft** (Aufrufzeitpunkt).

Zu 1 bis 3 reicht ein Mitschnitt eines echten Laufs (Bareos-FD gegen `bareos-dir`), wie er für PAM
schon gemacht wurde (`onesimus-rig/tests/pam_capture.rs`). Auf dieser Maschine ist kein `bareos-fd`
installiert; er muss bereitgestellt werden (Paket `bareos-filedaemon`, Installation ist Sache des Nutzers).

## Umfang in Onesimus

- **odir:** `accept.rs` nimmt `Hello Client` an (heute: schließen), prüft Name und Passwort, hält die
  Verbindung in einem Pool (`client_pool.rs`, neu) mit der Lebensprüfung wie das Original; die
  Job-Worker (`fd_protocol::connect_fd…`, `backup_worker`, `restore`, `verify`, `estimate`) fragen den
  Pool vor dem eigenen Verbindungsaufbau; `status client` und `list`-Befehle bleiben unberührt.
  `Run On Incoming Connect Interval` im Scheduler (`scheduler.rs`).
- **ofd:** Direktive `Connection From Client To Director` in der Director-Resource (Schema,
  `config.rs`), ein Thread je Director, der verbindet, sich anmeldet und die Verbindung wie eine
  eingehende des Directors behandelt (gemeinsame Sitzungsschleife `accept.rs`), danach neu verbindet.
- **Prüfung:** Rig mit ofd ↔ odir (beide Onesimus), Mitschnitt gegen Bareos als Gegenprobe, Fälle
  für Fehlerpfade (falsches Passwort, unbekannter Client, Verbindung stirbt im Pool, zwei Verbindungen).

## Reihenfolge

1. Mitschnitt mit einem Bareos-FD beschaffen (Punkte 1 bis 3).
2. odir-Seite mit einem Test-Client (Rig) bauen.
3. ofd-Seite.
4. `Run On Incoming Connect Interval`.
