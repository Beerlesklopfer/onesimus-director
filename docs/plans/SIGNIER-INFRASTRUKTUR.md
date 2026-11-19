# Planung: Signier-Infrastruktur (Plugins und Pakete)

Stand: 19.11.2026. Status: **Freigegeben** (Nutzer, 19.11.2026: „GO!“). Offen sind nur noch die Laufzeit der Signierzertifikate und der Ort der zweiten Offline-Kopie.

Zwei getrennte Signaturebenen, beide nötig:

| | Paket-Signatur (deb/rpm) | Plugin-Signatur (Bundle) |
| --- | --- | --- |
| Prüft | `apt` / `dnf` beim Installieren | `ofd` bei **jedem Laden** |
| Gegenstand | Repository (`InRelease`) bzw. jedes RPM | Manifest, WASM-Code, Hashes der Binärteile |
| Schützt gegen | manipulierte Downloads, gefälschte Repositories | Austausch nach der Installation, Plugins aus anderer Quelle, Skripte/LLM-Agenten mit Shellzugriff |
| Technik | OpenPGP | X.509: CA Ed25519, Signierschlüssel RSA 4096 (RSA-PSS/SHA-256) |

## Entscheidungen bisher

- **Vertrauensbasis in `ofd` einkompiliert** (Nutzer, 19.11.2026): das öffentliche
  CA-Zertifikat, die Grants und eine Widerrufsliste. Die lokale `Plugin Policy`
  darf nur noch **einschränken** (zusätzliche Widerrufe, engere Capabilities);
  eine nicht einkompilierte CA wird abgelehnt.
- **Kein Build-Host vorhanden.** Signiert wird auf einem Rechner, den der
  Nutzer kontrolliert; dieser hat einen TPM 2.0 (`/dev/tpmrm0`).
- **Schlüssel in den TPM importieren wie beim Kernel** (Nutzer, 19.11.2026):
  Für die Kernel-Signatur werden Schlüssel bereits extern erzeugt und in den
  TPM importiert. Dasselbe Verfahren für den Plugin- und den Paket-
  Signierschlüssel: offline erzeugen, verschlüsselt sichern, mit `tpm2_import`
  in den TPM des Build-Rechners laden.
- **Algorithmus und Werkzeug wie beim Kernel** (Nutzer, 19.11.2026):
  **RSA 4096**, signiert über den **OpenSSL-Provider `tpm2`**.

- **Wechsel des Signierschlüssels = Versionswechsel der kompletten Suite**
  (Nutzer, 19.11.2026). Signierzertifikate werden als neues Glied in die Kette
  gehängt; ein Wechsel MUSS zwangsläufig ein gemeinsames Release von `ofd`,
  `odir` und allen Plugins nach sich ziehen.

### Umsetzung der Suite-Bindung (Vorschlag)

- Je Generation ein Zwischenzertifikat der Offline-Root
  („Onesimus Plugin Signing G<n>“), darunter das Signierzertifikat.
- In `ofd` einkompiliert: Root und die Generation dieses Releases.
- Manifest (Schema 3): Feld `suite` (Suite-Version). `ofd` lädt nur Bundles,
  deren Kette über die Generation dieses Releases läuft **und** deren `suite`
  zur eigenen Version passt. Alte Bundles laden mit neuem `ofd` nicht und
  umgekehrt – der Versionswechsel der Suite ist damit erzwungen.
- **Suite-Version = Onesimus-Teil** der Workspace-Version (Nutzer,
  19.11.2026): bei `25.1.2+onesimus.0.2.0` ist das `0.2.0`; das Bareos-
  Kompatibilitätspräfix zählt nicht. `ofd` vergleicht `suite` des Manifests
  exakt mit seinem eigenen Onesimus-Teil.
- **Harter Schnitt** (Nutzer, 19.11.2026): `ofd` akzeptiert genau eine
  Generation, die seines Releases; kein Übergangsfenster. Ein Host mit neuem
  `ofd` braucht die neu signierten Plugins derselben Suite-Version –
  Paketabhängigkeiten erzwingen das (`onesimus-plugin-*` hängt von
  `ofd (= <Version>)` ab und umgekehrt über `Breaks:` älterer Plugins).

## Schlüssel und wo sie liegen

| Schlüssel | Ort | Gebraucht |
| --- | --- | --- |
| Plugin-CA (Ed25519) | **offline**, zwei verschlüsselte Kopien an verschiedenen Orten (z. B. USB-Stick zu Hause, Papier/zweiter Stick außer Haus) | selten: Signierzertifikat ausstellen, widerrufen |
| Plugin-Signierschlüssel (RSA 4096, wie beim Kernel) | offline erzeugt, zwei verschlüsselte Kopien; **in den TPM des Build-Rechners importiert** | bei jedem Plugin-Build |
| OpenPGP-Hauptschlüssel | offline, bei der CA | selten: Unterschlüssel erzeugen/widerrufen |
| OpenPGP-Signier-Unterschlüssel | offline erzeugt und gesichert, in den TPM importiert (GnuPG ≥ 2.3 `keytotpm`) | bei jedem Paket-/Repository-Build |
| OpenPGP-Reserve-Unterschlüssel | offline, bereits im Keyring-Paket veröffentlicht | nur bei Kompromittierung des Signier-Unterschlüssels |

RSA 4096 für den Signierschlüssel, wie bei der Kernel-Signatur (TPMs können
meist kein Ed25519). Die CA bleibt Ed25519.

## Zerstörung oder Diebstahl des Build-Rechners

- Installierte Plugins und Pakete **laufen weiter**: Prüfen braucht keinen
  privaten Schlüssel.
