#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-FileCopyrightText: 2026 Jörg Bernau <contact@onesimus.io>
"""SPDX-Kopfzeilen der Quell- und Skriptdateien prüfen oder setzen.

    scripts/ci/spdx.py check   # Exit 1, wenn eine Datei keinen Kopf hat
    scripts/ci/spdx.py apply   # fehlende Köpfe setzen, alte Lizenz ersetzen

Jede getrackte Datei der Endungen .rs .proto .wit .sh .fish .py .ps1 .bat beginnt
mit zwei Zeilen (nach einer Shebang-Zeile bzw. „@echo off“):

    SPDX-License-Identifier: AGPL-3.0-or-later
    SPDX-FileCopyrightText: <Jahr der ersten Fassung>[-2026] Jörg Bernau <contact@onesimus.io>

Bareos-nahe Teile (scripts/ci/bareos-derived.txt) bleiben AGPL-3.0-only und tragen
zusätzlich die Zeilen der Bareos-Rechteinhaber. Ausgenommen sind erzeugte Dateien
und Testdaten: siehe SKIP.
"""
import re
import subprocess
import sys

LICENSE = "AGPL-3.0-or-later"
HOLDER = "Jörg Bernau <contact@onesimus.io>"
THIS_YEAR = 2026
COMMENT = {".rs": "//", ".proto": "//", ".wit": "//", ".sh": "#", ".fish": "#", ".py": "#", ".ps1": "#", ".bat": "REM"}
# Pfadpräfixe, die nicht angefasst werden: Bareos-Ableitungen, Fremdcode, Testdaten
SKIP = ("proto/", "docs/upstream/", "crates/bareos-orm/tests/data/", "crates/bareos-format/testdata/", "crates/odir/tests/fixtures/",
        "crates/odir/src/ua_commands_table.rs", "target/", "dist/")


def tracked():
    out = subprocess.run(["git", "ls-files"], capture_output=True, text=True, check=True).stdout.split("\n")
    return [p for p in out if p and not p.startswith(SKIP) and "." + p.rsplit(".", 1)[-1] in COMMENT and "." in p.rsplit("/", 1)[-1]]


def first_years():
    """Pfad -> Jahr der ersten Fassung (aus dem Verlauf, mit Umbenennungen)."""
    out = subprocess.run(["git", "log", "--reverse", "-M", "--name-status", "--format=@%ad", "--date=format:%Y"],
                         capture_output=True, text=True, check=True).stdout.split("\n")
    years, year = {}, THIS_YEAR
    for line in out:
        if line.startswith("@"):
            year = int(line[1:])
        elif line[:1] in "AMR" and "\t" in line:
            parts = line.split("\t")
            if parts[0].startswith("R"):
                years[parts[2]] = years.get(parts[1], year)
            elif parts[0] == "A":
                years.setdefault(parts[1], year)
    return years


DERIVED_LIST = "scripts/ci/bareos-derived.txt"
DERIVED_LICENSE = "AGPL-3.0-only"
BAREOS_HOLDERS = [
    "2000-2012 Free Software Foundation Europe e.V.",
    "2010-2017 Planets Communications B.V.",
    "2012-2026 Bareos GmbH & Co. KG",
]


def derived_list():
    """Pfade und Präfixe aus bareos-derived.txt (Bareos-nahe Teile: AGPL-3.0-only)."""
    try:
        lines = open(DERIVED_LIST, encoding="utf-8").read().split("\n")
    except OSError:
        return []
    return [l.strip() for l in lines if l.strip() and not l.startswith("#")]


def is_derived(path, entries):
    return any(path.startswith(e) if e.endswith("/") else path == e for e in entries)


def header(path, year, entries):
    """Die Kopfzeilen: eigener Code or-later; Bareos-nahe Teile only mit den Zeilen der Rechteinhaber."""
    c = COMMENT["." + path.rsplit(".", 1)[-1]]
    span = f"{year}-{THIS_YEAR}" if year < THIS_YEAR else f"{THIS_YEAR}"
    mine = f"{c} SPDX-FileCopyrightText: {span} {HOLDER}"
    if is_derived(path, entries):
        return [f"{c} SPDX-License-Identifier: {DERIVED_LICENSE}"] + [f"{c} SPDX-FileCopyrightText: {h}" for h in BAREOS_HOLDERS] + [mine]
    return [f"{c} SPDX-License-Identifier: {LICENSE}", mine]


def split_prologue(lines, path):
    """Zeilen, die vor dem Kopf stehen müssen: Shebang, „@echo off“, Encoding-Zeile."""
    n = 0
    if lines and lines[0].startswith("#!"):
        n = 1
        if len(lines) > 1 and re.match(r"#.*coding[:=]", lines[1]):
            n = 2
    elif lines and lines[0].lower().strip() == "@echo off":
        n = 1
    return n


def process(path, years, apply, entries):
    raw = open(path, "rb").read()
    crlf = b"\r\n" in raw
    bom = raw.startswith(b"\xef\xbb\xbf")
    text = raw[3:].decode("utf-8") if bom else raw.decode("utf-8", "surrogateescape")
    eol = "\r\n" if crlf else "\n"
    lines = text.split(eol)
    n = split_prologue(lines, path)
    wanted = header(path, years.get(path, THIS_YEAR), entries)
    # the existing header block: consecutive SPDX lines after the prologue
    end = n
    while end < len(lines) and "SPDX-" in lines[end]:
        end += 1
    current = lines[n:end]
    ok = bool(current) and current[0] == wanted[0] and all(
        any(w.split(" ", 1)[1] in c for c in current) for w in wanted[1:] if "Bareos" in w or "Free Software" in w or "Planets" in w)
    if ok or not apply:
        return ok
    lines[n:end] = wanted
    out = eol.join(lines)
    data = (b"\xef\xbb\xbf" if bom else b"") + out.encode("utf-8", "surrogateescape")
    open(path, "wb").write(data)
    return True


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "check"
    years = first_years() if mode == "apply" else {}
    entries = derived_list()
    bad = [p for p in tracked() if not process(p, years, mode == "apply", entries)]
    if mode == "check" and bad:
        print("SPDX header missing or not " + LICENSE + ":\n  " + "\n  ".join(bad))
        sys.exit(1)
    print(f"{mode}: {len(tracked())} files")


if __name__ == "__main__":
    main()
