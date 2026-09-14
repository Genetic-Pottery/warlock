"""Check what a synthesised document asserts against the directory it describes.

Routing accuracy cannot see a false claim: a document that says `pact.rs` calls
something it does not call still sends a reader to the right file. This is the
other half — every name a `structure` or `rules` entry carries, and every lookup
target, put to the same evidence `Expected::knows` uses in the engine: a file or
child directory of the directory, or a token some file in it actually holds.

    python3 evals/claims.py synthesised.json

Mechanical and free. It proves nothing about whether a true-looking sentence is
true; it catches the names that were never there at all.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import REPO, load


def evidence(rel: str) -> tuple[set[str], str]:
    directory = REPO / rel
    here = {p.name for p in directory.iterdir() if p.is_file()}
    here |= {p.name for p in directory.iterdir() if p.is_dir()}
    text = ""
    for path in sorted(directory.iterdir()):
        if path.is_file() and path.name != "WARLOCK.md":
            text += path.read_text(errors="replace")
    return here, text


def known(name: str, here: set[str], text: str) -> bool:
    name = name.strip()
    return bool(name) and (name in here or name in text)


def main() -> None:
    answers = json.loads(Path(sys.argv[1]).read_text())
    for rel, fill in answers.items():
        here, text = evidence(rel)
        checked = failed = 0
        unknown = []
        for section in ("structure", "rules"):
            for index, entry in enumerate(fill.get(section, [])):
                for name in entry.get("names", []):
                    checked += 1
                    if not known(name, here, text):
                        failed += 1
                        unknown.append(f"{section}[{index}].names: {name!r}")
        for index, route in enumerate(fill.get("lookups", [])):
            target = route.get("open", "")
            checked += 1
            if target not in here:
                failed += 1
                unknown.append(f"lookups[{index}].open: {target!r}")
            symbol = route.get("symbol")
            if symbol:
                checked += 1
                # The engine checks a symbol against the text of the file the
                # route opens, not the directory: a name that exists elsewhere
                # is still a wrong route.
                opened = REPO / rel / target
                body = opened.read_text(errors="replace") if opened.is_file() else ""
                if symbol not in body:
                    failed += 1
                    unknown.append(f"lookups[{index}].symbol: {symbol!r} not in {target}")
        print(f"{rel}: {checked - failed}/{checked} names hold ({failed} would be refused)")
        for line in unknown:
            print(f"    {line}")


if __name__ == "__main__":
    main()
