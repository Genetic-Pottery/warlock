"""Build the question set: routing questions with a gold file and symbol.

Questions come from the source and never from a document. A document is one of
the things on trial, so a question phrased out of its own `## Where to look`
line would be scoring the arms on a test one of them wrote.

Only names unique within their directory are used, because a gold answer two
files could satisfy is not a gold answer, and any generated question that spells
its own symbol or filename is thrown away — about half of them are.

Writes questions.json. Costs roughly two model calls per kept question, so run
it when the question set needs rebuilding and not before.
"""

import json
import random
import re
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import DIRECTORIES as DIRS, EVALS as S, REPO, as_json, ask
DECL = re.compile(r"^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const|static|fn|struct|enum|trait|type)\s+([A-Za-z_][A-Za-z0-9_]*)", re.M)

def symbols(directory):
    """Every declared name in the directory, and how many files declare it."""
    seen = {}
    for path in sorted(directory.iterdir()):
        if not path.is_file() or path.suffix != ".rs":
            continue
        body = path.read_text(errors="replace")
        # Only the part before the test module: a name that exists solely in
        # tests is not something anybody routes to.
        body = body.split("\nmod tests {")[0]
        for name in set(DECL.findall(body)):
            seen.setdefault(name, []).append(path.name)
    return seen

def slice_of(path, name):
    body = path.read_text(errors="replace")
    m = re.search(rf"^[^\n]*\b{re.escape(name)}\b[^\n]*$", body, re.M)
    if not m:
        return None
    start = max(0, body.rfind("\n", 0, max(0, m.start() - 600)))
    return body[start:m.end() + 1200]

QUESTION = """Below is a slice of Rust source from one file of a directory.

Write ONE question a developer unfamiliar with this codebase might ask, whose
correct answer is "open {file} and look at {name}".

Rules:
- The question must describe the BEHAVIOUR or the PROBLEM, never the name.
- It must not contain the word "{name}" (or its parts), or the filename.
- It must be answerable by someone choosing between files in a directory.
- One sentence, lowercase, no question mark needed.

Return JSON: {{"question": "..."}}

The slice:
```rust
{slice}
```"""

def build_questions(per_dir=20, seed=11):
    random.seed(seed)
    out = []
    for rel in DIRS:
        directory = REPO / rel
        counts = symbols(directory)
        # Unique names only: a gold answer that two files could satisfy is not
        # a gold answer.
        unique = [(n, f[0]) for n, f in sorted(counts.items())
                  if len(f) == 1 and len(n) > 4 and not n.isupper()]
        random.shuffle(unique)
        picked, files_used = [], set()
        for name, filename in unique:
            if filename in files_used:
                continue
            body = slice_of(directory / filename, name)
            if not body or len(body) < 400:
                continue
            picked.append((name, filename, body))
            files_used.add(filename)
            if len(picked) == per_dir:
                break
        for name, filename, body in picked:
            out.append({"dir": rel, "file": filename, "symbol": name, "slice": body})

    def one(item):
        answer = as_json(ask(QUESTION.format(file=item["file"], name=item["symbol"], slice=item["slice"][:6000])))
        item["question"] = (answer or {}).get("question", "")
        return item

    with ThreadPoolExecutor(max_workers=8) as pool:
        out = list(pool.map(one, out))

    kept = []
    for item in out:
        q = item.get("question", "").lower()
        parts = [p for p in re.split(r"[_]", item["symbol"].lower()) if len(p) > 3]
        stem = item["file"].rsplit(".", 1)[0].lower()
        if not q:
            continue
        if item["symbol"].lower() in q or stem in q or any(p in q for p in parts):
            item["leak"] = True          # kept out: the question names its own answer
            continue
        kept.append({k: v for k, v in item.items() if k != "slice"})
    path = S / "questions.json"
    have = json.loads(path.read_text()) if path.exists() else []
    seen = {(q["dir"], q["file"]) for q in have}
    added = [k for k in kept if (k["dir"], k["file"]) not in seen]
    path.write_text(json.dumps(have + added, indent=2))
    print(f"{len(kept)} kept of {len(out)} generated; {len(added)} new, {len(have) + len(added)} total")

if __name__ == "__main__":
    # Per-directory sample size and seed: a second seed tops the set up with
    # files the first one missed, which is how 36 was reached.
    build_questions(
        per_dir=int(sys.argv[1]) if len(sys.argv) > 1 else 20,
        seed=int(sys.argv[2]) if len(sys.argv) > 2 else 11,
    )
