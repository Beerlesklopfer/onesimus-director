# odir: Free-Tier (1 TB/Monat) und Lizenzdatei

Stand 21.11.2026, Entwurf. Anforderung des Nutzers, übermittelt von der GUI-Sitzung:

> der dir speichert 1TB/monat limit als free version
>
> wer das umgehen will, der muss programmieren

Das Limit liegt im Code von odir. Keine Lizenzserver, keine Telemetrie.

Korrektur des Nutzers (21.11.2026): „Das ist der Zusatz der AGPL-Lizenz. Wer das umgeht, bricht die Lizenz.“ Das Limit soll also als Zusatzbedingung zur AGPL gelten.

**Rechtlicher Vorbehalt (offen, vor der Umsetzung klären; keine Rechtsberatung):**
- AGPLv3 §7 erlaubt nur bestimmte zusätzliche Bedingungen (Haftung, Namensnennung, Markenschutz …). Andere Einschränkungen gelten als „further restrictions“, die jeder Empfänger entfernen darf; §10 verbietet, sie aufzuerlegen. Ein Nutzungslimit als Lizenzbedingung wäre so eine Einschränkung.
- Ist odir als enger Nachbau von Bareos (AGPLv3) ein abgeleitetes Werk, muss es unter der AGPLv3 der Bareos-Urheber bleiben; zusätzliche Einschränkungen gehen dann nicht.
- Mögliche Wege: (1) AGPL ohne Zusatz, Limit nur als Voreinstellung, bezahlt wird für Lizenzdatei, Support und Pakete; (2) duale Lizenz für Teile, die nicht von Bareos abgeleitet sind; (3) Source-available-Lizenz nur für eigenen Code ohne Bareos-Anteil; (4) Markenschutz für „Onesimus“ (§7e verträglich).
- Empfehlung: Klärung durch einen Anwalt für Open-Source-Lizenzen, insbesondere ob odir ein abgeleitetes Werk von Bareos ist. Die Technik unten ist von dieser Entscheidung unabhängig.

## Anforderung (übernommen)

- Gezählt werden die von **Backup-Jobs** geschriebenen Bytes (`JobBytes`) je Kalendermonat. Copy und Migration zählen nicht doppelt.
- Am Limit: laufende Jobs laufen zu Ende; **neue Backup-Jobs starten nicht** bis Monatsende, mit klarer Meldung im Job-Log und einem Push-Ereignis. **Restores gehen immer.**
- Bezahlstufe: **signierte Lizenzdatei** im Konfigurationsverzeichnis (Kunde, Limit, Gültigkeit), offline geprüft.
- GUI: Verbrauch und Limit in `status dir` und typisiert im gRPC-Status, für Statusanzeige und Tray-Ampel (gelb nahe am Limit, rot am Limit). Felder werden vorher abgestimmt (wie D-19/D-21).

## Vorschläge zu den offenen Punkten (Entscheidung Nutzer)

| # | Frage | Vorschlag |
| --- | --- | --- |
| L-1 | Was zählt genau? | Summe `JobBytes` aller Jobs mit `Type='B'` und Level ≠ `V` (VirtualFull schreibt vorhandene Daten neu zusammen), egal mit welchem Endstatus (auch ein abgebrochener Job hat geschrieben). Nicht gezählt: Copy (`C`), Migration (`g`/`M`), Restore, Verify, Admin, Archive. |
| L-2 | 1 TB | dezimal: 10^12 Bytes (wie Plattenhersteller; `status dir` zeigt „1.0 TB“). |
| L-3 | Monatsgrenze, Zeitzone | Kalendermonat in der Zeitzone des Directors (wie Schedules und `list jobs`); Zuordnung nach `StartTime` des Jobs (ein Job über Mitternacht zählt im Startmonat). |
| L-4 | Mehrere Kataloge | Summe über alle Catalog-Ressourcen des Directors (Lizenz gilt je Director). |
| L-5 | Gehaltene geplante Jobs | **Überspringen**, nicht in den nächsten Monat schieben (sonst startet am 1. eine Lawine). Jeder übersprungene Lauf erscheint im Job-Verlauf (Job-Satz mit Status `f`, Meldung mit Grund und nächstem möglichem Termin), damit nichts still fehlt. |
| L-6 | Manueller `run` am Limit | abgelehnt mit derselben Meldung; kein Job-Satz (wie ein abgelehnter Start in Bareos). |
| L-7 | Laufender Job überschreitet das Limit | läuft zu Ende; der Monat steht danach über 100 %. |
| L-8 | Vorwarnung | Director-Meldung und Ereignis bei 80 % und 95 % (einmal je Monat und Schwelle); Ampel gelb ab 80 %. |
| L-9 | Lizenzdatei, Ort | außerhalb von `bareos-dir.d` (der Bareos-Parser kennt sie nicht, und `-t` bleibt bareos-kompatibel): `/etc/bareos/onesimus-licence.json`, Pfad änderbar mit `odir --licence-file`. Kein neuer Direktivname nötig. |
| L-10 | Lizenzdatei, Format | JSON: `format` (1), `licence_id`, `customer`, `director` (Name des Directors, an den sie gebunden ist; `*` = beliebig), `limit_bytes_per_month` (0 = unbegrenzt), `valid_from`, `valid_until`, `issued`; dazu `signature`: **Ed25519** über die kanonische JSON-Form ohne `signature` (base64). Der öffentliche Schlüssel ist in odir eingebaut. |
| L-11 | Signierwerkzeug | eigenes kleines Programm `onesimus-licence` (sign/verify/show) in einem **nicht** ausgelieferten Crate; der private Schlüssel liegt offline (Schlüsselzeremonie, offener Punkt aus dem Backlog). |
| L-12 | Ungültige/abgelaufene Lizenz | odir startet trotzdem, fällt auf das Free-Tier zurück und meldet es beim Start, in `status dir` und als Ereignis; 30 Tage vor Ablauf Vorwarnung. Eine kaputte Signatur ist kein Konfigurationsfehler (kein Exit 42). |
| L-13 | Wann geprüft | beim Start und `reload` (Lizenz), vor jedem Start eines Backup-Jobs (Verbrauch aus dem Katalog, eine Summenabfrage), dazu der Wert in `status dir`. |

