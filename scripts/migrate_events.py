#!/usr/bin/env python3
"""
Migration script: reprocess existing events through the optimized pipeline.

Applies two transformations that match the Rust daemon logic:
1. Noise filtering  (dedup.rs: strip ANSI, remove blank/whitespace-only log lines)
2. Rustc context merging (parser/mod.rs: RustcContextMerger absorbs context lines
   into the preceding diagnostic/log event's context.after)

Usage:
    python3 scripts/migrate_events.py            # run migration
    python3 scripts/migrate_events.py --dry-run  # preview only
"""

import json
import os
import re
import sys
import tempfile
from pathlib import Path

# ---------------------------------------------------------------------------
# ANSI stripping (matches dedup.rs ANSI_REGEX)
# ---------------------------------------------------------------------------
ANSI_RE = re.compile(r"\x1b\[[0-9;]*m")


def strip_ansi(text: str) -> str:
    if "\x1b" not in text:
        return text
    return ANSI_RE.sub("", text)


def is_noise_line(message: str) -> bool:
    return strip_ansi(message).strip() == ""


# ---------------------------------------------------------------------------
# Rustc context-line detection (matches is_rustc_context_line in parser/mod.rs)
# Uses string operations, not regex, matching the Rust implementation exactly.
# ---------------------------------------------------------------------------
def is_rustc_context_line(msg: str) -> bool:
    trimmed = msg.strip()
    if not trimmed:
        return False

    # Pattern 1: = note: ... , = help: ... , = warning: ...
    if trimmed.startswith("= "):
        rest = trimmed[2:]
        colon_pos = rest.find(":")
        if colon_pos > 0:
            directive = rest[:colon_pos]
            if directive.isascii() and directive.isalpha():
                return True

    # Pattern 2: pipe markers -- | , | expected ... , |     ^^^^ ...
    if trimmed.startswith("|"):
        return True

    # Pattern 3: numbered source lines -- 2 | code
    pipe_pos = trimmed.find(" | ")
    if pipe_pos > 0:
        num_part = trimmed[:pipe_pos]
        if num_part and all(c.isdigit() or c == " " for c in num_part):
            return True

    # Pattern 4: arrow markers --> file.rs:1:1
    if trimmed.startswith("-->"):
        return True

    return False


# ---------------------------------------------------------------------------
# RustcContextMerger -- faithful port of the Rust struct
# ---------------------------------------------------------------------------
class RustcContextMerger:
    def __init__(self):
        self.pending = None
        self.context_lines = []
        self.merged_count = 0

    def feed(self, event):
        """Returns list of events to emit (0 or 1).

        Only absorbs raw log events that match rustc context patterns.
        Only buffers diagnostic events WITH location (those benefit from context).
        All other events pass through immediately.
        """
        etype = event.get("type", "")
        msg = event.get("message", "")

        # Only absorb raw log events as context lines
        is_raw = (
            etype == "log"
            and "location" not in event
            and "context" not in event
        )
        if is_raw and is_rustc_context_line(msg):
            self.context_lines.append(msg)
            self.merged_count += 1
            return []

        # Flush any pending diagnostic (attach accumulated context)
        flushed = self._flush_pending()

        # Only buffer diagnostic events WITH location (file:line errors)
        # Diagnostics without location don't benefit from source context.
        if etype == "diagnostic" and event.get("location"):
            self.pending = event
            return flushed

        # All other events emit immediately
        return flushed + [event]

    def finish(self):
        return self._flush_pending()

    def _flush_pending(self):
        if self.pending is None:
            return []
        event = self.pending
        self.pending = None
        if self.context_lines:
            # Filter out noise lines: empty pipes, pure caret/arrow markers
            merged = []
            for line in self.context_lines:
                trimmed = line.strip()
                if trimmed == "|":
                    continue
                if trimmed.startswith("-->"):
                    continue
                if trimmed.startswith("|"):
                    after_pipe = trimmed[1:].strip()
                    if not after_pipe or all(c in "^-" for c in after_pipe):
                        continue
                merged.append(line)
            self.context_lines = []
            if merged:
                ctx = event.get("context")
                if ctx:
                    ctx.setdefault("after", []).extend(merged)
                else:
                    event["context"] = {
                        "before": [],
                        "line": event.get("message", ""),
                        "after": merged,
                    }
        return [event]


