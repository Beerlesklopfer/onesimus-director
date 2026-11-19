# Planung: Signier-Infrastruktur (Plugins und Pakete)

Stand: 19.11.2026. Status: **Freigegeben** (Nutzer, 19.11.2026: „GO!“).
**Code-Änderungen 1–5 umgesetzt** (19.11.2026); Paketbau (Schritt G) und die
Schritte mit TPM am echten System stehen aus. Offen ist außerdem der Ort der
zweiten Offline-Kopie (Sache des Nutzers, nicht im Repository).

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
- **Laufzeit der Signierzertifikate 3 Jahre**, kein Zeitstempeldienst
  (Nutzer, 19.11.2026). Die Gültigkeit aller Kettenglieder wird bei jedem
  Laden geprüft.
- **Pakete heißen `ofd-plugin-<name>`** (Nutzer, 19.11.2026), z. B.
  `ofd-plugin-postgresql`, nicht `onesimus-plugin-<name>`.

- **Wechsel des Signierschlüssels = Versionswechsel der kompletten Suite**
  (Nutzer, 19.11.2026). Signierzertifikate werden als neues Glied in die Kette
  gehängt; ein Wechsel MUSS zwangsläufig ein gemeinsames Release von `ofd`,
  `odir` und allen Plugins nach sich ziehen.

### Umsetzung der Suite-Bindung (umgesetzt)

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
  Paketabhängigkeiten erzwingen das (`ofd-plugin-*` hängt von
  `ofd (= <Version>)` ab und umgekehrt über `Breaks:` älterer Plugins).

### Festlegungen bei der Umsetzung (19.11.2026)

- **OID der Grants-Erweiterung: `2.25.166226948137511682553772901840964231856.1`.**
  `2.25.<UUID>` ist der UUID-Bogen (ITU-T X.667 / ISO/IEC 9834-8, RFC 4122
  Abschnitt 4.4), den jeder ohne Registrierung aus einer zufälligen UUID
  (`7d0e254c-7a67-46bf-9f89-baee512f22b0`) ableiten darf; `.1` darunter ist
  „Plugin-Grants“. Inhalt (DER):
  `SEQUENCE { version INTEGER (1), pluginIds SEQUENCE SIZE(1..64) OF UTF8String,
  capabilities SEQUENCE SIZE(0..64) OF UTF8String }`, Namen `[a-z0-9._-]{1,128}`,
  keine Doppelten. Eine Änderung der OID macht jedes Signierzertifikat
  ungültig, ist also ein Generationswechsel.
- **Schema 1 und 2 laden nicht mehr.** Sie werden weiter eingelesen und
  geprüft (Felder, `native`-Regeln), aber `verify` lehnt sie ab („anderer
  Suite-Stand“), weil sie kein `suite` tragen. Begründung: Der Plan verlangt
  „Alte Bundles laden mit neuem `ofd` nicht“ und den harten Schnitt ohne
  Übergangsfenster; ein Bundle ohne `suite` kann die exakte Suite-Prüfung
  nicht bestehen. Schema 3 darf `native` haben oder nicht.
- **`ONESIMUS_PLUGIN_CA`** ist ein **absoluter Verzeichnispfad** mit drei
  Pflichtdateien: `root.pem`, `generation.pem` (je genau ein Zertifikat),
  `revoked.json` (`{}` wenn leer; Felder `revoked_keys`,
  `revoked_certificates` als SHA-256-Hex, `revoked_plugins`). `build.rs`
  bettet sie per `include_bytes!` ein, `ofd` prüft sie bei jedem Laden
  (Root selbstsigniert, Ed25519, CA; Generation von der Root ausgestellt).
- **Kette:** angenommen wird nur Leaf → einkompilierte Generation →
  einkompilierte Root (byte-genau). Das Bundle trägt das Leaf und darf die
  einkompilierte Generation wiederholen; jedes andere Zwischenzertifikat im
  Bundle ist ein Ladefehler. Ein direkt von der Root ausgestelltes Leaf wird
  abgelehnt.
- **Signaturverfahren nach Schlüsseltyp des Leafs:** Ed25519 (64 Byte) oder
  RSA (`rsaEncryption`) mit genau 4096 Bit, RSA-PSS, SHA-256, MGF1-SHA-256,
  Salt-Länge = 32. PKCS#1 v1.5, andere Salt-Längen, andere Schlüssellängen
  und RSA-PSS-beschränkte Schlüssel (`id-RSASSA-PSS`) werden abgelehnt.
