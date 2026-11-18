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
- **Schlüssel in den TPM importieren wie beim Kernel** (Nutzer, 19.11.2026):
  Für die Kernel-Signatur werden Schlüssel bereits extern erzeugt und in den
  TPM importiert. Dasselbe Verfahren für den Plugin- und den Paket-
  Signierschlüssel: offline erzeugen, verschlüsselt sichern, mit `tpm2_import`
  in den TPM des Build-Rechners laden.

## Schlüssel und wo sie liegen

| Schlüssel | Ort | Gebraucht |
| --- | --- | --- |
| Plugin-CA (Ed25519) | **offline**, zwei verschlüsselte Kopien an verschiedenen Orten (z. B. USB-Stick zu Hause, Papier/zweiter Stick außer Haus) | selten: Signierzertifikat ausstellen, widerrufen |
| Plugin-Signierschlüssel (ECDSA P-256 oder RSA, wie beim Kernel) | offline erzeugt, zwei verschlüsselte Kopien; **in den TPM des Build-Rechners importiert** | bei jedem Plugin-Build |
| OpenPGP-Hauptschlüssel | offline, bei der CA | selten: Unterschlüssel erzeugen/widerrufen |
| OpenPGP-Signier-Unterschlüssel | offline erzeugt und gesichert, in den TPM importiert (GnuPG ≥ 2.3 `keytotpm`) | bei jedem Paket-/Repository-Build |
| OpenPGP-Reserve-Unterschlüssel | offline, bereits im Keyring-Paket veröffentlicht | nur bei Kompromittierung des Signier-Unterschlüssels |

ECDSA P-256 oder RSA für den Signierschlüssel, weil die meisten TPMs (gerade
Firmware-TPMs) kein Ed25519 können; vorzugsweise derselbe Algorithmus und
dieselben Werkzeuge wie bei der Kernel-Signatur. Die CA bleibt Ed25519.

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
2. **ECDSA P-256 und/oder RSA-PSS** für Manifest-Signaturen und Leaf-Schlüssel
   zusätzlich zu Ed25519 (Verifier; `sign-component` über PKCS#11 /
   `tpm2-pkcs11` oder den OpenSSL-Provider `tpm2`, wie bei der Kernel-Signatur).
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

1. Freigabe des Modells (CA offline, Signierschlüssel offline erzeugt und in
   den TPM importiert).
1a. Welcher Algorithmus und welche Werkzeuge bei der Kernel-Signatur im
    Einsatz sind (RSA oder ECDSA, `tpm2-pkcs11` oder OpenSSL-Provider), damit
    Plugins und Pakete dasselbe nutzen.
2. Laufzeit der Signierzertifikate (Vorschlag 3 Jahre) oder Zeitstempeldienst.
3. Wo die zweite Offline-Kopie liegt.
