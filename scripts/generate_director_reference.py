#!/usr/bin/env python3
"""Extract the pinned Bareos command tables and compare local dispatch metadata.

No runtime commands are executed. Usage strings are upstream help text, not a
formal grammar. Run with --check to detect stale generated documentation.
"""
import argparse
import hashlib
import json
import re
import subprocess
from collections import Counter
from functools import lru_cache
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / 'docs/wiki/commands'
COMMIT = 'fc2c79b6c43eb5752e98ebd86116593dbbbe0c68'
TAG = 'WIP/25.1.2-pre'
BASE = f'https://github.com/bareos/bareos/blob/{COMMIT}/'
STRING = r'"(?:\\.|[^"\\])*"'
TOKEN = re.compile(STRING + r'|//[^\n]*|/\*[\s\S]*?\*/|.', re.S)


def clean(text):
    # Preserve line numbers and quoted strings while removing comments.
    return TOKEN.sub(lambda m: re.sub(r'[^\n]', ' ', m[0]) if m[0].startswith(('//', '/*')) else m[0], text)


def split_top(text, delimiter=','):
    parts, start, depth = [], 0, 0
    for match in TOKEN.finditer(text):
        t = match[0]
        if t in ('(', '[', '{'):
            depth += 1
        elif t in (')', ']', '}'):
            depth -= 1
        elif t == delimiter and depth == 0:
            parts.append(text[start:match.start()].strip())
            start = match.end()
    if text[start:].strip():
        parts.append(text[start:].strip())
    return parts


def matching(text, start):
    depth = 0
    for match in TOKEN.finditer(text, start):
        if match[0] == '{':
            depth += 1
        elif match[0] == '}':
            depth -= 1
            if depth == 0:
                return match.start()
    raise ValueError('Unbalanced initializer')


def table(text, name):
    match = re.search(r'\b' + re.escape(name) + r'\s*\[\]\s*=\s*\{', text)
    if not match:
        raise ValueError(f'Table not found: {name}')
    start = match.end() - 1
    end = matching(text, start)
    result, pos = [], start + 1
    while pos < end:
        opening = text.find('{', pos, end)
        if opening == -1:
            break
        closing = matching(text, opening)
        result.append((split_top(text[opening + 1:closing]), text.count('\n', 0, opening) + 1))
        pos = closing + 1
    return result


def literal(expression):
    if expression.strip() in ('NULL', 'nullptr'):
        return None
    # Only allow translated/literal string expressions; never execute C++.
    if not re.fullmatch(r'\s*(?:(?:NT_|T_)\(\s*)?(?:' + STRING + r'\s*)+\)?\s*', expression):
        raise ValueError(f'Not a literal string: {expression}')
    return ''.join(json.loads(m[0]) for m in re.finditer(STRING, expression))


def location(path, line, upstream=False):
    value = {'file': path, 'line': line}
    if upstream:
        value['url'] = BASE + path + f'#L{line}'
    return value


@lru_cache(maxsize=None)
def clean_file(path):
    return clean(path.read_text())


def definitions(source, handler):
    pattern = re.compile(r'\b(?:bool|int)\s+' + re.escape(handler) + r'\s*\([^;{}]*\)\s*\{', re.S)
    for path in sorted((source / 'core/src/dird').glob('*.cc')):
        text = clean_file(path)
        match = pattern.search(text)
        if match:
            return location(str(path.relative_to(source)), text.count('\n', 0, match.start()) + 1, True)
    raise ValueError(f'Upstream handler definition not found: {handler}')


def registry():
    path = 'crates/onesimus-dird/src/command_registry.rs'
    text = clean((ROOT / path).read_text())
    result = {}
    for match in re.finditer(r'CommandDef\s*\{\s*name:\s*(' + STRING + ')', text):
        end = matching(text, text.index('{', match.start()))
        block = text[match.start():end]
        name = json.loads(match[1])
        aliases = re.search(r'aliases:\s*&\[([^]]*)\]', block)[1]
        handler = re.search(r'handler:\s*commands::(\w+)::(\w+)', block)
        module, function = handler.groups()
        handler_path = f'crates/onesimus-dird/src/commands/{module}.rs'
        code = (ROOT / handler_path).read_text()
        definition = re.search(r'pub fn ' + function + r'\b', code)
        item = {
            'canonical_name': name,
            'handler': f'commands::{module}::{function}',
            'registry': location(path, text.count('\n', 0, match.start()) + 1),
            'implementation': location(handler_path, code.count('\n', 0, definition.start()) + 1),
            'declared_arguments': re.findall(r'arg(?:_required|_enum|_resource|_int)?\("([^"\n]+)"', block),
        }
        for key in [name] + re.findall(r'"([^"\n]+)"', aliases):
            if key in result:
                raise ValueError(f'Duplicate local command: {key}')
            result[key] = item
    return result