- **`sign-component`** (`crates/onesimus-plugins/examples/sign-component.rs`)
  kennt nur `prepare` und `assemble`; die Logik liegt in
  `onesimus_plugins::signing`. `assemble --trust-dir <Verzeichnis wie
  ONESIMUS_PLUGIN_CA>` führt zusätzlich die volle Prüfung von `ofd` aus.

### Vorschläge, offen (konservativ umgesetzt, Freigabe ausstehend)

1. **`Plugin Policy` bleibt Pflicht** für Plugins in Jobs, ebenso
   `--plugin-policy` für `ofd --check-plugin`, obwohl die Vertrauensbasis
   einkompiliert ist. Inhalt darf `{}` sein. Sie ist damit eine zweite
   Freigabe des Administrators neben `Plugin Names`.
2. **Format der lokalen Policy** (alle Felder optional, nur einschränkend):
   `plugin_ids` (nur diese laden), `allowed_capabilities` (Obergrenze),
   `revoked_keys`/`revoked_certificates`/`revoked_plugins` (zusätzlich zur
   eingebauten Liste), `valid_from`+`valid_until` (nur zusammen),
   `target` (nur `wasm32-wasip2`), `ca_certificates` (nur zum Festnageln:
   jede Datei muss ein einkompiliertes Zertifikat enthalten). Ladefehler:
   fremde CA, `signers` (Grants alter Form), anderes Ziel, unbekannte Felder,
   Widerrufe, die kein 64-stelliges Kleinbuchstaben-Hex sind.
3. **Die Grants-Erweiterung ist nicht kritisch markiert**, aber Pflicht:
   OpenSSL würde eine unbekannte kritische Erweiterung ablehnen, `ofd`
   verlangt sie ohnehin genau einmal.
4. **Root muss Ed25519 sein** (wird geprüft); der Schlüsseltyp der
   Generation wird nicht erzwungen (Anleitung: Ed25519).
5. **Ein `ofd` ohne `+onesimus.`-Teil in der Version lädt kein Plugin.**
6. **`prepare` setzt die Manifest-Gültigkeit standardmäßig auf die des
   Signierzertifikats** (`--not-before`/`--not-after` zum Überschreiben).
7. **`--check-plugin` prüft bei Binärteilen Hash und reguläre Datei**
   (`O_NOFOLLOW`), nicht Eigentümer und Rechte; das tut erst der Job-Pfad
   vor dem Start. So läuft der Check auch vor der Freigabe im `postinst`.
8. **Ed25519-Signierzertifikate bleiben zulässig** („zusätzlich zu
   Ed25519“). Wer nur RSA 4096 will, stellt einfach keine Ed25519-Leafs aus.
9. **`sign-component` bleibt ein Beispielprogramm** des Crates (kein
   installiertes Werkzeug), bis der Paketbau es braucht.

## Schlüssel und wo sie liegen

| Schlüssel | Ort | Gebraucht |
| --- | --- | --- |
| Plugin-CA (Ed25519) | **offline**, zwei verschlüsselte Kopien an verschiedenen Orten (z. B. USB-Stick zu Hause, Papier/zweiter Stick außer Haus) | selten: Generation und Signierzertifikat ausstellen, widerrufen |
| Generation G<n> (Ed25519) | offline bei der CA, ebenso gesichert | Signierzertifikate dieser Generation ausstellen |
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
  Plugins werden neu signiert und als Paket-Update verteilt; dank Änderung 1
  (Grants im Zertifikat) technisch ohne Neubau von `ofd`, nach der
  Entscheidung „Wechsel = Versionswechsel der Suite“ aber mit neuer
  Generation und gemeinsamem Release (siehe Ablauf, Schritt 9).
- Verlust der **CA** ist der eigentliche Ernstfall: dann neue CA, jedes `ofd`
  neu bauen und ausrollen. Deshalb zwei Offline-Kopien.

## Nötige Änderungen am Code (umgesetzt 19.11.2026)

1. **Grants im Zertifikat statt an der Schlüssel-ID** – umgesetzt
   (`trust.rs`, `certificate.rs`): Die CA bescheinigt Plugin-IDs und
   Capabilities in der Erweiterung `2.25.166226948137511682553772901840964231856.1`
   des Signierzertifikats; `ofd` liest sie aus der geprüften Kette. Die Policy
   kennt keine `signers` mehr.
