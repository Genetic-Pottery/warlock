"""The maps under test, all built to the same token budget.

An arm is everything the reader is allowed to see. They are built here so that
one run compares like with like: the same 36 questions, the same model, and
budgets within a few percent of each other.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import EVALS, REPO, load, tokens

# `- \`name.rs\` (12.3 KB) — entry · declares \`a\`, \`b\` (+4)`, which is what
# `document::render` writes. Parsing warlock's own output is what lets an arm
# change one thing about a document and hold the rest still; if render changes
# its line, this stops matching and every arm built from it is wrong, which is
# why the tests in document.rs pin those bytes.
LINE = re.compile(r"^- `([^`]+)` \(([^)]+)\) — (.*?)(?: · declares .*)?$")

SECTIONS = ("## Structure", "## Rules", "## Where to look")


def document(rel: str) -> str:
    return (REPO / rel / "WARLOCK.md").read_text()


def declares(rel: str, shown: int) -> str:
    """The committed document with a different number of declared names.

    No model pass: `declares` is rendered from what warlock measured, so this
    is what `DECLARED_SHOWN` would have produced.
    """
    names_for = load("declared.json")[str(REPO / rel)]
    out = []
    for line in document(rel).splitlines():
        found = LINE.match(line)
        if not found:
            out.append(line)
            continue
        path, size, entry = found.groups()
        names = names_for.get(path, [])
        text = f"- `{path}` ({size}) — {entry}"
        if names:
            text += " · declares " + ", ".join(f"`{n}`" for n in names[:shown])
            if len(names) > shown:
                text += f" (+{len(names) - shown})"
        out.append(text)
    return "\n".join(out) + "\n"


def isolated(rel: str, shown: int = 16) -> str:
    """The same document with every file line replaced by one written alone.

    Sizes and declared names are untouched: those are warlock's measurements
    and not the pass's, so the only thing that differs is whether the pass
    could see the file's siblings.
    """
    lines = load("isolated_lines.json")[str(REPO / rel)]
    out = []
    for line in declares(rel, shown).splitlines():
        found = LINE.match(line)
        if not found:
            out.append(line)
            continue
        path, size, _ = found.groups()
        entry = lines.get(path)
        if entry is None:
            out.append(line)
            continue
        tail = line.split(" · declares ", 1)
        rebuilt = f"- `{path}` ({size}) — {entry}"
        if len(tail) == 2:
            rebuilt += " · declares " + tail[1]
        out.append(rebuilt)
    return "\n".join(out) + "\n"


def per_file(rel: str) -> str:
    """A whole document as per-file granularity would produce one.

    Isolated file lines, and the cross-file sections written by a pass that saw
    only those lines — no pass in this document ever read the source. Assembled
    the way `document::render` lays it out, so the only difference from the
    committed document is who wrote what.
    """
    synthesised = load("synthesised.json")[rel]
    text = isolated(rel)

    head, _, rest = text.partition("\n## Files")
    # The stamp, then the blank, then `# name`: everything down to the heading
    # is warlock's and stays, and the purpose under it is the pass's to replace.
    lines = head.splitlines()
    heading = lines[: next(i for i, line in enumerate(lines) if line.startswith("# ")) + 1]
    kept = []
    for line in rest.splitlines():
        if line.startswith("## ") and line.strip() in SECTIONS:
            break
        kept.append(line)

    out = heading + ["", synthesised["purpose"].strip(), "## Files"] + kept[1:]
    for name, entries in (("Structure", synthesised["structure"]), ("Rules", synthesised["rules"])):
        if entries:
            out += ["", f"## {name}", ""]
            out += [f"- {entry['line'].strip()}" for entry in entries]
    if synthesised["lookups"]:
        out += ["", "## Where to look", ""]
        for route in synthesised["lookups"]:
            symbol = route.get("symbol")
            tail = f" `{symbol}`" if symbol else ""
            out.append(f"- {route['for'].strip()} → `{route['open'].strip()}`{tail}")
    return "\n".join(out).replace("\n\n\n", "\n\n") + "\n"


def without_synthesis(text: str, drop_purpose: bool = False, sections=SECTIONS) -> str:
    out, skipping = [], False
    for line in text.splitlines():
        if line.startswith("## "):
            skipping = line.strip() in sections
        if not skipping:
            out.append(line)
    text = "\n".join(out) + "\n"
    if drop_purpose:
        head, _, rest = text.partition("## Files")
        text = "\n".join(head.splitlines()[:3]) + "\n\n## Files" + rest
    return text


def listing(rel: str) -> str:
    """The floor: names and sizes, no prose at all."""
    directory = REPO / rel
    return "\n".join(
        f"{p.name} ({p.stat().st_size // 1024} KB)"
        for p in sorted(directory.iterdir())
        if p.is_file() and p.name != "WARLOCK.md"
    )


def repomap(rel: str, budget: int) -> str:
    """Aider's tree-sitter and PageRank map, the bar a written line has to beat.

    Imported here rather than at module scope: aider needs its own virtualenv
    and Python 3.12, and every other arm runs without it.
    """
    from aider.io import InputOutput
    from aider.repomap import RepoMap

    class Shim:
        def token_count(self, text):
            return tokens(text)

    directory = (REPO / rel).resolve()
    files = sorted(
        str(p) for p in directory.iterdir() if p.is_file() and p.name != "WARLOCK.md"
    )
    mapper = RepoMap(
        map_tokens=budget,
        root=str(directory),
        main_model=Shim(),
        io=InputOutput(yes=True),
        refresh="always",
    )
    return mapper.get_repo_map([], files) or ""


def build(rel: str, names: list[str]) -> dict[str, str]:
    """Named arms for one directory, each sized against the committed document."""
    budget = tokens(declares(rel, 16))
    made = {}
    for name in names:
        if name == "document":
            made[name] = declares(rel, 16)
        elif name.startswith("declares"):
            made[name] = declares(rel, int(name.removeprefix("declares")))
        elif name == "isolated":
            made[name] = isolated(rel)
        elif name == "per_file":
            made[name] = per_file(rel)
        elif name == "no_routes":
            made[name] = without_synthesis(declares(rel, 16), sections=("## Where to look",))
        elif name == "document_no_synthesis":
            made[name] = without_synthesis(declares(rel, 16))
        elif name == "isolated_no_synthesis":
            made[name] = without_synthesis(isolated(rel), drop_purpose=True)
        elif name == "listing":
            made[name] = listing(rel)
        elif name == "repomap":
            made[name] = repomap(rel, budget)
        else:
            raise SystemExit(f"no arm named {name!r}")
    return made


if __name__ == "__main__":
    for rel in load("questions.json") and sorted({q["dir"] for q in load("questions.json")}):
        sizes = {n: tokens(t) for n, t in build(rel, sys.argv[1:] or ["document"]).items()}
        print(rel, sizes)