def filename(name):
    if name == '.':
        return 'dot-noop.md'
    return ('dot-' + name[1:] if name.startswith('.') else name).replace('_', '-') + '.md'


def extract(source, tree=False):
    path = 'core/src/dird/ua_tree.cc' if tree else 'core/src/dird/ua_cmds.cc'
    text = clean((source / path).read_text())
    usage_match = re.search(r'const char list_cmd_usage\[\]\s*=\s*([\s\S]*?);', text)
    list_usage = literal(usage_match[1]) if usage_match else None
    items = []
    for fields, line in table(text, 'commands'):
        assert len(fields) == (4 if tree else 6), fields
        name, handler, description = literal(fields[0]), fields[1], literal(fields[2])
        item = {
            'name': name,
            'context': 'restore_file_selection' if tree else 'director_console',
            'description': description,
            'upstream_handler': handler,
            'source': location(path, line, True),
            'handler_source': definitions(source, handler),
            'audit_event': fields[-1] == 'true',
        }
        if tree:
            item.update(usage=None, usage_kind='not_declared_in_table')
        else:
            usage = fields[3]
            if usage == 'list_cmd_usage':
                item.update(usage=list_usage, usage_kind='shared_literal')
            elif 'GetUsageStringForConsoleConfigureCommand' in usage:
                conf = (source / 'core/src/dird/dird_conf.cc').read_text()
                resources = [literal(f[0]).lower() for f, _ in table(clean(conf), 'dird_resource_tables')
                             if f[0] != 'nullptr' and f[2] != 'NULL' and f[3] != 'R_DIRECTOR']
                item.update(usage=None, usage_kind='generated_from_resource_directives',
                            usage_generator=location('core/src/dird/dird_conf.cc', conf[:conf.index('const char* GetUsageStringForConsoleConfigureCommand()')].count('\n') + 1, True),
                            configure_add_resource_types=resources,
                            usage_summary='add <resource-type> <resource-directives> | export client=<client>')
            else:
                item.update(usage=literal(usage), usage_kind='not_declared' if usage == 'NULL' else 'literal')
            item['allowed_in_runscript'] = fields[4] == 'true'
            item['argument_keywords_in_help'] = list(dict.fromkeys(re.findall(r'\b([a-zA-Z_][a-zA-Z_0-9]*)\s*=', item['usage'] or '')))
        items.append(item)
    if len({i['name'] for i in items}) != len(items):
        raise ValueError('Duplicate upstream command')
    return items