2. **RSA 4096 mit RSA-PSS/SHA-256** zusätzlich zu Ed25519 – umgesetzt. Das
   Proto-Feld `ed25519_signature` trägt die jeweilige Signatur; der Name wird
   in einer späteren Proto-Version bereinigt. **`sign-component` bekommt nie
   einen privaten Schlüssel:** `prepare` (Manifest-Bytes) und `assemble`
   (Bundle aus Manifest, externer Signaturdatei und Kette; prüft die
   Signatur vor dem Schreiben). Signiert wird dazwischen mit
   `openssl dgst -sha256 -sign handle:<TPM-Handle> -provider tpm2
   -provider default -sigopt rsa_padding_mode:pss -sigopt rsa_pss_saltlen:digest`,
   wie bei der Kernel-Signatur.
3. **Einkompilierte Vertrauensbasis** – umgesetzt (`crates/ofd/build.rs`,
   `crates/ofd/src/plugin/trust.rs`): `ONESIMUS_PLUGIN_CA`, `include_bytes!`.
   Ohne sie hat ein Build keine eingebaute Vertrauensbasis und lädt kein
   Plugin; nur die Tests des Crates nehmen ihre Test-Hierarchie
   (`cfg(test)`, Feature `test-ca` von `onesimus-plugins`). Lokale Policy:
   nur Einschränkungen, sonst Ladefehler.
4. **Gültigkeit** – umgesetzt: Leaf, Generation und Root werden bei jedem
   Laden zur aktuellen Zeit geprüft. Ohne Zeitstempeldienst: Signier-
   zertifikate 3 Jahre, Neusignieren rechtzeitig vor Ablauf.
5. **`ofd --check-plugin` für Broker-Plugins** – umgesetzt: endet nach
   Signatur, Manifest und Hashes der Binärteile (Triplet des Hosts) mit
   „not initialized (needs a broker)“.

Manifest Schema 3 (Beispiel):

```json
{
  "schema_version": 3,
  "plugin_id": "org.onesimus.postgresql",
  "version": "1.0.0",
  "api_version": 1,
  "artifact_sha256": "…",
  "signing_key_id": "<SHA-256 des SPKI des Signierzertifikats>",
  "not_before": 1795000000,
  "not_after": 1889600000,
  "capabilities": ["file.read", "postgresql.read"],
  "target": "wasm32-wasip2",
  "native": {"x86_64-linux-gnu": [{"path": "postgresql/onesimus-pg-broker", "sha256": "…"}]},
  "suite": "0.2.0"
}
```

## Pakete

- `onesimus-archive-keyring`: öffentlicher OpenPGP-Schlüssel mit aktivem und
  Reserve-Unterschlüssel nach `/usr/share/keyrings/`.
- `ofd-plugin-<name>` je Architektur (`Architecture: any`,
  `Multi-Arch: same`): Bundle und Binärteile unter
  `/usr/lib/<triplet>/onesimus/plugins/`; das Paket gibt sich nicht selbst frei
  (`Plugin Names` setzt der Administrator).
- RPM: derselbe OpenPGP-Schlüssel (`rpm --addsign`, `gpgcheck=1`).

## Ablauf: Schritt für Schritt

Die Befehle sind mit Wegwerf-Testschlüsseln auf dem Entwicklungsrechner
nachgespielt (OpenSSL 3.5, ohne TPM), außer wo **[ungetestet]** steht:
alles mit `tpm2_*` und `-provider tpm2` war hier nicht ausführbar.
Passphrasen fragt OpenSSL interaktiv ab, wenn `-pass`/`-passin` fehlt; so
ist es gedacht (nichts in die Shell-History).

### 1. Offline-Rechner vorbereiten

Ein Rechner ohne Netz (oder ein Live-System), Arbeitsverzeichnis auf einem
RAM-Dateisystem, damit unverschlüsselte Schlüssel nie auf eine Platte kommen:

```sh
mkdir -p /dev/shm/onesimus-ca && cd /dev/shm/onesimus-ca && umask 077
```

### 2. Offline-CA (Ed25519) und Generation G1 erzeugen

`root.cnf`:

```ini
[req]
distinguished_name = dn
prompt             = no
[dn]
O  = Onesimus
CN = Onesimus Plugin Root CA
[v3_root]
basicConstraints       = critical, CA:TRUE
keyUsage               = critical, keyCertSign, cRLSign
subjectKeyIdentifier   = hash
authorityKeyIdentifier = keyid:always
[v3_generation]
basicConstraints       = critical, CA:TRUE, pathlen:0
keyUsage               = critical, keyCertSign, cRLSign
extendedKeyUsage       = codeSigning
subjectKeyIdentifier   = hash
authorityKeyIdentifier = keyid:always
```

