"""The cross-file half of a per-file document, written from the lines alone.

A per-file world has no pass that reads the whole directory, so `purpose`,
`## Structure`, `## Rules` and `## Where to look` cannot come from the code —
they have to be synthesised from the assembled file lines. This writes that
pass's answer so the result can be scored like any other arm, and so the claims
it makes can be checked against the directory it never saw.

The slot wordings are copied out of `document.rs`'s `PROMPT`. Copied rather
than imported, for the reason `isolated.py` copies its own: a drift in the Rust
should surface here as a stale copy and not be followed silently.

Reads isolated_lines.json, writes synthesised.json. One model call per
directory.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from arms import LINE, isolated
from common import DIRECTORIES, REPO, as_json, ask, save

PROMPT = """You are writing the parts of a WARLOCK.md that are about a directory as a
whole. It is read by a model, not a person, before any source file is opened,
and its one job is routing: to say what is here and which file to open for a
given question. Warlock is the tool that lays the document out from your answer;
it is not the project being described, and its name belongs in no value unless
the files themselves use it.

You are not shown the source. You are shown the directory's name, and the line
already written for each file in it. Every claim you make must come from those
lines.

Directory: {name}

Lines:
{lines}

Fill in this object and output it and nothing else:

{{"purpose": "...", "structure": [], "rules": [], "lookups": []}}

"purpose": one to three sentences. What this directory is and what it does, in
the words a question about it would use.

"structure": how the files here fit together, one fact per entry: what calls
what, in what order, which way a dependency runs. Only what the lines you were
shown show. Each entry is {{"line": ..., "names": [...]}}: "line" is the fact in
prose, and "names" lists every file, directory, type, function or constant the
line refers to, spelt as the lines spell it. A structure entry names at least
one. An empty list is fine.

"rules": constraints this directory's own files state as rules, one per entry:
an invariant a comment asserts, a check the code makes, a setting a manifest
pins. Not something inferred. The same {{"line": ..., "names": [...]}} shape, and
a rule that refers to nothing leaves "names" empty. An empty list is fine.

"lookups": routes, each {{"for": ..., "open": ..., "symbol": ...}}. "for" is a
question or topic a reader might arrive with, in plain words. "open" is exactly
one of the filenames above. "symbol" is optional and must be a name that occurs
in that file's line. Prefer the routes a reader could not guess from the file
names.

At most 12 entries in each list, 280 characters per line, 700 for the purpose."""


def lines_of(rel: str) -> list[str]:
    return [line for line in isolated(rel).splitlines() if LINE.match(line)]


def main() -> None:
    out = {}
    for rel in DIRECTORIES:
        name = Path(rel).name
        answer = as_json(ask(PROMPT.format(name=name, lines="\n".join(lines_of(rel)))))
        if not answer:
            raise SystemExit(f"{rel}: the synthesis pass answered with no object")
        out[rel] = answer
        print(
            f"{rel}: {len(answer.get('structure', []))} structure, "
            f"{len(answer.get('rules', []))} rules, {len(answer.get('lookups', []))} lookups"
        )
    save("synthesised.json", out)


if __name__ == "__main__":
    main()
