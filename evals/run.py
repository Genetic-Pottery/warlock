"""Put the question set to one or more arms and record what came back.

    python3 evals/run.py <name> document repomap listing

Costs one model call per question per arm — 36 questions, so an arm is 36
calls. Writes runs/<name>.json.

An arm run more than once is how the noise floor is measured: pass --runs 3 and
score.py reports each run separately, because the same content has scored 91.7%
and 87.0% on two passes and nothing smaller than that spread is a finding.
"""

from __future__ import annotations

import argparse
import sys
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from arms import build
from common import RUNS, as_json, ask, load, save

ASK = """You are deciding which single file to open in the directory `{rel}`, and
which name in it to look at. All you have is the map below.

--- map of {rel} ---
{context}
--- end of map ---

Question: {question}

Answer with JSON only: {{"file": "<one filename from that directory>", "symbol": "<one name>"}}"""


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("name", help="what to call this run: runs/<name>.json")
    parser.add_argument("arms", nargs="+", help="document, declares8, isolated, listing, repomap, ...")
    parser.add_argument("--runs", type=int, default=1, help="times to repeat each arm")
    parser.add_argument("--workers", type=int, default=8)
    args = parser.parse_args()

    questions = load("questions.json")
    built = {rel: build(rel, args.arms) for rel in sorted({q["dir"] for q in questions})}

    jobs = [
        (question, arm, run)
        for question in questions
        for arm in args.arms
        for run in range(args.runs)
    ]

    def one(job):
        question, arm, run = job
        answer = as_json(
            ask(ASK.format(rel=question["dir"], context=built[question["dir"]][arm], question=question["question"]))
        ) or {}
        return {
            **{k: question[k] for k in ("dir", "file", "symbol")},
            "arm": arm,
            "run": run,
            "said_file": str(answer.get("file", "")),
            "said_symbol": str(answer.get("symbol", "")),
        }

    with ThreadPoolExecutor(max_workers=args.workers) as pool:
        results = list(pool.map(one, jobs))

    path = save(str(Path("runs") / f"{args.name}.json"), results)
    print(f"{len(results)} answers -> {path.relative_to(RUNS.parent.parent)}")


if __name__ == "__main__":
    main()
