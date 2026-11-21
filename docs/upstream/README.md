# Fehler in Bareos (Upstream-Log)

Fehler im Bareos-Original, die beim Nachbau gefunden wurden. Onesimus übernimmt sie nicht, sondern berichtigt sie (Vorgabe Maintainer, 21.11.2026: „Fehler von Bareos berichtigen wir. Diese Fehler kommen in ein Log und wir stellen einen PR.“). Jede Berichtigung ist zugleich eine bewusste Abweichung und steht deshalb auch im [Kompatibilitätslog](../COMPATIBILITY-LOG.md).

Referenz: Bareos `18be77e6` (wie das Kompatibilitätslog). Je Eintrag ein Patch gegen diesen Stand in diesem Verzeichnis.

Status: **gefunden** · **PR vorbereitet** (Patch hier, noch nicht eingereicht) · **PR eingereicht** (Link) · **übernommen** (Upstream-Commit) · **abgelehnt** (mit Grund).

| ID | Datum | Bereich | Fehler | Folge | Onesimus | Patch | Status |
| --- | --- | --- | --- | --- | --- | --- | --- |
| U-001 | 21.11.2026 | Director, Jobstart (`dird/job.cc:856` GetOrCreatePoolRecord) | Die Funktion bekommt den Pool-Namen, legt einen fehlenden Satz aber aus `jcr->dir_impl->res.pool` an, dem Pool des Jobs. VirtualFull (`vbackup.cc:195`) und Migration/Copy (`migrate.cc:290`) rufen sie auch für den **Next Pool** auf. | Fehlt der Satz des Next Pool im Katalog (z. B. nach `configure add pool` ohne `reload`), gibt CreatePool für den vorhandenen Job-Pool 0 („existiert“) zurück; die Schleife meldet „Created database record for Pool "<Next Pool>"“ und fragt erneut – **endlos**, der Job hängt und füllt das Log. | odir legt den Satz unter dem angefragten Namen an (`pool_db::get_or_create_pool_record`, C-088). | [U-001-getorcreatepoolrecord-nextpool.patch](U-001-getorcreatepoolrecord-nextpool.patch) – Pool-Ressource per `GetResWithName(R_POOL, pool_name)`; **nicht übersetzt und nicht gegen Bareos getestet** (Analyse am Quelltext) | PR vorbereitet |

## Ablauf für einen PR

1. Fehler am Quelltext belegen, möglichst mit einem Testfall gegen das Original (Prüfstand wie im Kompatibilitätslog).
2. Patch gegen den aktuellen Bareos-Master, übersetzen, Bareos-Systemtest bzw. Unittest ergänzen.
3. PR auf github.com/bareos/bareos erst nach Freigabe durch den Maintainer einreichen (öffentlich, unter seinem Konto); Link und Status hier nachtragen.
