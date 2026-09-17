#!/bin/bash
# Capture a versioned, reproducible metrics snapshot into docs/evidence/.
# Usage: scripts/evidence_snapshot.sh
#   ARSHY=/path/to/arshy scripts/evidence_snapshot.sh   # override binary
# Writes docs/evidence/<YYYY-MM-DD>.json (same-day runs overwrite), tagged
# with arshy version + git commit so the numbers can be traced to a code state.
set -euo pipefail

ARSHY="${ARSHY:-./target/debug/arshy}"
DATE="$(date +%Y-%m-%d)"
STAMP="$(date +%Y%m%d-%H%M%S)"
OUT_DIR="docs/evidence"
OUT="$OUT_DIR/$DATE.json"

mkdir -p "$OUT_DIR"

# Ensure the daemon is up so stats/analyze reflect the current store.
"$ARSHY" daemon start >/dev/null 2>&1 || true

GIT_COMMIT="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
GIT_DIRTY="$(git status --porcelain 2>/dev/null | wc -l | tr -d ' ')"
VERSION="$("$ARSHY" --version 2>/dev/null | head -1)"
STATS="$("$ARSHY" stats --format json 2>/dev/null || echo '{}')"
ANALYZE="$("$ARSHY" analyze 2>/dev/null || echo '{}')"
PLATFORM="$(uname -s)-$(uname -m)"

python3 - "$OUT" "$DATE" "$STAMP" "$VERSION" "$GIT_COMMIT" "$GIT_DIRTY" "$STATS" "$ANALYZE" "$PLATFORM" <<'PY'
import json, sys

out, date, stamp, version, commit, dirty, stats, analyze, platform = sys.argv[1:10]
stats_data = json.loads(stats)
snapshot = {
    "snapshot": {
        "date": date,
        "stamp": stamp,
        "arshy_version": version,
        "git_commit": commit,
        "git_dirty_files": dirty,
        "corpus": "local daemon store at snapshot time; not a controlled benchmark corpus",
        "platform": platform,
        "sample_size": stats_data.get("total_tasks", 0),
        "exclusions": [
            "token estimates",
            "typed-tool calls that never reach arshy",
            "repair-loop inference",
        ],
    },
    "stats": stats_data,
    "analyze": json.loads(analyze),
}
with open(out, "w", encoding="utf-8") as f:
    json.dump(snapshot, f, indent=2, ensure_ascii=False)
    f.write("\n")
print(f"wrote {out}")
PY
