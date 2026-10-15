#!/usr/bin/env python3
"""Score a benchmark run directory produced by run.py.

Metrics per (task, mode):
  success          — diagnose: last_message matches expected fields
                     fix: independent verify command exits 0 (meta.verify_pass)
  primary_hit      — diagnose only: location+message matched
  tool_rounds      — count of mcp_tool_call + command_execution item.completed
  retries          — repeated identical commands (heuristic)
  input_tokens / output_tokens / cached_input_tokens — from turn.completed.usage
                     (fix tasks: treat as total including repair rounds)
  path_compliance  — arshy mode used arshy_*; shell mode avoided arshy_*

Writes summary.json and summary.md into the run directory.
"""
from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

try:
    import yaml
except ImportError:
    yaml = None

REPO = Path(__file__).resolve().parents[2]
TASKS_PATH = REPO / "docs" / "benchmarks" / "tasks.json"


def load_tasks(path: Path) -> dict[str, dict]:
    text = path.read_text()
    if path.suffix == ".json":
        data = json.loads(text)
    elif yaml is not None:
        data = yaml.safe_load(text)
    else:
        sys.exit("Use docs/benchmarks/tasks.json (no PyYAML needed), or pip install pyyaml for .yaml")
    return {t["id"]: t for t in data["tasks"]}


def parse_events(path: Path) -> dict:
    tools = []
    usage = {}
    agent_texts = []
    errors = []
    if not path.exists() or path.stat().st_size == 0:
        return {"tools": tools, "usage": usage, "agent_texts": agent_texts, "errors": ["empty_events"]}
    with path.open() as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            try:
                o = json.loads(line)
            except json.JSONDecodeError:
                continue
            t = o.get("type")
            if t == "error":
                errors.append(o.get("message") or "error")
            if t == "turn.completed":
                usage = o.get("usage") or usage
            if t == "item.completed":
                item = o.get("item") or {}
                it = item.get("type")
                if it == "mcp_tool_call":
                    tools.append({
                        "kind": "mcp",
                        "server": item.get("server"),
                        "tool": item.get("tool"),
                        "arguments": item.get("arguments"),
                        "status": item.get("status"),
                        "result": item.get("result"),
                    })
                elif it == "command_execution":
                    tools.append({
                        "kind": "shell",
                        "command": item.get("command") or item.get("cmd"),
                        "exit_code": item.get("exit_code"),
                    })
                elif it == "agent_message":
                    agent_texts.append(item.get("text") or "")
    return {"tools": tools, "usage": usage, "agent_texts": agent_texts, "errors": errors}


def extract_report(text: str) -> dict:
    out = {"exit_code": None, "code": None, "message": None, "location": None}
    if not text:
        return out
    m = re.search(r"EXIT_CODE:\s*(\S+)", text, re.I)
    if m:
        out["exit_code"] = m.group(1).strip()
    m = re.search(r"CODE:\s*(.+)", text)
    if m:
        out["code"] = m.group(1).strip()
    m = re.search(r"MESSAGE:\s*(.+)", text)
    if m:
        out["message"] = m.group(1).strip()
    m = re.search(r"LOCATION:\s*(.+)", text)
    if m:
        out["location"] = m.group(1).strip()
    return out


def contains_any(hay: str, needles: list[str] | None) -> bool:
    if not needles:
        return True
    h = hay.lower()
    return any(n.lower() in h for n in needles)


def extract_fix_status(text: str) -> str | None:
    if not text:
        return None
    m = re.search(r"^STATUS:\s*(pass|fail)\b", text, re.I | re.M)
    return m.group(1).lower() if m else None