```sh
# Root: Ed25519, privater Schlüssel sofort verschlüsselt (PKCS#8, AES-256), 20 Jahre
openssl genpkey -algorithm ed25519 -aes-256-cbc -out root.key
openssl req -new -x509 -config root.cnf -extensions v3_root -key root.key -days 7305 -out root.pem

# Generation G1: eigener Ed25519-Schlüssel, von der Root ausgestellt, 10 Jahre
openssl genpkey -algorithm ed25519 -aes-256-cbc -out g1.key
openssl req -new -key g1.key -subj "/O=Onesimus/CN=Onesimus Plugin Signing G1" -out g1.csr
openssl x509 -req -in g1.csr -CA root.pem -CAkey root.key -set_serial "0x$(openssl rand -hex 16)" \
    -days 3653 -extfile root.cnf -extensions v3_generation -out g1.pem
openssl verify -x509_strict -CAfile root.pem g1.pem
```

Die Generation muss mindestens so lange gelten wie die Signierzertifikate,
die sie ausstellt (3 Jahre plus Reserve für Neusignieren).

### 3. Signierschlüssel (RSA 4096) offline erzeugen, CSR mit Grants, mit G1 signieren

`signer.cnf` – die Grants (Plugin-IDs, Capabilities) stehen im Abschnitt
`onesimus_grants`; die Liste ist ein Beispiel:

```ini
[req]
distinguished_name = dn
prompt             = no
req_extensions     = v3_signer_req
[dn]
O  = Onesimus
CN = Onesimus Plugin Signer G1
[v3_signer_req]
basicConstraints = critical, CA:FALSE
keyUsage         = critical, digitalSignature
extendedKeyUsage = codeSigning
2.25.166226948137511682553772901840964231856.1 = ASN1:SEQUENCE:onesimus_grants
[v3_signer]
basicConstraints       = critical, CA:FALSE
keyUsage               = critical, digitalSignature
extendedKeyUsage       = codeSigning
subjectKeyIdentifier   = hash
authorityKeyIdentifier = keyid:always
2.25.166226948137511682553772901840964231856.1 = ASN1:SEQUENCE:onesimus_grants
[onesimus_grants]
version      = INTEGER:1
plugin_ids   = SEQUENCE:onesimus_plugin_ids
capabilities = SEQUENCE:onesimus_capabilities
[onesimus_plugin_ids]
p1 = UTF8String:org.onesimus.example-tree
p2 = UTF8String:org.onesimus.postgresql
p3 = UTF8String:org.onesimus.openldap
[onesimus_capabilities]
c1 = UTF8String:file.read
c2 = UTF8String:file.write
c3 = UTF8String:postgresql.read
c4 = UTF8String:openldap.read
c5 = UTF8String:openldap.write
```

```sh
openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:4096 -out signer.key   # nur in /dev/shm
openssl req -new -config signer.cnf -key signer.key -out signer.csr
openssl req -in signer.csr -noout -verify

# Die CA übernimmt die Erweiterungen aus ihrer eigenen Datei (-extfile), nicht
# aus dem CSR: maßgeblich ist, was die CA bescheinigt. 3 Jahre = 1095 Tage.
openssl x509 -req -in signer.csr -CA g1.pem -CAkey g1.key -set_serial "0x$(openssl rand -hex 16)" \
    -days 1095 -extfile signer.cnf -extensions v3_signer -out signer.pem
openssl verify -x509_strict -CAfile root.pem -untrusted g1.pem signer.pem
openssl x509 -in signer.pem -noout -text     # Grants unter 2.25.1662…1 kontrollieren
cat signer.pem g1.pem > chain.pem            # Kette für das Bundle (Leaf zuerst)
openssl pkey -in signer.key -pubout -out signer-pub.pem
```

### 4. Schlüssel zweifach verschlüsselt sichern

Zwei Schichten mit verschiedenen Passphrasen, und davon zwei Kopien an
verschiedenen Orten (Root und G1 sind bereits mit Schicht 1 erzeugt):

```sh
# Schicht 1: PKCS#8, AES-256 (OpenSSL); Schicht 2: OpenPGP symmetrisch (AES-256)
openssl pkey -in signer.key -aes-256-cbc -out signer.key.p8
for k in root.key g1.key signer.key.p8; do gpg --symmetric --cipher-algo AES256 -o "$k.gpg" "$k"; done
# Auf zwei Datenträger kopieren (Ort 1 und Ort 2), danach prüfen:
gpg -d signer.key.p8.gpg | openssl pkey -noout && echo lesbar
```

