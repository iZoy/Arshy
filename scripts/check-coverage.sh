#!/usr/bin/env bash
# Enforce line coverage from an lcov.info file.
set -euo pipefail
LCOV="${1:-lcov.info}"
[[ -f "$LCOV" ]] || { echo "coverage file not found: $LCOV" >&2; exit 1; }

python3 - "$LCOV" <<'PY'
import sys
from pathlib import PurePosixPath
path = sys.argv[1]
critical = tuple(
    tuple(part.strip("/").split("/"))
    for part in ("src/daemon/parser/", "src/daemon/exec/", "src/daemon/ipc_handler/",
                 "src/daemon/security/", "src/proxy/")
)
totals = {"all": [0, 0], "critical": [0, 0]}
current = ""
for line in open(path, encoding="utf-8"):
    line = line.strip()
    if line.startswith("SF:"):
        current = line[3:]
    elif line.startswith("DA:"):
        _, data = line.split(":", 1)
        _, hits, *_ = data.split(",")
        hit = int(hits) > 0
        totals["all"][0] += 1
        totals["all"][1] += hit
        source = PurePosixPath(current.replace("\\", "/")).parts
        if any(source[i:i + len(prefix)] == prefix
               for prefix in critical for i in range(len(source) - len(prefix) + 1)):
            totals["critical"][0] += 1
            totals["critical"][1] += hit

def pct(pair):
    return 100.0 * pair[1] / pair[0] if pair[0] else 0.0

for name, pair in totals.items():
    value = pct(pair)
    # Keep the published CI gate explicit and consistent across both scopes.
    threshold = 75.0
    print(f"{name} line coverage: {value:.2f}% (required {threshold:.0f}%)")
    if value < threshold:
        raise SystemExit(f"coverage threshold failed for {name}")
PY