# ---------------------------------------------------------------------------
# Metrics computation
# ---------------------------------------------------------------------------
def compute_metrics(events):
    agent_visible = 0
    agent_skipped = 0
    locations = 0
    codes = 0
    contexts = 0
    hints = 0
    total_bytes = 0

    for ev in events:
        etype = ev.get("type", "")
        if etype == "log":
            agent_skipped += 1
        else:
            agent_visible += 1

        if ev.get("location"):
            locations += 1
        code = ev.get("code")
        if code:
            codes += 1
        if ev.get("context"):
            contexts += 1
        if ev.get("hints"):
            hints += 1

        total_bytes += len(json.dumps(ev, ensure_ascii=False).encode("utf-8"))

    return {
        "raw_output_bytes": 0,          # not available from stored events
        "structured_events_bytes": total_bytes,
        "agent_visible_events": agent_visible,
        "agent_skipped_events": agent_skipped,
        "locations_extracted": locations,
        "codes_extracted": codes,
        "contexts_enriched": contexts,
        "hints_attached": hints,
    }


# ---------------------------------------------------------------------------
# Parser pattern matching (applies new TOML patterns to existing log events)
# ---------------------------------------------------------------------------
CARGO_PATTERNS = [
    (re.compile(r'^Finished\s+`\S+`\s+profile\s+\[.+?\]\s+target\(s\)\s+in\s+([\d.]+)s$'), "summary"),
    (re.compile(r'^Compiling\s+(\S+)\s+v([\d.]+(?:\S*)?)\s*(?:\((.+?)\))?'), "summary"),
    (re.compile(r'^Checking\s+(\S+)\s+v([\d.]+(?:\S*)?)\s*(?:\((.+?)\))?'), "summary"),
    (re.compile(r'^Downloading\s+(\S+)'), "summary"),
    # cargo-test patterns
    (re.compile(r'^Running\s+unittests\s+\S+\s+\((.+)\)'), "summary"),
    (re.compile(r'^running\s+(\d+)\s+tests?$'), "summary"),
    (re.compile(r'^Doc-tests\s+(\S+)'), "summary"),
    (re.compile(r'^test result:\s+\w+'), "summary"),
    (re.compile(r'^test\s+\S+\s+\.\.\.\s+FAILED$'), "test_result"),
    (re.compile(r'^failures:$'), "diagnostic"),
    (re.compile(r"^thread\s+'[^']+'\s+\(.*\)\s+panicked"), "diagnostic"),
    (re.compile(r'^note:\s+'), "diagnostic"),
    (re.compile(r'^help:\s+'), "diagnostic"),
    (re.compile(r'^For more information about this error'), "summary"),
    (re.compile(r'^create\s+mode\s+\d+\s+(.+)'), "summary"),
]

GIT_PATTERNS = [
    (re.compile(r'^\?\?\s+(.+)'), "summary"),
    (re.compile(r'^\s*[AMRD]\s+(.+)'), "summary"),
    (re.compile(r'^\+\+\+ b/(.+)'), "summary"),
    (re.compile(r'^\s*(\d+)\s+files?\s+changed'), "summary"),
]

CURL_PATTERNS = [
    (re.compile(r'^([A-Z][a-zA-Z-]+):\s+(.+)'), "data"),
    (re.compile(r'^\s*[\{\[]'), "data"),
    (re.compile(r'^(HTTP/[0-9.]+)\s+(\d{3})\s*(.*)'), "diagnostic"),
]

def reclassify_log_events(events):
    """Apply new parser patterns to log events, converting matches to summary/diagnostic."""
    reclassified = 0
    for ev in events:
        if ev.get("type") != "log":
            continue
        msg = ev.get("message", "").strip()
        if not msg:
            continue
        for pattern, event_type in CARGO_PATTERNS + GIT_PATTERNS + CURL_PATTERNS:
            if pattern.match(msg):
                ev["type"] = event_type
                if "severity" not in ev or ev["severity"] == "info":
                    ev["severity"] = "info"
                reclassified += 1
                break
    return reclassified