Öffentlich und unkritisch: `root.pem`, `g1.pem`, `signer.pem`, `chain.pem`,
`signer-pub.pem`.

### 5. Signierschlüssel in den TPM laden und persistent machen **[ungetestet]**

Auf dem Build-Rechner, wie beim Kernel-Schlüssel (dieselben Eltern-Parameter
wie dort verwenden; das Handle darf nicht mit dem Kernel-Schlüssel
kollidieren, vorher `tpm2_getcap handles-persistent`):

```sh
cd /dev/shm/onesimus-ca && umask 077
gpg -d signer.key.p8.gpg | openssl pkey -out signer.key          # Klartext nur im RAM
tpm2_createprimary -C o -g sha256 -G rsa -c primary.ctx
tpm2_import -C primary.ctx -G rsa -i signer.key -a "sign|userwithauth" -u signer.tpm.pub -r signer.tpm.priv
tpm2_load -C primary.ctx -u signer.tpm.pub -r signer.tpm.priv -c signer.ctx
tpm2_evictcontrol -C o -c signer.ctx 0x81000010
shred -u signer.key; rm -f primary.ctx signer.ctx signer.tpm.priv signer.tpm.pub
tpm2_readpublic -c 0x81000010                                     # RSA 4096?
```

Ein Schlüssel-Passwort (`tpm2_import -p …`) wird beim Signieren als
`handle:0x81000010?pass` mit `-passin` angegeben. `signer.pem` und
`chain.pem` kommen neben das Build-Verzeichnis.

**Am echten TPM prüfen:** Der Verifier verlangt PSS mit Salt-Länge = 32
(Digest-Länge). TPMs nach Spezifikation ≥ 1.38 signieren so; ältere TPMs
nehmen teils die größtmögliche Salt-Länge. Die Kontrolle unten (Schritt 6,
`openssl dgst -verify … rsa_pss_saltlen:digest`) und `assemble` zeigen das
sofort.

### 6. Ein Plugin signieren: `prepare` → `openssl -provider tpm2` → `assemble`

Im Repository (gleicher Stand wie das `ofd`-Release, damit `suite` passt):

```sh
(cd plugins/postgresql && CARGO_TARGET_DIR=../../target/plugins cargo +1.99.1 build --release --target wasm32-wasip2)
cargo build --release -p onesimus-pg-broker
W=target/plugins/wasm32-wasip2/release/onesimus_plugin_postgresql.wasm

cargo run -p onesimus-plugins --example sign-component -- prepare \
    --component "$W" --cert signer.pem \
    --plugin-id org.onesimus.postgresql --version 1.0.0 \
    --capability file.read --capability postgresql.read \
    --native x86_64-linux-gnu:postgresql/onesimus-pg-broker=target/release/onesimus-pg-broker \
    --out manifest.json            # --suite: Vorgabe ist die Suite dieses Arbeitsstands

# [ungetestet] Signatur im TPM (Kernel-Verfahren):
openssl dgst -sha256 -sign handle:0x81000010 -provider tpm2 -provider default \
    -sigopt rsa_padding_mode:pss -sigopt rsa_pss_saltlen:digest -out manifest.sig manifest.json
# (getestet mit Schlüsseldatei statt TPM: -sign signer.key ohne -provider)

openssl dgst -sha256 -verify signer-pub.pem -sigopt rsa_padding_mode:pss -sigopt rsa_pss_saltlen:digest \
    -signature manifest.sig manifest.json

cargo run -p onesimus-plugins --example sign-component -- assemble \
    --component "$W" --manifest manifest.json --signature manifest.sig --chain chain.pem \
    --trust-dir /pfad/zu/plugin-ca-g1 --out postgresql.pb
```

Für mehrere Architekturen je Triplet ein `--native …` in demselben
`prepare`; das Bundle ist für alle Architekturen gleich. Für jede weitere
Architektur muss der Binärteil vor `prepare` gebaut sein.

### 7. `ofd` mit `ONESIMUS_PLUGIN_CA` bauen

