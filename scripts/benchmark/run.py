#!/usr/bin/env python3
"""Run Arshy diagnose / fix-until-pass benchmark against Codex.

Modes:
  shell  — built-in shell only (arshy MCP disabled)
  arshy  — force arshy_exec via prompt (arshy MCP enabled)

Task kinds (tasks.json `kind`):
  diagnose (default) — identify primary failure, do not edit
  fix                — edit until `verify` command exits 0

Examples:
  # Smoke one diagnose task both ways on NotBot:
  python3 scripts/benchmark/run.py --task rust-type-mismatch --modes shell,arshy

  # Hard fix tasks only:
  python3 scripts/benchmark/run.py --tag hard --modes shell,arshy

  # Full suite offline diagnose tasks:
  python3 scripts/benchmark/run.py --tag '!network' --tag '!optional' --tag '!fix'

Stdout is human progress; JSONL results go to --out (default docs/benchmarks/results/).
Codex --json events are saved under the same run directory.
"""
from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path

try:
    import yaml
except ImportError:
    yaml = None

REPO = Path(__file__).resolve().parents[2]
TASKS_PATH = REPO / "docs" / "benchmarks" / "tasks.json"
FIXTURES = REPO / "docs" / "benchmarks" / "fixtures"
PROMPTS = REPO / "docs" / "benchmarks" / "prompts"


def load_tasks(path: Path) -> dict:
    text = path.read_text()
    if path.suffix == ".json":
        return json.loads(text)
    if yaml is not None:
        return yaml.safe_load(text)
    sys.exit("Use docs/benchmarks/tasks.json (no PyYAML needed), or pip install pyyaml for .yaml")


def tag_match(task: dict, include: list[str], exclude: list[str]) -> bool:
    tags = set(task.get("tags") or [])
    for t in include:
        if t not in tags:
            return False
    for t in exclude:
        if t in tags:
            return False
    return True


def which_ok(req: str) -> bool:
    if req == "network":
        return True  # checked separately / assumed
    if req == "python3":
        return shutil.which("python3") is not None
    return shutil.which(req) is not None


def prepare_workdir(task: dict, run_root: Path) -> Path:
    fixture = task["fixture"]
    src = FIXTURES / fixture
    if not src.is_dir():
        raise FileNotFoundError(f"fixture missing: {src}")
    dest = (run_root / "work" / task["id"]).resolve()
    if dest.exists():
        shutil.rmtree(dest)
    shutil.copytree(src, dest, ignore=shutil.ignore_patterns("node_modules", "target", ".git"))
    setup = task.get("setup")
    if setup:
        subprocess.run(setup, shell=True, cwd=dest, check=True)
    return dest


def render_prompt(mode: str, command: str, kind: str = "diagnose") -> str:
    prefix = "fix" if kind == "fix" else "diagnose"
    name = f"{prefix}_arshy.txt" if mode == "arshy" else f"{prefix}_shell.txt"
    tmpl = (PROMPTS / name).read_text()
    return tmpl.replace("{{command}}", command)


def codex_cmd(mode: str, workdir: Path, prompt: str, model: str | None,
              events_path: Path, last_path: Path) -> list[str]:
    cmd = [
        "codex", "exec",
        "--ephemeral",
        "--approve-for-me",
        "--skip-git-repo-check",
        "--json",
        "--color", "never",
        "-C", str(workdir),
        "-o", str(last_path),
    ]
    # Reduce MCP noise / tokens for both modes (names present in NotBot config)
    for srv in ("node_repl", "computer-use"):
        cmd.extend(["-c", f"mcp_servers.{srv}.enabled=false"])
    if mode == "shell":
        cmd.extend(["-c", "mcp_servers.arshy.enabled=false"])
    else:
        cmd.extend(["-c", "mcp_servers.arshy.enabled=true"])
    if model:
        cmd.extend(["-m", model])
    cmd.append(prompt)
    return cmd


def run_verify(workdir: Path, verify_cmd: str, timeout: int) -> dict:
    """Independently check whether the project now passes."""
    try:
        proc = subprocess.run(
            verify_cmd,
            shell=True,
            cwd=str(workdir),
            capture_output=True,
            text=True,
            timeout=timeout,
        )
        return {
            "exit_code": proc.returncode,
            "stdout_tail": (proc.stdout or "")[-2000:],
            "stderr_tail": (proc.stderr or "")[-2000:],
        }
    except subprocess.TimeoutExpired:
        return {"exit_code": 124, "stdout_tail": "", "stderr_tail": "verify timeout"}