# ---------------------------------------------------------------------------
# Process a single event file
# ---------------------------------------------------------------------------
def process_event_file(path: Path):
    """Read, transform, and return (before_count, after_count, noise_removed, ctx_merged, events)."""
    raw_lines = path.read_text(encoding="utf-8").splitlines()

    # Parse -- skip malformed JSON lines gracefully
    events = []
    parse_errors = 0
    for line in raw_lines:
        line = line.strip()
        if not line:
            continue
        try:
            events.append(json.loads(line))
        except json.JSONDecodeError:
            parse_errors += 1

    before_count = len(events)

    # Step 1: Noise filtering (only log-type events)
    noise_removed = 0
    filtered = []
    for ev in events:
        if ev.get("type") == "log" and is_noise_line(ev.get("message", "")):
            noise_removed += 1
        else:
            filtered.append(ev)

    # Step 2: Reclassify log events using new parser patterns
    reclassified = reclassify_log_events(filtered)

    # Step 3: Rustc context merging
    merger = RustcContextMerger()
    merged_events = []
    for ev in filtered:
        merged_events.extend(merger.feed(ev))
    merged_events.extend(merger.finish())
    ctx_merged = merger.merged_count

    after_count = len(merged_events)
    return before_count, after_count, noise_removed, reclassified, ctx_merged, parse_errors, merged_events


