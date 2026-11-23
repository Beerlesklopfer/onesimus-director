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

Ausgenommen sind Dateien, die von Bareos stammen (Lizenz bleibt AGPL-3.0-only
und die Zeilen von Bareos), erzeugte und Testdaten: siehe SKIP.
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
        "crates/odir/src/ua_commands_table.rs", "target/", "dist/", "plugins/")


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


def header(path, year):
    c = COMMENT["." + path.rsplit(".", 1)[-1]]
    span = f"{year}-{THIS_YEAR}" if year < THIS_YEAR else f"{THIS_YEAR}"
    sep = " " if c != "REM" else " "
    return [f"{c}{sep}SPDX-License-Identifier: {LICENSE}", f"{c}{sep}SPDX-FileCopyrightText: {span} {HOLDER}"]


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


def process(path, years, apply):
    raw = open(path, "rb").read()
    crlf = b"\r\n" in raw
    bom = raw.startswith(b"\xef\xbb\xbf")
    text = raw[3:].decode("utf-8") if bom else raw.decode("utf-8", "surrogateescape")
    eol = "\r\n" if crlf else "\n"
    lines = text.split(eol)
    n = split_prologue(lines, path)
    wanted = header(path, years.get(path, THIS_YEAR))
    existing = [i for i, l in enumerate(lines[:n + 6]) if "SPDX-License-Identifier" in l]
    if existing:
        i = existing[0]
        ok = LICENSE in lines[i]
        if ok or not apply:
            return ok
        lines[i] = wanted[0]
        if i + 1 < len(lines) and "SPDX-FileCopyrightText" in lines[i + 1]:
            return_ok = True
        else:
            lines.insert(i + 1, wanted[1])
    else:
        if not apply:
            return False
        lines[n:n] = wanted
    out = eol.join(lines)
    data = (b"\xef\xbb\xbf" if bom else b"") + out.encode("utf-8", "surrogateescape")
    open(path, "wb").write(data)
    return True


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "check"
    years = first_years() if mode == "apply" else {}
    bad = [p for p in tracked() if not process(p, years, mode == "apply")]
    if mode == "check" and bad:
        print("SPDX header missing or not " + LICENSE + ":\n  " + "\n  ".join(bad))
        sys.exit(1)
    print(f"{mode}: {len(tracked())} files")


if __name__ == "__main__":
    main()