def score_task(task: dict, mode: str, task_dir: Path) -> dict:
    events_path = task_dir / "events.jsonl"
    last_path = task_dir / "last_message.txt"
    meta_path = task_dir / "meta.json"
    meta = json.loads(meta_path.read_text()) if meta_path.exists() else {}
    ev = parse_events(events_path)
    last = last_path.read_text() if last_path.exists() else ""
    report = extract_report(last)
    blob = last + "\n" + json.dumps(ev["tools"])
    exp = task.get("expected") or {}
    kind = task.get("kind") or meta.get("kind") or "diagnose"

    arshy_calls = [t for t in ev["tools"] if t.get("kind") == "mcp" and t.get("server") == "arshy"]
    shell_calls = [t for t in ev["tools"] if t.get("kind") == "shell"]
    cmds = []
    for t in ev["tools"]:
        if t.get("kind") == "shell":
            cmds.append(str(t.get("command")))
        elif t.get("kind") == "mcp" and t.get("tool") == "arshy_exec":
            args = t.get("arguments") or {}
            cmds.append(str(args.get("command") if isinstance(args, dict) else args))
    retries = 0
    seen = set()
    for c in cmds:
        if c in seen:
            retries += 1
        seen.add(c)

    if mode == "arshy":
        # Fix tasks may use host patch/write tools; still forbid shell for builds.
        path_compliance = len(arshy_calls) > 0 and len(shell_calls) == 0
    else:
        path_compliance = len(arshy_calls) == 0 and (len(shell_calls) > 0 or kind == "fix")

    usage = ev.get("usage") or {}

    if kind == "fix":
        verify_pass = bool(meta.get("verify_pass"))
        claimed = extract_fix_status(last)
        success = verify_pass
        return {
            "task_id": task["id"],
            "mode": mode,
            "kind": kind,
            "success": success,
            "verify_pass": verify_pass,
            "claimed_status": claimed,
            "primary_hit": verify_pass,
            "path_compliance": path_compliance,
            "tool_rounds": len(ev["tools"]),
            "arshy_calls": len(arshy_calls),
            "shell_calls": len(shell_calls),
            "retries": retries,
            "input_tokens": usage.get("input_tokens"),
            "output_tokens": usage.get("output_tokens"),
            "cached_input_tokens": usage.get("cached_input_tokens"),
            "elapsed_sec": meta.get("elapsed_sec"),
            "codex_status": meta.get("status"),
            "report": report,
            "errors": ev.get("errors")[:5],
        }

    # diagnose success heuristics
    exit_ok = True
    if exp.get("exit_code") is not None:
        exit_ok = str(report["exit_code"]) == str(exp["exit_code"]) or str(exp["exit_code"]) in blob
    code_ok = True
    if exp.get("code"):
        code_ok = (exp["code"].lower() in (report.get("code") or "").lower()) or (exp["code"] in blob)
    msg_ok = contains_any(blob, exp.get("message_contains"))
    loc_ok = True
    if exp.get("location_file"):
        loc_ok = exp["location_file"] in blob
        if loc_ok and exp.get("location_line") is not None:
            line = str(exp["location_line"])
            loc_ok = (
                f"{exp['location_file']}:{line}" in blob
                or f"{exp['location_file']}:{line}:" in blob
                or (exp["location_file"] in blob and line in (report.get("location") or blob))
            )
    success = bool(exit_ok and code_ok and msg_ok and loc_ok and meta.get("status") in ("ok", "codex_nonzero", None))

    return {
        "task_id": task["id"],
        "mode": mode,
        "kind": kind,
        "success": success,
        "primary_hit": loc_ok and msg_ok,
        "exit_ok": exit_ok,
        "code_ok": code_ok,
        "msg_ok": msg_ok,
        "loc_ok": loc_ok,
        "path_compliance": path_compliance,
        "tool_rounds": len(ev["tools"]),
        "arshy_calls": len(arshy_calls),
        "shell_calls": len(shell_calls),
        "retries": retries,
        "input_tokens": usage.get("input_tokens"),
        "output_tokens": usage.get("output_tokens"),
        "cached_input_tokens": usage.get("cached_input_tokens"),
        "elapsed_sec": meta.get("elapsed_sec"),
        "codex_status": meta.get("status"),
        "report": report,
        "errors": ev.get("errors")[:5],
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("run_dir", type=Path)
    ap.add_argument("--tasks-file", type=Path, default=TASKS_PATH)
    args = ap.parse_args()
    catalog = load_tasks(args.tasks_file)
    results = []
    for meta_path in sorted(args.run_dir.glob("*/*/meta.json")):
        # pattern taskid__mode/meta.json — actually run.py uses out/task__mode/meta.json
        pass
    for meta_path in sorted(args.run_dir.glob("*/meta.json")):
        meta = json.loads(meta_path.read_text())
        tid, mode = meta.get("task_id"), meta.get("mode")
        if tid not in catalog:
            continue
        results.append(score_task(catalog[tid], mode, meta_path.parent))

    # Also accept index-driven discovery
    if not results and (args.run_dir / "index.json").exists():
        for meta in json.loads((args.run_dir / "index.json").read_text()):
            tid, mode = meta.get("task_id"), meta.get("mode")
            if tid not in catalog:
                continue
            task_dir = args.run_dir / f"{tid}__{mode}"
            if task_dir.is_dir():
                results.append(score_task(catalog[tid], mode, task_dir))

    by_mode: dict[str, list] = {}
    for r in results:
        by_mode.setdefault(r["mode"], []).append(r)

    summary = {"runs": results, "by_mode": {}}
    lines = ["# Benchmark summary", ""]
    for mode, rs in sorted(by_mode.items()):
        n = len(rs)
        succ = sum(1 for r in rs if r["success"])
        hit = sum(1 for r in rs if r["primary_hit"])
        comp = sum(1 for r in rs if r["path_compliance"])
        rounds = sum(r["tool_rounds"] for r in rs) / n if n else 0
        retries = sum(r["retries"] for r in rs) / n if n else 0
        tin = sum(r["input_tokens"] or 0 for r in rs)
        tout = sum(r["output_tokens"] or 0 for r in rs)
        summary["by_mode"][mode] = {
            "n": n,
            "success_rate": succ / n if n else 0,
            "primary_hit_rate": hit / n if n else 0,
            "path_compliance_rate": comp / n if n else 0,
            "avg_tool_rounds": rounds,
            "avg_retries": retries,
            "total_input_tokens": tin,
            "total_output_tokens": tout,
        }
        lines += [
            f"## mode={mode}",
            f"- n={n} success={succ}/{n} ({succ/n:.0%})" if n else f"- n=0",
            f"- primary_hit={hit}/{n}",
            f"- path_compliance={comp}/{n}",
            f"- avg tool_rounds={rounds:.2f} avg retries={retries:.2f}",
            f"- tokens in={tin} out={tout}",
            "",
        ]

    lines.append("## per task")
    lines.append("| task | kind | mode | success | primary/verify | comply | rounds | retries | in_tok | elapsed |")
    lines.append("|---|---|---|---|---|---|---|---|---|---|")
    for r in results:
        lines.append(
            f"| {r['task_id']} | {r.get('kind','diagnose')} | {r['mode']} | {r['success']} | {r['primary_hit']} | "
            f"{r['path_compliance']} | {r['tool_rounds']} | {r['retries']} | {r['input_tokens']} | {r.get('elapsed_sec')} |"
        )

    (args.run_dir / "summary.json").write_text(json.dumps(summary, indent=2))
    (args.run_dir / "summary.md").write_text("\n".join(lines) + "\n")
    print("\n".join(lines))
    print(f"\nwrote {args.run_dir / 'summary.json'}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