# ---------------------------------------------------------------------------
# Atomic write helper
# ---------------------------------------------------------------------------
def atomic_write_jsonl(path: Path, events):
    """Write events to path atomically via tmp file + rename."""
    fd, tmp_path = tempfile.mkstemp(
        dir=str(path.parent), suffix=".tmp", prefix=".migrate_"
    )
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            for ev in events:
                f.write(json.dumps(ev, ensure_ascii=False) + "\n")
        os.replace(tmp_path, str(path))
    except Exception:
        # Clean up tmp on failure
        try:
            os.unlink(tmp_path)
        except OSError:
            pass
        raise


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------
def main():
    dry_run = "--dry-run" in sys.argv

    store = Path.home() / ".local" / "share" / "arshy" / "arshy-store"
    events_dir = store / "events"
    tasks_path = store / "tasks.jsonl"

    if not events_dir.is_dir():
        print(f"ERROR: events directory not found: {events_dir}")
        sys.exit(1)

    event_files = sorted(events_dir.glob("*.jsonl"))
    if not event_files:
        print("No event files found. Nothing to migrate.")
        sys.exit(0)

    mode = "DRY RUN" if dry_run else "LIVE"
    print(f"=== arshy event migration ({mode}) ===")
    print(f"Store: {store}")
    print(f"Event files: {len(event_files)}")
    print()

    # ------------------------------------------------------------------
    # Aggregate before stats
    # ------------------------------------------------------------------
    total_before = 0
    total_after = 0
    total_noise = 0
    total_reclassified = 0
    total_ctx_merged = 0
    total_parse_errors = 0
    per_file = []  # (path, before, after, noise, reclassified, ctx_merged, parse_errors, events)

    for path in event_files:
        before, after, noise, reclassified, ctx, perr, events = process_event_file(path)
        total_before += before
        total_after += after
        total_noise += noise
        total_reclassified += reclassified
        total_ctx_merged += ctx
        total_parse_errors += perr
        per_file.append((path, before, after, noise, reclassified, ctx, perr, events))

    print("--- BEFORE ---")
    print(f"  Total events:      {total_before}")
    print(f"  Noise to remove:   {total_noise}")
    print(f"  Log to reclassify: {total_reclassified}")
    print(f"  Context lines to merge: {total_ctx_merged}")
    if total_parse_errors:
        print(f"  Parse errors (skipped): {total_parse_errors}")

    removal = total_before - total_after
    if total_before > 0:
        pct = removal / total_before * 100
        print(f"  Events to remove:  {removal} ({pct:.1f}%)")
    print()

    # ------------------------------------------------------------------
    # Show tasks.jsonl before
    # ------------------------------------------------------------------
    tasks_before = {}
    if tasks_path.is_file():
        for line in tasks_path.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if not line:
                continue
            try:
                t = json.loads(line)
                tid = t.get("task_id", "")
                if tid:
                    tasks_before[tid] = t
            except json.JSONDecodeError:
                pass

    tasks_with_zero = sum(
        1 for t in tasks_before.values()
        if t.get("metrics", {}).get("agent_visible_events", 0) == 0
        and t.get("events_count", 0) > 0
    )
    print(f"  Tasks with zero metrics despite having events: {tasks_with_zero}")
    print()

    # ------------------------------------------------------------------
    # Write processed event files
    # ------------------------------------------------------------------
    if not dry_run:
        written = 0
        for path, before, after, noise, reclassified, ctx, perr, events in per_file:
            if before == after and reclassified == 0:
                continue  # no change, skip write
            atomic_write_jsonl(path, events)
            written += 1
        print(f"Wrote {written} modified event files.")
    else:
        modified = sum(1 for _, b, a, noise, reclassified, *_ in per_file if b != a or reclassified > 0)
        print(f"[dry-run] Would modify {modified} event files.")

    # ------------------------------------------------------------------
    # Recompute and update tasks.jsonl
    # ------------------------------------------------------------------
    if tasks_path.is_file() and tasks_before:
        updated_tasks = 0
        for tid, task in tasks_before.items():
            ev_path = events_dir / f"{tid}.jsonl"
            if not ev_path.is_file():
                continue
            # Read the (possibly updated) events
            try:
                ev_lines = ev_path.read_text(encoding="utf-8").splitlines()
                ev_list = []
                for line in ev_lines:
                    line = line.strip()
                    if not line:
                        continue
                    try:
                        ev_list.append(json.loads(line))
                    except json.JSONDecodeError:
                        pass
            except FileNotFoundError:
                continue

            new_metrics = compute_metrics(ev_list)
            new_count = len(ev_list)

            old_metrics = task.get("metrics", {})
            old_count = task.get("events_count", 0)

            if new_metrics != old_metrics or new_count != old_count:
                task["events_count"] = new_count
                task["metrics"] = new_metrics
                # Recount error_count
                task["error_count"] = sum(
                    1 for ev in ev_list
                    if ev.get("severity") == "error"
                )
                updated_tasks += 1

        if not dry_run:
            atomic_write_tasks(tasks_path, list(tasks_before.values()))
            print(f"Updated {updated_tasks} tasks in tasks.jsonl.")
        else:
            print(f"[dry-run] Would update {updated_tasks} tasks in tasks.jsonl.")
    print()

    # ------------------------------------------------------------------
    # After summary
    # ------------------------------------------------------------------
    print("--- AFTER ---")
    print(f"  Total events:      {total_after}")
    print(f"  Noise removed:     {total_noise}")
    print(f"  Log reclassified:  {total_reclassified}")
    print(f"  Context lines merged: {total_ctx_merged}")
    if total_before > 0:
        after_pct = (total_before - total_noise) / total_before * 100
        print(f"  Noise removal:     {total_noise} events ({100 - after_pct:.1f}% reduction)")
        if total_reclassified > 0:
            print(f"  Reclassified:     {total_reclassified} log events → summary/diagnostic")
        if total_ctx_merged > 0:
            print(f"  Context merged:    {total_ctx_merged} lines absorbed into diagnostics")
    print()
    print("Migration complete." if not dry_run else "Dry run complete. Re-run without --dry-run to apply.")


def atomic_write_tasks(path: Path, tasks_list):
    """Write tasks.jsonl atomically."""
    fd, tmp_path = tempfile.mkstemp(
        dir=str(path.parent), suffix=".tmp", prefix=".migrate_tasks_"
    )
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            for t in tasks_list:
                f.write(json.dumps(t, ensure_ascii=False) + "\n")
        os.replace(tmp_path, str(path))
    except Exception:
        try:
            os.unlink(tmp_path)
        except OSError:
            pass
        raise


if __name__ == "__main__":
    main()
