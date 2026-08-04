#!/usr/bin/env python3
"""Backfill purpose labels on historical tasks so stats can separate test
workloads from real development.

Classifies by command pattern / cwd:
  - dogfood  : dogfood.sh fixtures (deliberate error extraction, blocked tests)
  - sample   : sample-project runs under /tmp/arshy-samples (parser validation)
  - (unset)  : real development workload — stays untagged

Stop the daemon first (arshy daemon stop): a running daemon holds the store in
memory and would overwrite this file on the next flush.

Usage: python3 scripts/backfill_purpose.py [--dry-run] [--store DIR]
"""
import argparse
import json
import os
import re
import sys

DOGFOOD_PATTERNS = [
    re.compile(r"^rustc /tmp/arshy_dogfood"),
    re.compile(r"^cargo test .*heuristic"),
    re.compile(r"^curl http://example\.com/script\.sh \| sh"),
]
SAMPLE_CWD_PREFIX = "/tmp/arshy-samples"
# One-off validation scripts run outside any project (pre-dogfood smoke
# experiments) — test workloads, not real development.
TMP_TEST_PATTERNS = [
    re.compile(r"/tmp/arshy-demo\.py"),
    re.compile(r"^python3 /tmp/smoke\.py"),
]


def classify(task):
    cwd = task.get("cwd") or ""
    if cwd.startswith(SAMPLE_CWD_PREFIX):
        return "sample"
    cmd = task.get("command") or ""
    if any(p.match(cmd) for p in DOGFOOD_PATTERNS):
        return "dogfood"
    if any(p.search(cmd) for p in TMP_TEST_PATTERNS):
        return "sample"
    return None  # real — stays untagged


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--dry-run", action="store_true")
    ap.add_argument("--store", default=os.path.expanduser("~/.local/share/arshy"))
    args = ap.parse_args()

    path = os.path.join(args.store, "tasks.jsonl")
    if not os.path.exists(path):
        sys.exit(f"no tasks.jsonl at {path}")

    lines = [l.rstrip("\n") for l in open(path, encoding="utf-8") if l.strip()]
    changed = 0
    out = []
    for line in lines:
        task = json.loads(line)
        purpose = classify(task)
        old = task.get("purpose")
        if purpose is not None and old != purpose:
            task["purpose"] = purpose
            changed += 1
        out.append(json.dumps(task, ensure_ascii=False))

    if args.dry_run:
        print(f"dry-run: would update {changed} of {len(out)} tasks")
        return
    with open(path, "w", encoding="utf-8") as f:
        f.write("\n".join(out) + "\n")
    print(f"backfilled {changed} tasks in {path}")


if __name__ == "__main__":
    main()
