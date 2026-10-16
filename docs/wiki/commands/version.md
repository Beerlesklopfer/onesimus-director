# version — Show Director Version

<!-- bareos-reference:start -->
## Aus den Bareos-Originalquellen

Referenz: [WIP/25.1.2-pre-43-g18be77e68, `18be77e6832e`](https://github.com/bareos/bareos/blob/18be77e6832e33f5acd64094b9cba6066f2503f9/core/src/dird/ua_cmds.cc#L521) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** Print Director version

**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.

**RunScript erlaubt:** ja. **Audit-Ereignis:** nein. Beide Flags sind keine Read-only-Klassifizierung.

**Onesimus-Status:** `parity_verified`. Alle unten genannten Paritätsfälle antworten byte-identisch zum Original; nicht genannte Varianten sind damit nicht belegt.

- Paritätslauf 16.10.2026 (`development` `7993ce6`): 9 von 9 Fällen byte-identisch; Modi: Text, API 1, API 2.

**Paritätsfälle (9):** `c004-quit-after-command`, `c009-version`, `c009-version-json`, `c009-version-api1`, `c020-compact-version`, `c020-json-version`, `c020-compact-persists`, `c020-compact-reset`, `version`.

**Lokaler Handler:** [crates/onesimus-dird/src/commands/basic.rs:13](../../../crates/onesimus-dird/src/commands/basic.rs).

Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.
<!-- bareos-reference:end -->
Display the Director daemon version, build info, and operating system.

## Syntax

```
version
```

## Example

```
* version
bareos-dir Version: 25.1.2+onesimus.0.2.0 (Debian GNU/Linux 13 (trixie))
```

## Notes

The version string format is `bareos-dir Version: <version> (<os>)` for protocol compatibility with C++ bconsole and Bareos WebUI. The actual daemon binary is `onesimus-dird`.
