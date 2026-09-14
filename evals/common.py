"""Shared plumbing: where things are, and how a question is put to a model.

Nothing here runs on its own. See README.md.
"""

from __future__ import annotations

import json
import re
import subprocess
from pathlib import Path

EVALS = Path(__file__).resolve().parent
REPO = EVALS.parent
RUNS = EVALS / "runs"

# The two directories every measurement so far has used: the only ones in this
# repository with enough files for routing to be a question rather than a
# glance.
DIRECTORIES = ("crates/warlock-engine/src", "crates/warlock-tui/src")

# The same model and effort `claude.rs` runs a document pass at, so a reader is
# being measured on the terms warlock actually ships.
MODEL = "claude-sonnet-5"
EFFORT = "low"

# Four bytes to the token. Every arm is sized by the same wrong number, which
# is what makes them comparable; nothing here is billed by it.
def tokens(text: str) -> int:
    return max(1, len(text) // 4)


def ask(prompt: str, system: str = "Answer with JSON only. No prose, no code fences.") -> str:
    out = subprocess.run(
        [
            "claude", "--print",
            "--model", MODEL,
            "--effort", EFFORT,
            # No tools and no settings: the arm under test is the whole of what
            # the reader may look at. A reader that could open the file would be
            # measuring the repository, not the map.
            "--tools", "",
            "--setting-sources", "",
            "--system-prompt", system,
            prompt,
        ],
        capture_output=True,
        text=True,
        timeout=300,
    )
    return out.stdout.strip()


def as_json(text: str) -> dict | None:
    text = text.strip()
    if text.startswith("```"):
        text = re.sub(r"^```[a-z]*\n|\n```$", "", text)
    try:
        return json.loads(text)
    except json.JSONDecodeError:
        found = re.search(r"\{.*\}", text, re.S)
        return json.loads(found.group(0)) if found else None


def load(name: str):
    return json.loads((EVALS / name).read_text())


def save(name: str, value) -> Path:
    path = EVALS / name
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(value, indent=2))
    return path
