# PAM-Anmeldung von Konsolen (`Use Pam Authentication`)

Stand 21.11.2026, S-43 in [STUBS.md](../STUBS.md). Gegenstück im Original: `dird/auth_pam.cc`, `console/auth_pam.cc`.

## Konfiguration

```text
Console { Name = pamc; Password = "…"; Use Pam Authentication = yes }
User    { Name = anna; Command ACL = status, list; Job ACL = *all* }
```

- Die Console trägt nur das Passwort (CRAM-MD5 wie bei jeder Console). Rechte hat sie nicht: `odir -t` lehnt `Use Pam Authentication` zusammen mit ACLs oder `Profile` in der Console ab.
- Nach dem Passwort verlangt der Director eine PAM-Anmeldung. Die Rechte der Sitzung sind die der `User`-Ressource mit dem PAM-Benutzernamen. Gibt es keine, ist die Anmeldung abgelehnt (Original: „No user config found for user …“).
- PAM-Dienst: `bareos` (`/etc/pam.d/bareos`), wie im Original. Ohne diese Datei gilt `other`.
- Die Katalog-ACL (`.acl`) kennt keine PAM-Benutzer: Für sie gilt „keine Einträge“, es wirkt allein die `User`-Ressource.

## Ablauf auf der Leitung (beobachtet mit `bconsole` 25.1.3)

Nach CRAM-MD5 und vor dem Banner. RS ist das Byte 0x1e.

| Richtung | Rahmen |
|---|---|
| Director → Konsole | `1001 PAM required` |
| Konsole → Director | `4002` RS Benutzer RS Passwort (Datei `bconsole -p`) **oder** `4001` RS (fragt den Benutzer) |
| Director → Konsole, nur interaktiv, je PAM-Frage | Rahmen mit einem Byte (1 = Antwort nicht anzeigen, 2 = anzeigen), dann Rahmen mit dem Fragetext |
| Konsole → Director | Rahmen mit der Antwort |
| Director → Konsole, nur interaktiv, am Ende | Rahmen mit dem Byte 0, Rahmen mit einem Byte Ergebnis (1 gut, 0 schlecht; die Konsole wertet es nicht aus) |
| Director → Konsole | Erfolg: der übliche Banner `1000 OK: …`. Misserfolg: `1999 PAM authentication failed.` und Ende (bconsole beendet sich ohne Meldung) |

Konsolen vor 18.2 werden abgelehnt („PAM authentication using outdated Bareos console denied“). PAM-Hinweise und Fehlermeldungen der Module (`PAM_TEXT_INFO`, `PAM_ERROR_MSG`) gibt es auf der Leitung nicht, sie werden verworfen.

## Grenzen

- Über gRPC melden sich PAM-Consolen nicht an (`grpc/auth.rs`): Dort gäbe es nur das Passwort.
- Die Anmeldezeile des Banners nennt die Console (`You are logged in as: pamc`), nicht den PAM-Benutzer.
- Der Dienst `bareos` unter `/etc/pam.d` und eine echte Anmeldung sind nicht getestet (ohne root nicht einzurichten). Geprüft ist der Aufruf von PAM selbst (`pam_auth::tests::the_system_pam_refuses_an_unknown_login`), der Dialog mit dem Original-bconsole (`accept::tests::pam_console_logs_in_through_the_original_bconsole`, mit einem Testbackend) und die Rahmen im Einzelnen (`pam_auth::tests`, Mitschnitt `crates/onesimus-rig/tests/pam_capture.rs`).
- Bau: `libpam0g-dev`; das Paket `odir` hängt über `$auto` von `libpam0g` ab. Crate `pam-client2` 0.5.6 (MPL-2.0).
