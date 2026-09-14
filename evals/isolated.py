"""One line per file, written by a pass that saw only that file.

The instruction is warlock's own `"files"` wording, copied verbatim out of
`document.rs`'s `PROMPT`, so the only variable between these lines and a
committed document's is whether the pass could see the file's siblings. Copied
rather than imported: it is a Rust string constant, and a wording drift there
should show up here as a stale copy rather than be followed silently.

Reads skeletons.json, writes isolated_lines.json. One model call per file.
"""
import json
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from common import as_json, ask, load, save

FILES_RULE = (
    "What the file is and what it holds, naming the types, functions or "
    "constants a reader would come to it for, spelt as the file spells them. "
    "A file that is small, generated, or a re-export gets a line saying so."
)

ONE = """You are writing one line of a WARLOCK.md, the document that sits in a directory
of a codebase. It is read by a model, not a person, before any source file is
opened, and its one job is routing: to say what is here and which file to open
for a given question. Warlock is the tool that lays the document out from your
answer; it is not the project being described, and its name belongs in the line
unless the file itself uses it.

Write the line for `{name}` ({size}), the file below. {rule}

At most {cap} characters. Return JSON: {{"line": "..."}}

```
{body}
```"""

def main():
    skeletons = load("skeletons.json")
    jobs = []
    for directory, files in skeletons.items():
        for name, body in files.items():
            size = (Path(directory) / name).stat().st_size
            jobs.append((directory, name, size, body))

    def one(job):
        directory, name, size, body = job
        answer = as_json(ask(ONE.format(name=name, size=f"{size/1024:.1f} KB",
                                        rule=FILES_RULE, cap=280, body=body[:120000]))) or {}
        return directory, name, str(answer.get("line", "")).strip()

    with ThreadPoolExecutor(max_workers=10) as pool:
        out = list(pool.map(one, jobs))

    lines = {}
    for directory, name, line in out:
        lines.setdefault(directory, {})[name] = line
    save("isolated_lines.json", lines)
    n = sum(len(v) for v in lines.values())
    empty = sum(1 for v in lines.values() for line in v.values() if not line)
    over = sum(1 for v in lines.values() for line in v.values() if len(line) > 280)
    print(f"{n} lines written, {empty} empty, {over} over the 280 cap")

if __name__ == "__main__":
    main()
