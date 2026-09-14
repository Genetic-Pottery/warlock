"""Count hits in a run.

    python3 evals/score.py runs/isolation-arms.json

Two numbers per arm: whether the right file was named, and whether the right
file and symbol both were. The file is the number that matters — it is what a
document is for — and the symbol number is low for every arm ever measured,
because a gold name is often internal and a file line carries only its first
`DECLARED_SHOWN`.

Read differences against the noise floor, not against zero: identical content
has scored 91.7% and 87.0% on two runs, so about four answers in 108 is the
smallest thing worth claiming.
"""

from __future__ import annotations

import json
import sys
from collections import defaultdict
from pathlib import Path


def file_hit(row: dict) -> bool:
    return row["said_file"].strip().rsplit("/", 1)[-1].lower() == row["file"].lower()


def symbol_hit(row: dict) -> bool:
    said = row["said_symbol"].strip().lower().strip("`()")
    return said == row["symbol"].lower() or row["symbol"].lower() in said.split("::")


def main() -> None:
    path = Path(sys.argv[1])
    rows = json.loads(path.read_text())
    per_arm = defaultdict(lambda: defaultdict(lambda: {"file": 0, "both": 0, "n": 0}))
    for row in rows:
        tally = per_arm[row["arm"]][row.get("run", 0)]
        tally["n"] += 1
        tally["file"] += file_hit(row)
        tally["both"] += file_hit(row) and symbol_hit(row)

    print(f"{'arm':24} {'right file':>16} {'right file+symbol':>20}")
    for arm, runs in per_arm.items():
        files = [runs[r]["file"] for r in sorted(runs)]
        both = [runs[r]["both"] for r in sorted(runs)]
        total = sum(runs[r]["n"] for r in runs)
        each = f"  {files}" if len(files) > 1 else ""
        print(
            f"{arm:24} {sum(files):>4}/{total:<4} {sum(files) / total * 100:5.1f}%"
            f" {sum(both):>6}/{total:<4} {sum(both) / total * 100:5.1f}%{each}"
        )

    # Where the arms disagree, which is the part worth reading by hand: a
    # percentage says one arm is better and this says on what.
    if len(per_arm) == 2:
        first, second = per_arm.keys()
        by_question = defaultdict(dict)
        for row in rows:
            by_question[(row["file"], row["symbol"])].setdefault(row["arm"], []).append(file_hit(row))
        for arm, other in ((first, second), (second, first)):
            names = [
                f"{k[0]}::{k[1]}"
                for k, v in sorted(by_question.items())
                if all(v.get(arm, [])) and not any(v.get(other, [True]))
            ]
            if names:
                print(f"\n{arm} right where {other} was wrong:")
                for name in names:
                    print(f"  {name}")


if __name__ == "__main__":
    main()
