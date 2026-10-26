#!/usr/bin/env python3
"""Publish docs/wiki to the GitHub wiki of onesimus-director.

docs/wiki (branch development) is the source of truth; the GitHub wiki is
generated from it. GitHub wikis are flat, so pages are renamed:

  docs/wiki/README.md            -> Home
  docs/wiki/<page>.md            -> <page>
  docs/wiki/commands/README.md   -> Commands
  docs/wiki/commands/<cmd>.md    -> <cmd>

Links between wiki pages become wiki links (`[x](Page#anchor)`); links to
anything else in the repository (code, JSON files) become absolute links to
github.com/<repo>/blob/<branch>/... .

Usage: scripts/publish_wiki.py <wiki-checkout> [--repo OWNER/NAME] [--branch B]
Writes the pages into the wiki checkout; committing and pushing is left to the
caller (git -C <wiki-checkout> add -A && git commit && git push).
"""
import argparse
import posixpath
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
WIKI = ROOT / "docs" / "wiki"
LINK = re.compile(r"(\[[^\]]*\]\()([^)\s]+)(\))")


def page_name(rel: str) -> str:
    """docs/wiki-relative .md path -> wiki page name."""
    if rel == "README.md":
        return "Home"
    if rel == "commands/README.md":
        return "Commands"
    return Path(rel).stem


def pages() -> dict:
    result = {}
    for path in sorted(WIKI.rglob("*.md")):
        rel = path.relative_to(WIKI).as_posix()
        name = page_name(rel)
        if name in result.values():
            sys.exit(f"page name collision: {name} ({rel})")
        result[rel] = name
    return result


def rewrite(text: str, rel: str, mapping: dict, blob: str) -> str:
    src_dir = posixpath.dirname(f"docs/wiki/{rel}")

    def fix(m):
        target = m.group(2)
        if re.match(r"^[a-z]+:|^#|^mailto:", target):
            return m.group(0)
        path, _, anchor = target.partition("#")
        repo_path = posixpath.normpath(posixpath.join(src_dir, path))
        anchor = f"#{anchor}" if anchor else ""
        if repo_path.startswith("docs/wiki/") and repo_path.endswith(".md"):
            wiki_rel = repo_path[len("docs/wiki/"):]
            if wiki_rel in mapping:
                return f"{m.group(1)}{mapping[wiki_rel]}{anchor}{m.group(3)}"
        if repo_path.startswith(".."):
            return m.group(0)
        return f"{m.group(1)}{blob}/{repo_path}{anchor}{m.group(3)}"

    return LINK.sub(fix, text)


def sidebar(mapping: dict) -> str:
    top = [n for r, n in mapping.items() if "/" not in r and n != "Home"]
    commands = sorted(n for r, n in mapping.items() if r.startswith("commands/") and n != "Commands")
    # GitHub Markdown supports native details/summary, including nested groups.
    # Blank lines around Markdown content inside HTML blocks are required.
    lines = ["**[Home](Home)**", "", "<details>", "<summary>Themen</summary>", ""]
    lines += [f"- [{n}]({n})" for n in sorted(top)]
    lines += ["", "</details>", "", "<details>", "<summary>Befehle</summary>", "",
              "- [Befehlsübersicht](Commands)"]
    groups = {"Konsolenbefehle": [], "Punktbefehle": [], "BVFS": [], "OVFS": []}
    for name in commands:
        if name == "bareos-reference":
            lines.append(f"- [Bareos-Originalreferenz]({name})")
        elif name.startswith("dot-bvfs-"):
            groups["BVFS"].append(name)
        elif name.startswith("dot-ovfs-"):
            groups["OVFS"].append(name)
        elif name.startswith("dot-"):
            groups["Punktbefehle"].append(name)
        else:
            groups["Konsolenbefehle"].append(name)
    for title, names in groups.items():
        if not names:
            continue
        lines += ["", "<details>", f"<summary>{title}</summary>", ""]
        for name in names:
            label = "." + name[4:].replace("-", "_") if name.startswith("dot-") else name
            lines.append(f"- [{label}]({name})")
        lines += ["", "</details>"]
    lines += ["", "</details>"]
    return "\n".join(lines) + "\n"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("checkout", type=Path)
    parser.add_argument("--repo", default="Beerlesklopfer/onesimus-director")
    parser.add_argument("--branch", default="development")
    args = parser.parse_args()
    blob = f"https://github.com/{args.repo}/blob/{args.branch}"
    mapping = pages()
    out = args.checkout
    # Remove previously generated pages (all *.md except none are hand-made).
    for old in out.glob("*.md"):
        old.unlink()
    footer = (f"\n\n---\n*Erzeugt aus [`docs/wiki`]({blob}/docs/wiki) "
              f"(Branch `{args.branch}`) mit `scripts/publish_wiki.py`. "
              "Änderungen bitte dort vornehmen.*\n")
    for rel, name in mapping.items():
        text = (WIKI / rel).read_text(encoding="utf-8")
        (out / f"{name}.md").write_text(rewrite(text, rel, mapping, blob) + footer, encoding="utf-8")
    (out / "_Sidebar.md").write_text(sidebar(mapping), encoding="utf-8")
    print(f"{len(mapping)} pages written to {out}")


if __name__ == "__main__":
    main()