- **Zerstörung:** Derselbe Signierschlüssel wird aus der Offline-Kopie in den
  TPM des neuen Rechners importiert. Kein neues Zertifikat, kein Neusignieren,
  kein neues `ofd`.
- **Diebstahl** (der Dieb hat den TPM und könnte signieren): neuer
  Signierschlüssel, die Offline-CA stellt ihm ein Zertifikat aus, der alte
  wird widerrufen (eingebaute Widerrufsliste mit dem nächsten `ofd`-Release,
  sofort über die lokale Policy). Pakete: weiter mit dem Reserve-Unterschlüssel.
  Plugins werden neu signiert und als Paket-Update verteilt; ohne Neubau von
  `ofd` nur mit Änderung 1 unten.
- Verlust der **CA** ist der eigentliche Ernstfall: dann neue CA, jedes `ofd`
  neu bauen und ausrollen. Deshalb zwei Offline-Kopien.

## Nötige Änderungen am Code

1. **Grants im Zertifikat statt an der Schlüssel-ID** (durch den Import nicht
   mehr für den Ausfall nötig, nur noch für den Wechsel nach Kompromittierung).
   Heute bindet die Policy
   Grants an die SPKI-ID des Signierschlüssels; ein neuer Schlüssel bräuchte
   einen neuen Grant und damit ein neues `ofd`. Künftig bescheinigt die CA die
   erlaubten Plugin-IDs und Capabilities in einer X.509-Erweiterung des
   Signierzertifikats (eigene OID); `ofd` vertraut der einkompilierten CA und
   liest die Grants aus der geprüften Kette.
2. **RSA 4096 mit RSA-PSS/SHA-256** für Manifest-Signaturen zusätzlich zu
   Ed25519. Der Verifier wählt das Verfahren nach dem Schlüsseltyp des
   geprüften Leaf-Zertifikats (Ed25519: 64 Byte; RSA: genau 4096 Bit, PSS,
   SHA-256, Salt-Länge = Digest-Länge; andere Längen und PKCS#1 v1.5 werden
   abgelehnt). Das Proto-Feld `ed25519_signature` trägt dann die jeweilige
   Signatur; der Name wird in einer späteren Proto-Version bereinigt.
   **`sign-component` bekommt nie einen privaten Schlüssel:** Es wird geteilt
   in `prepare` (Manifest-Bytes erzeugen) und `assemble` (Bundle aus Manifest,
   externer Signaturdatei und Zertifikatskette). Signiert wird dazwischen mit
   `openssl dgst -sha256 -sign handle:<TPM-Handle> -provider tpm2
   -provider default -sigopt rsa_padding_mode:pss -sigopt rsa_pss_saltlen:digest`,
   wie bei der Kernel-Signatur. `assemble` prüft die Signatur vor dem Schreiben.
3. **Einkompilierte Vertrauensbasis:** Pfad zum CA-Zertifikat (und
   Widerrufsliste) als Build-Variable (`ONESIMUS_PLUGIN_CA`), `include_bytes!`.
   Ohne sie hat ein Build keine eingebaute Vertrauensbasis (nur Tests mit
   eigener Test-CA). Lokale Policy: nur Einschränkungen, sonst Ladefehler.
4. **Gültigkeit:** `ofd` prüft die Zertifikatsgültigkeit beim Laden. Ohne
   Zeitstempeldienst heißt das: Signierzertifikate mit langer Laufzeit
   (Vorschlag 3 Jahre) und Neusignieren rechtzeitig vor Ablauf. Alternative
   später: TSA-Zeitstempel.
5. **`ofd --check-plugin` für Broker-Plugins:** endet nach Signatur, Manifest
   und Hashes der Binärteile, statt an der Instanziierung ohne Broker zu
   scheitern.

## Pakete

- `onesimus-archive-keyring`: öffentlicher OpenPGP-Schlüssel mit aktivem und
  Reserve-Unterschlüssel nach `/usr/share/keyrings/`.
- `onesimus-plugin-<name>` je Architektur (`Architecture: any`,
  `Multi-Arch: same`): Bundle und Binärteile unter
  `/usr/lib/<triplet>/onesimus/plugins/`; das Paket gibt sich nicht selbst frei
  (`Plugin Names` setzt der Administrator).
- RPM: derselbe OpenPGP-Schlüssel (`rpm --addsign`, `gpgcheck=1`).

## Ablauf (Anleitung folgt nach Freigabe)

1. Offline: CA (Ed25519) und OpenPGP-Hauptschlüssel erzeugen, zwei Kopien.
2. Offline: Plugin-Signierschlüssel erzeugen, CSR, mit der CA signieren
   (Grants als Erweiterung); Schlüssel verschlüsselt zweifach sichern.
3. Build-Rechner: Signierschlüssel mit `tpm2_import` in den TPM laden (wie
   beim Kernel), Zertifikat dazulegen.
4. OpenPGP-Signier-Unterschlüssel in den TPM verlagern, Reserve offline.
5. CA-Zertifikat in den `ofd`-Build einbinden; Keyring-Paket bauen.

## Offene Fragen

1. ~~Freigabe des Modells~~ – freigegeben (Nutzer, 19.11.2026): CA offline,
   Signierschlüssel offline erzeugt und in den TPM importiert.
1a. ~~Algorithmus und Werkzeug~~ – entschieden: RSA 4096, OpenSSL-Provider `tpm2`.
2. Laufzeit der Signierzertifikate (Vorschlag 3 Jahre) oder Zeitstempeldienst.
3. Wo die zweite Offline-Kopie liegt.