## Ausgabe und Felder (Vorschlag, mit der GUI-Sitzung abzustimmen)

`status dir`, nach der Kopfzeile:

```
Free tier: 412.7 GB of 1.0 TB written this month (41 %), resets 01-Dec-2026 00:00
```
bzw. `Licence: <customer>, limit 10.0 TB/month (valid until 31-Dec-2027): 2.3 TB written this month (23 %)` oder `Licence: unlimited (<customer>, valid until …)`.

gRPC (Vorschlag, Feldnummern nach Abstimmung):

```
message UsageInfo {
  string month = 1;              // "2026-11", Zeitzone des Directors
  uint64 bytes_written = 2;      // L-1
  uint64 limit_bytes = 3;        // 0 = unbegrenzt
  UsageState state = 4;          // OK, NEAR (>= 80 %), REACHED
  int64 resets_at = 5;           // Unix-Zeit des Monatswechsels
  LicenceInfo licence = 6;
  uint64 bytes_running = 7;      // was laufende gezählte Jobs bisher geschrieben haben (nicht in bytes_written)
  uint32 held_jobs_this_month = 8; // wegen des Limits übersprungene geplante Jobs (L-5)
}
message LicenceInfo {
  LicenceKind kind = 1;          // FREE, LICENSED, EXPIRED, INVALID
  string customer = 2;
  string licence_id = 3;
  int64 valid_until = 4;
  string message = 5;            // Grund bei EXPIRED/INVALID
}
```
in `GetDirectorStatus` (Feld `usage`) und als Push-Ereignis `UsageEvent { UsageInfo usage = 1; }` bei Zustandswechsel (OK → NEAR → REACHED, Monatswechsel, Lizenz geladen/abgelaufen) **und einmal direkt nach dem Login** auf dem Push-Strom (die Ampel braucht kein Abfragen).

Ergänzungen der GUI-Sitzung (21.11.2026), übernommen:
- `bytes_running` getrennt von `bytes_written` (abgeschlossene Jobs aus dem Katalog), damit der Balken nicht erst am Jobende springt; der Zustand NEAR/REACHED rechnet mit der Summe beider.
- `held_jobs_this_month` für „3 Sicherungen übersprungen“.
- Erkennbarer Grund ohne Textauswertung: `JobStatusEvent.reason` (Enum, `LIMIT_REACHED`) und im typisierten Job-Satz `extra_fields["skip_reason"] = "free_tier_limit"`; der Grund wird je Job im Schema `onesimus` gespeichert (Tabelle `job_skip`, JobId + Grund, Migrationsschritt in bareos-orm), weil die Bareos-Tabelle `Job` keine Spalte dafür hat.
- Hochrechnung bis Monatsende rechnet die GUI aus `month`, `bytes_written`, `bytes_running` und `resets_at`; odir legt keine Prognose-Politik fest.

Job-Log-Meldung beim Zurückhalten (M_FATAL im übersprungenen Job, bzw. Antwort auf `run`):

```
Free tier limit reached: 1.0 TB written by backup jobs in November 2026 (limit 1.0 TB per month).
New backup jobs start again on 01-Dec-2026. Restores are not affected.
A licence file removes or raises the limit (see https://onesimus.io).
```

## Umsetzung (nach den Entscheidungen)

1. `odir::licence`: Laden, Ed25519-Prüfung (`ed25519-dalek`), Bindung an den Director-Namen, Ablauf.
2. `odir::usage`: Summenabfrage je Katalog (L-1, L-3, L-4), Zustand, Schwellen-Meldungen (L-8).
3. Prüfung vor dem Start eines Backup-Jobs: im Scheduler-Pfad (L-5) und in `run` (L-6), Ausnahme Restore/Verify/Copy/Migrate/Admin.
4. `status dir` (Text, JSON), gRPC-Felder und Ereignis nach Abstimmung; MCP `status_director`.
5. `onesimus-licence` (nicht ausgeliefert), Testschlüssel nur in Tests.
6. Tests: Zählung (Typen, Level V, Monatsgrenze, Zeitzone, mehrere Kataloge), Sperre und Ausnahmen, Lizenz gültig/abgelaufen/falsche Signatur/falscher Director, Meldungstexte.
7. Doku: betrieb-odir.md (Abweichung von Bareos: Bareos kennt kein Limit), Wiki-Seite, COMPATIBILITY-LOG als Entscheidung.