def run_one(task: dict, mode: str, out_dir: Path, model: str | None,
            timeout: int, dry_cmd: bool) -> dict:
    task_dir = (out_dir / f"{task['id']}__{mode}").resolve()
    task_dir.mkdir(parents=True, exist_ok=True)
    work = prepare_workdir(task, task_dir)
    kind = task.get("kind") or "diagnose"
    prompt = render_prompt(mode, task.get("verify") or task["command"], kind=kind)
    (task_dir / "prompt.txt").write_text(prompt)
    events = task_dir / "events.jsonl"
    last = task_dir / "last_message.txt"
    stderr_path = task_dir / "stderr.txt"
    cmd = codex_cmd(mode, work, prompt, model, events, last)
    meta = {
        "task_id": task["id"],
        "mode": mode,
        "kind": kind,
        "command": task["command"],
        "verify": task.get("verify") or task["command"],
        "workdir": str(work),
        "started_at": datetime.now(timezone.utc).isoformat(),
        "codex_argv_preview": cmd[:12] + ["<prompt>", *cmd[13:]] if False else None,
    }
    # Store argv without embedding full prompt twice
    meta["codex_argv"] = cmd[:-1] + ["<prompt from prompt.txt>"]
    if dry_cmd:
        meta["status"] = "dry"
        (task_dir / "meta.json").write_text(json.dumps(meta, indent=2))
        return meta

    t0 = time.time()
    with open(events, "wb") as ev, open(stderr_path, "wb") as err:
        # Critical: close stdin so Codex does not wait for more input.
        proc = subprocess.run(
            cmd,
            stdin=subprocess.DEVNULL,
            stdout=ev,
            stderr=err,
            cwd=str(work),
            timeout=timeout,
        )
    elapsed = time.time() - t0
    meta.update({
        "status": "ok" if proc.returncode == 0 else "codex_nonzero",
        "codex_exit": proc.returncode,
        "elapsed_sec": round(elapsed, 2),
        "events_path": str(events),
        "last_message_path": str(last),
        "finished_at": datetime.now(timezone.utc).isoformat(),
        "kind": kind,
    })
    if kind == "fix" and not dry_cmd:
        verify_cmd = task.get("verify") or task["command"]
        verify = run_verify(work, verify_cmd, min(timeout, 180))
        meta["verify"] = verify
        meta["verify_pass"] = verify.get("exit_code") == 0
        (task_dir / "verify.json").write_text(json.dumps(verify, indent=2))
    (task_dir / "meta.json").write_text(json.dumps(meta, indent=2))
    return meta


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--tasks-file", type=Path, default=TASKS_PATH)
    ap.add_argument("--task", action="append", default=[], help="Task id (repeatable)")
    ap.add_argument("--tag", action="append", default=[],
                    help="Include tag, or !tag to exclude (repeatable)")
    ap.add_argument("--modes", default="shell,arshy", help="Comma list: shell,arshy")
    ap.add_argument("--model", default=None)
    ap.add_argument("--out", type=Path, default=None,
                    help="Results root (default docs/benchmarks/results/<timestamp>)")
    ap.add_argument("--timeout", type=int, default=None)
    ap.add_argument("--dry-run", action="store_true", help="Prepare workdirs + prompts only")
    ap.add_argument("--list", action="store_true", help="List matching tasks and exit")
    ap.add_argument("--skip-missing-requires", action="store_true", default=True)
    args = ap.parse_args()

    catalog = load_tasks(args.tasks_file)
    tasks = catalog["tasks"]
    include, exclude = [], []
    for t in args.tag:
        if t.startswith("!"):
            exclude.append(t[1:])
        else:
            include.append(t)
    if args.task:
        want = set(args.task)
        tasks = [t for t in tasks if t["id"] in want]
    else:
        tasks = [t for t in tasks if tag_match(t, include, exclude)]

    filtered = []
    for t in tasks:
        missing = [r for r in (t.get("requires") or []) if r != "network" and not which_ok(r)]
        if missing and args.skip_missing_requires:
            print(f"skip {t['id']}: missing {missing}", file=sys.stderr)
            continue
        filtered.append(t)
    tasks = filtered

    if args.list:
        for t in tasks:
            print(f"{t['id']}\t{t.get('command')}\t{','.join(t.get('tags') or [])}")
        return 0

    modes = [m.strip() for m in args.modes.split(",") if m.strip()]
    for m in modes:
        if m not in ("shell", "arshy"):
            sys.exit(f"unknown mode: {m}")

    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    out = (args.out or (REPO / "docs" / "benchmarks" / "results" / stamp)).resolve()
    out.mkdir(parents=True, exist_ok=True)
    timeout = args.timeout or int(catalog.get("default_timeout_sec") or 180)
    model = args.model or catalog.get("default_model")

    index = []
    print(f"run dir: {out}")
    for t in tasks:
        for mode in modes:
            print(f"→ {t['id']} / {mode} ...", flush=True)
            try:
                task_timeout = int(t.get("timeout_sec") or timeout)
                meta = run_one(t, mode, out, model, task_timeout, args.dry_run)
            except subprocess.TimeoutExpired:
                meta = {"task_id": t["id"], "mode": mode, "status": "timeout"}
                print(f"  timeout", flush=True)
            except Exception as e:
                meta = {"task_id": t["id"], "mode": mode, "status": "error", "error": str(e)}
                print(f"  error: {e}", flush=True)
            else:
                print(f"  {meta.get('status')} in {meta.get('elapsed_sec', '?')}s", flush=True)
            index.append(meta)
            with open(out / "index.jsonl", "a") as f:
                f.write(json.dumps(meta) + "\n")

    (out / "index.json").write_text(json.dumps(index, indent=2))
    print(f"done: {len(index)} runs → {out}")
    print(f"score with: python3 scripts/benchmark/score.py {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