```sh
mkdir -p /pfad/zu/plugin-ca-g1
cp root.pem /pfad/zu/plugin-ca-g1/root.pem
cp g1.pem   /pfad/zu/plugin-ca-g1/generation.pem
echo '{}' > /pfad/zu/plugin-ca-g1/revoked.json
ONESIMUS_PLUGIN_CA=/pfad/zu/plugin-ca-g1 cargo build --release -p ofd
# Kontrolle (Bundle und Broker liegen im selben Verzeichnis, Policy darf {} sein):
./target/release/ofd --check-plugin /usr/lib/x86_64-linux-gnu/onesimus/plugins/postgresql.pb \
    --plugin-policy /etc/onesimus/plugin-policy.json
```

Erwartet: `Plugin org.onesimus.postgresql 1.0.0 (suite 0.2.0, Onesimus Plugin
Signing G1): X.509, signature, manifest and 1 native part(s) for
x86_64-linux-gnu OK; not initialized (needs a broker)`. Ein Build ohne die
Variable meldet „built without ONESIMUS_PLUGIN_CA“ und lädt kein Plugin.
Der Pfad muss absolut sein; `cargo deb` braucht dieselbe Variable.

### 8. Widerruf

```sh
# Fingerabdrücke (SHA-256, Kleinbuchstaben-Hex)
openssl x509 -in signer.pem -pubkey -noout | openssl pkey -pubin -outform DER | sha256sum   # Schlüssel (SPKI)
openssl x509 -in signer.pem -outform DER | sha256sum                                         # Zertifikat
```

- **Sofort auf den Hosts** über die lokale Policy (wirkt beim nächsten Job):
  `{"revoked_keys": ["<spki>"], "revoked_certificates": ["<zert>"],
  "revoked_plugins": ["org.onesimus.…"]}` in `/etc/onesimus/plugin-policy.json`
  (root-eigen, Modus 0644).
- **Dauerhaft** in `revoked.json` des Build-Verzeichnisses (gleiche Felder)
  und mit dem nächsten `ofd`-Release ausliefern.
- Bei **Kompromittierung des Schlüssels** immer den Schlüssel (`revoked_keys`)
  sperren, nicht nur das Zertifikat; danach Generationswechsel (Schritt 9).

### 9. Erneuerung und Generationswechsel

- **Erneuerung vor Ablauf** (Schlüssel nicht kompromittiert, ca. 3 Monate
  vor Ende): aus demselben CSR mit G1 ein neues `signer.pem` ausstellen
  (Schritt 3, letzter `openssl x509 -req`), `chain.pem` neu bilden, alle
  Plugins neu `prepare`/signieren/`assemble`n (die Manifest-Gültigkeit folgt
  dem neuen Zertifikat). `ofd` bleibt unverändert.
- **Generationswechsel** (Kompromittierung des Signierschlüssels oder
  geplant): offline `g2.key`/`g2.pem` mit
  `CN = Onesimus Plugin Signing G2` wie in Schritt 2 ausstellen, neuen
  Signierschlüssel wie in Schritt 3–5 (Ausstellung mit `-CA g2.pem -CAkey
  g2.key`, neues TPM-Handle oder altes mit `tpm2_evictcontrol -C o -c
  0x81000010` vorher freigeben **[ungetestet]**). Dann gemeinsames Release:
  Workspace-Version (Onesimus-Teil) erhöhen, `generation.pem` im
  Build-Verzeichnis durch `g2.pem` ersetzen, `ofd` bauen, alle Plugins mit
  der neuen Suite signieren, Pakete mit exakter Abhängigkeit
  `ofd (= <Version>)` ausliefern. Alte Bundles laden mit dem neuen `ofd`
  nicht (andere Generation, andere Suite) und umgekehrt.

### 10. Pakete (Schritt G, noch nicht umgesetzt)

OpenPGP-Signier-Unterschlüssel in den TPM verlagern, Reserve offline;
Keyring-Paket bauen; `ofd-plugin-<name>` aus dem signierten Bundle bauen.

## Offene Fragen

1. ~~Freigabe des Modells~~ – freigegeben (Nutzer, 19.11.2026): CA offline,
   Signierschlüssel offline erzeugt und in den TPM importiert.
1a. ~~Algorithmus und Werkzeug~~ – entschieden: RSA 4096, OpenSSL-Provider `tpm2`.
2. ~~Laufzeit der Signierzertifikate~~ – entschieden (Nutzer, 19.11.2026): **3 Jahre**, kein Zeitstempeldienst; Neusignieren rechtzeitig vor Ablauf.
3. Wo die zweite Offline-Kopie liegt.
4. Die „Vorschläge, offen“ oben (Policy-Pflicht, Policy-Format, nicht
   kritische Erweiterung u. a.).