def md_link(loc, prefix='../../../'):
    return f"[{loc['file']}:{loc['line']}]({loc.get('url', prefix + loc['file'])})"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('bareos_source', type=Path)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    source = args.bareos_source.resolve()
    head = subprocess.check_output(['git', '-C', str(source), 'rev-parse', 'HEAD'], text=True).strip()
    if head != COMMIT:
        raise ValueError(f'Expected upstream commit {COMMIT}, got {head}')
    if subprocess.check_output(['git', '-C', str(source), 'diff', 'HEAD', '--', 'core/src/dird'], text=True):
        raise ValueError('Upstream sources have uncommitted changes')
    local = registry()
    overrides = json.loads((OUT / 'audit-notes.json').read_text())
    commands, tree = extract(source), extract(source, True)
    for command in commands:
        name = command['name']
        implementation = dict(local[name]) if name in local else {'canonical_name': None}
        implementation.update(status='registered_unverified' if name in local else 'missing', findings=[])
        if name in overrides['commands']:
            implementation.update(overrides['commands'][name])
        implementation['functional_parity_verified'] = False
        command['onesimus'] = implementation
        command['documentation'] = filename(name)
    for command in tree:
        command['onesimus'] = {'status': 'missing_context', 'functional_parity_verified': False,
                              'findings': ['Onesimus restore queues a job directly; no interactive restore file-selection shell is implemented.'],
                              'implementation': local['restore']['implementation']}
    extras = [{'name': n, **v} for n, v in sorted(local.items()) if n not in {c['name'] for c in commands}]
    counts = dict(sorted(Counter(c['onesimus']['status'] for c in commands).items()))
    data = {
        '$schema': './bareos-director-commands.schema.json',
        'schema_version': 1,
        'title': 'Bareos Director command reference and Onesimus static coverage audit',
        'reference_date': '2026-10-15',
        'upstream': {'repository': 'https://github.com/bareos/bareos', 'tag': TAG, 'commit': COMMIT,
                     'license': 'AGPL-3.0-only',
                     'notice': 'Command help and usage extracted from Bareos, copyright Bareos GmbH & Co. KG and contributors.',
                     'files': [{'path': str(p.relative_to(source)), 'sha256': hashlib.sha256(p.read_bytes()).hexdigest()}
                               for p in sorted((source / 'core/src/dird').glob('*')) if p.suffix in ('.cc', '.h')]},
        'local_source_files': [
            {'path': str(p.relative_to(ROOT)), 'sha256': hashlib.sha256(p.read_bytes()).hexdigest()}
            for p in sorted(set((ROOT / 'crates/onesimus-dird/src/commands').glob('*.rs')) | {
                ROOT / 'crates/onesimus-dird/src/command_registry.rs',
                ROOT / 'crates/onesimus-dird/src/command_executor.rs',
                ROOT / 'crates/bareos-bnet/src/debug.rs',
                ROOT / 'crates/onesimus-dird/src/main.rs',
                ROOT / 'crates/onesimus-dird/src/fd_protocol.rs',
                ROOT / 'crates/onesimus-dird/src/logging.rs',
                ROOT / 'crates/bareos-stored/src/command.rs',
                ROOT / 'crates/onesimus-dird/src/catalog_worker.rs',
                ROOT / 'crates/onesimus-dird/src/job_worker.rs',
                ROOT / 'crates/onesimus-dird/src/job_manager.rs',
                ROOT / 'crates/onesimus-dird/src/session.rs',
                ROOT / 'crates/onesimus-dird/src/scheduler.rs',
                ROOT / 'crates/oconsole/src/mcp.rs',
                ROOT / 'crates/oconsole/src/mcp_mirror.rs',
                ROOT / 'crates/oconsole/src/main.rs',
                ROOT / 'crates/oconsole/Cargo.toml',
            })
        ],
        'mcp_build_policies': {
            'default': 'mcp-production',
            'mutually_exclusive': True,
            'production': {'feature': 'mcp-production', 'tools': ['query'], 'write_handler_compiled': False},
            'development': {'feature': 'mcp-development', 'tools': ['query', 'execute'],
                            'write_handler_compiled': True, 'runtime_opt_in': '--mcp-allow-write'},
        },
        'scope': 'All top-level commands[] entries in ua_cmds.cc and restore file-selection commands[] entries in ua_tree.cc. Not FD/SD wire commands, bconsole @commands or every internal keyword/prompt.',
        'llm_instructions': [
            'Prefer full command names; Onesimus now resolves Bareos prefixes in the original table order, while local extensions remain exact matches.',
            'Descriptions and usage describe upstream Bareos, not guaranteed Onesimus behavior.',
            'registered_unverified means a handler exists, not that its semantics or options are complete.',
            'usage is upstream help text, not a formal grammar or exhaustive argument validation schema. NULL remains null; it does not mean no arguments are accepted.',
            'argument_keywords_in_help is a lexical index only, not a list of required or implemented arguments.',
            'Never infer read-only access from a leading dot, allowed_in_runscript or audit_event.',
            'Bareos console query is different from the MCP tool named query.',
            'Restore file-selection commands belong to a separate shell; do not send them as top-level Director commands.',
            'No live Director or destructive command was executed for this audit; all functional_parity_verified values are false.',
        ],
        'summary': {'top_level_commands': len(commands), 'registered_top_level_commands': sum(c['name'] in local for c in commands),
                    'status_counts': counts, 'restore_file_selection_commands': len(tree), 'onesimus_extra_names': len(extras)},
        'cross_cutting_findings': overrides['cross_cutting_findings'],
        'commands': commands, 'restore_file_selection_commands': tree, 'onesimus_extensions': extras,
    }
    outputs = {OUT / 'bareos-director-commands.json': json.dumps(data, ensure_ascii=False, indent=2) + '\n'}
    intro = f'''# Bareos Director: Originalreferenz und Vollständigkeitsabgleich

[LLM-Referenz (JSON)](bareos-director-commands.json) · [JSON-Schema](bareos-director-commands.schema.json) · [Wiki-Übersicht](README.md) · [Audit-Notizen](audit-notes.json)

Stand: 15.10.2026. Originalquelle: [Bareos `{TAG}`]({BASE}core/src/dird/ua_cmds.cc), Commit `{COMMIT}`.
Der Projektstand nennt 25.1.2; upstream existiert hierfür nur der Tag `WIP/25.1.2-pre`, kein Release-Tag 25.1.2. Diese Referenz behauptet daher keine Übereinstimmung mit einem finalen Release.

## Ergebnis

**{len(commands)} Originalbefehle, davon {sum(c['name'] in local for c in commands)} lokal registriert.** Hinzu kommen {len(tree)} Befehle der separaten Restore-Dateiauswahl und {len(extras)} lokale Zusatznamen.
Registrierung ist kein Nachweis funktionaler Vollständigkeit. Der Director ist nach den belegten Lücken **nicht vollständig kompatibel**.
Dies ist ein statischer Quellcodeabgleich; es wurden keine Live-Jobs, Löschungen oder anderen Director-Befehle ausgeführt.

| Status | Anzahl | Bedeutung |
|---|---:|---|
'''
    meanings = {'registered_unverified': 'Handler vorhanden; Funktionsparität nicht nachgewiesen',
                'partial': 'Konkrete fehlende Optionen oder Teilfunktionen belegt',
                'stub': 'Platzhalter statt der beschriebenen Wirkung',
                'semantic_mismatch': 'Abweichende Bedeutung gegenüber Bareos', 'missing': 'Kein lokaler Dispatch-Eintrag'}
    for status, count in counts.items():
        intro += f'| `{status}` | {count} | {meanings[status]} |\n'
    intro += '''
## Herkunft und Grenzen

Die Namen, englischen Beschreibungen, Syntaxtexte, RunScript-Freigaben und Audit-Flags stammen direkt aus `ua_cmds.cc::commands[]`; die Flag-Bedeutung aus `ua_cmdstruct.h`.
Die vollständige Tabelle ist maßgeblich, nicht der historisch unvollständige Syntaxtext von `help`.
`list` und `llist` teilen `list_cmd_usage`. `configure` erzeugt seine Syntax aus Ressourcendirektiven in `dird_conf.cc`; die JSON nennt den Generator und seine Ressourcentypen, statt einen Funktionsnamen als Syntax auszugeben.
`NULL`-Hilfetexte bleiben in JSON `null` (insbesondere `.locations`). Ein leerer/fehlender Syntaxtext beweist nicht, dass ein Befehl keine Parameter akzeptiert.

Bareos und Onesimus akzeptieren in der Original-Tabellenreihenfolge Groß-/Kleinschreibung ignorierende Präfixe; lokale Erweiterungen werden zusätzlich über ihre vollständigen Namen und Aliase aufgelöst.
Die JSON bewahrt jeden Originalnamen, auch `exit`, `quit`, `.quit`, `llist` und `umount`.
Konfigurationsdirektiven, FD/SD-Protokollbefehle und lokale bconsole-`@`-Befehle gehören nicht zu dieser Konsolentabelle.
Die Syntaxtexte sind Originalhilfe, keine vollständige formale Grammatik aller internen Optionen und Dialoge.

## MCP-Buildgrenzen

Der Standard- und Debian-Build verwendet `mcp-production`: ausschließlich das lesende MCP-Tool `query`, kein aktivierbarer Schreibhandler und kein `--mcp-allow-write`-Schalter.
Nur `--no-default-features --features mcp-development` kompiliert `execute` ein; zum Start ist zusätzlich `--mcp-allow-write` erforderlich. Beide Features zusammen oder keines werden vom Compiler abgewiesen. Diese Grenze gilt für stdio und HTTP. BNET/gRPC zum Director sowie normale interaktive Konsolenzugriffe bleiben davon unabhängig.

## Übergreifende Befunde

'''
    for finding in overrides['cross_cutting_findings']:
        intro += f"- **{finding['title']}** {finding['description']} " + ' · '.join(md_link(loc) for loc in finding['evidence']) + '\n'
    intro += '\n## Director-Konsole: vollständige Originaltabelle\n\n| Befehl | Originalbeschreibung | Lokaler Status |\n|---|---|---|\n'
    for command in commands:
        desc = command['description'] or 'Keine Beschreibung in der Originaltabelle (NULL)'
        intro += f"| [{command['name']}]({command['documentation']}) | {desc.replace('|', '&#124;')} | `{command['onesimus']['status']}` |\n"
        page = OUT / command['documentation']
        old = page.read_text() if page.exists() else f"# {command['name']}\n"
        start, end = '<!-- bareos-reference:start -->', '<!-- bareos-reference:end -->'
        if start in old:
            before, remaining = old.split(start, 1)
            _, after = remaining.split(end, 1)
        else:
            first, _, rest = old.partition('\n')
            before, after = first + '\n\n', '\n' + rest.lstrip('\n')
        section = f'''{start}
## Aus den Bareos-Originalquellen

Referenz: [{TAG}, `{COMMIT[:12]}`]({command['source']['url']}) · [Vollständigkeitsabgleich](bareos-reference.md) · [LLM-JSON](bareos-director-commands.json)

**Originalbeschreibung:** {desc}

'''
        if command['usage']:
            section += f"**Originalsyntax:**\n\n```text\n{command['name']} {command['usage']}\n```\n\n"
        elif command['usage_kind'] == 'generated_from_resource_directives':
            section += f"**Dynamische Syntax:** `{command['usage_summary']}`. Der [Originalgenerator]({command['usage_generator']['url']}) leitet Pflichtfelder, optionale Felder und Typen aus Ressourcendirektiven ab.\n\nRessourcentypen für `add`: " + ', '.join(f'`{r}`' for r in command['configure_add_resource_types']) + '.\n\n'
        else:
            section += '**Originalsyntax:** Kein Parametertext in der Tabelle; daraus folgt keine Aussage über interaktive Eingaben oder intern akzeptierte Optionen.\n\n'
        section += f"**RunScript erlaubt:** {'ja' if command['allowed_in_runscript'] else 'nein'}. **Audit-Ereignis:** {'ja' if command['audit_event'] else 'nein'}. Beide Flags sind keine Read-only-Klassifizierung.\n\n"
        local_item = command['onesimus']
        section += f"**Onesimus-Status:** `{local_item['status']}`. Funktionsparität ist nicht durch einen Live-Test nachgewiesen.\n\n"
        for finding in local_item['findings']:
            section += f'- {finding}\n'
        if local_item['findings']:
            section += '\n'
        if 'implementation' in local_item:
            section += '**Lokaler Handler:** ' + md_link(local_item['implementation']) + '.\n\n'
        section += 'Die folgenden vorhandenen Beispiele und Verhaltensbeschreibungen sind lokale Dokumentation; sie sind kein Beleg für vollständige Bareos-Kompatibilität. Bei Widersprüchen gilt der obige Befund.\n' + end
        outputs[page] = before + section + after
    intro += '\n## Restore-Dateiauswahl (eigener Kontext)\n\nDiese Befehle stehen in `ua_tree.cc::commands[]`, nicht in der Hauptkonsole. Onesimus besitzt derzeit keine entsprechende interaktive Dateiauswahl. Gleichnamige Konsolenbefehle zählen nicht als Implementierung dieses Kontexts.\n\n| Befehl | Originalbeschreibung | Originalquelle |\n|---|---|---|\n'
    for command in tree:
        intro += f"| `{command['name']}` | {command['description']} | [Handler]({command['handler_source']['url']}) |\n"
    intro += '\n## Onesimus-Erweiterungen\n\nZusatznamen sind keine Belege für upstream vorhandene Funktionen. Die JSON enthält ihre lokalen Handler und Aliase.\n\n' + ', '.join(f"`{e['name']}`" for e in extras) + '.\n'
    intro += '''
## Reproduzieren und aktualisieren

```bash
git clone --depth 1 --branch WIP/25.1.2-pre --filter=blob:none --sparse https://github.com/bareos/bareos.git /tmp/bareos-reference
git -C /tmp/bareos-reference sparse-checkout set core/src/dird
python3 scripts/generate_director_reference.py /tmp/bareos-reference
python3 scripts/generate_director_reference.py /tmp/bareos-reference --check
```

Der Generator prüft Commit und unveränderte Originalquellen, extrahiert beide Tabellen samt Handler-Quellen, prüft eindeutige Namen und gleicht die lokale Registry einschließlich Aliase ab. Die SHA-256-Prüfsummen der gelesenen upstream Director-Dateien stehen in der JSON.
Belegte Verhaltenslücken werden in `audit-notes.json` gepflegt. Bei Quellcodeänderungen müssen diese Befunde fachlich neu bewertet werden; eine Registrierung darf nicht automatisch als vollständige Implementierung gelten.
Die JSON hält außerdem die SHA-256-Prüfsummen der betrachteten lokalen Implementierungen fest; `--check` erkennt dadurch auch Handler-Änderungen ohne neue Befehlsnamen.
Original-Hilfetexte und Syntax: Bareos GmbH & Co. KG und Mitwirkende, AGPL-3.0.
'''
    outputs[OUT / 'bareos-reference.md'] = intro
    for path, text in outputs.items():
        if args.check:
            if not path.exists() or path.read_text() != text:
                raise SystemExit(f'Stale generated file: {path.relative_to(ROOT)}')
        else:
            path.write_text(text)
    print(json.dumps(data['summary'], ensure_ascii=False))


if __name__ == '__main__':
    main()
