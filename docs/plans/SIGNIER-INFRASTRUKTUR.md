# Planung: Signier-Infrastruktur (Plugins und Pakete)

Stand: 19.11.2026. Status: **Entwurf zur Freigabe.**

Zwei getrennte Signaturebenen, beide nötig:

| | Paket-Signatur (deb/rpm) | Plugin-Signatur (Bundle) |
| --- | --- | --- |
| Prüft | `apt` / `dnf` beim Installieren | `ofd` bei **jedem Laden** |
| Gegenstand | Repository (`InRelease`) bzw. jedes RPM | Manifest, WASM-Code, Hashes der Binärteile |
| Schützt gegen | manipulierte Downloads, gefälschte Repositories | Austausch nach der Installation, Plugins aus anderer Quelle, Skripte/LLM-Agenten mit Shellzugriff |
| Technik | OpenPGP | X.509, Ed25519 / ECDSA P-256 |

## Entscheidungen bisher

- **Vertrauensbasis in `ofd` einkompiliert** (Nutzer, 19.11.2026): das öffentliche
  CA-Zertifikat, die Grants und eine Widerrufsliste. Die lokale `Plugin Policy`
  darf nur noch **einschränken** (zusätzliche Widerrufe, engere Capabilities);
  eine nicht einkompilierte CA wird abgelehnt.
- **Kein Build-Host vorhanden.** Signiert wird auf einem Rechner, den der
  Nutzer kontrolliert; dieser hat einen TPM 2.0 (`/dev/tpmrm0`).

## Schlüssel und wo sie liegen

| Schlüssel | Ort | Gebraucht |
| --- | --- | --- |
| Plugin-CA (Ed25519) | **offline**, zwei verschlüsselte Kopien an verschiedenen Orten (z. B. USB-Stick zu Hause, Papier/zweiter Stick außer Haus) | selten: Signierzertifikat ausstellen, widerrufen |
| Plugin-Signierschlüssel (ECDSA P-256) | **TPM des Build-Rechners**, nicht exportierbar | bei jedem Plugin-Build |
| OpenPGP-Hauptschlüssel | offline, bei der CA | selten: Unterschlüssel erzeugen/widerrufen |
| OpenPGP-Signier-Unterschlüssel | TPM des Build-Rechners (GnuPG ≥ 2.3 `keytotpm`) oder Token | bei jedem Paket-/Repository-Build |
| OpenPGP-Reserve-Unterschlüssel | offline, bereits im Keyring-Paket veröffentlicht | nur nach Ausfall des Build-Rechners |

ECDSA P-256 für den Signierschlüssel, weil die meisten TPMs (gerade
Firmware-TPMs) kein Ed25519 können. Die CA bleibt Ed25519.

## Zerstörung oder Diebstahl des Build-Rechners

- Installierte Plugins und Pakete **laufen weiter**: Prüfen braucht keinen
  privaten Schlüssel.
- Neuer Rechner, neuer TPM-Schlüssel; die Offline-CA stellt ihm ein
  Signierzertifikat aus. Bei **Diebstahl** wird der alte Schlüssel widerrufen
  (eingebaute Widerrufsliste mit dem nächsten `ofd`-Release, sofort über die
  lokale Policy).
- Pakete: weiter mit dem Reserve-Unterschlüssel, den alle Clients schon kennen.
- Plugins werden mit dem neuen Schlüssel neu signiert (WASM unverändert, neues
  Manifest und Signatur) und als Paket-Update verteilt. **`ofd` muss nicht neu
  gebaut werden** – Voraussetzung ist Änderung 1 unten.
- Verlust der **CA** ist der eigentliche Ernstfall: dann neue CA, jedes `ofd`
  neu bauen und ausrollen. Deshalb zwei Offline-Kopien.

## Nötige Änderungen am Code

1. **Grants im Zertifikat statt an der Schlüssel-ID.** Heute bindet die Policy
   Grants an die SPKI-ID des Signierschlüssels; ein neuer Schlüssel bräuchte
   einen neuen Grant und damit ein neues `ofd`. Künftig bescheinigt die CA die
   erlaubten Plugin-IDs und Capabilities in einer X.509-Erweiterung des
   Signierzertifikats (eigene OID); `ofd` vertraut der einkompilierten CA und
   liest die Grants aus der geprüften Kette.
2. **ECDSA P-256** für Manifest-Signaturen und Leaf-Schlüssel zusätzlich zu
   Ed25519 (Verifier, `sign-component` über PKCS#11 / `tpm2-pkcs11`).
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
2. Build-Rechner: `tpm2-tools`/`tpm2-pkcs11` installieren, Signierschlüssel
   ECDSA P-256 im TPM erzeugen, CSR exportieren.
3. Offline: CSR mit der CA signieren (Grants als Erweiterung), Zertifikat
   zurück auf den Build-Rechner.
4. OpenPGP-Signier-Unterschlüssel in den TPM verlagern, Reserve offline.
5. CA-Zertifikat in den `ofd`-Build einbinden; Keyring-Paket bauen.

## Offene Fragen

1. Freigabe des Zwei-Stufen-Modells (CA offline, Signierschlüssel im TPM).
2. Laufzeit der Signierzertifikate (Vorschlag 3 Jahre) oder Zeitstempeldienst.
3. Wo die zweite Offline-Kopie liegt.
