#!/usr/bin/env bash
# Measure component-only Arshy quality indicators across ecosystems.
# This opt-in analytics workload never changes MCP execution responses and
# deliberately does not estimate tokens.
set -u

ARSHY="${ARSHY:-./target/release/arshy}"
JSON="${JSON:-0}"
if [[ ! -x "$ARSHY" ]]; then
    [[ -x ./target/debug/arshy ]] && ARSHY=./target/debug/arshy || {
        echo "error: arshy binary not found; build it first" >&2; exit 1;
    }
fi
"$ARSHY" daemon start >/dev/null 2>&1 || true
WORK="$(mktemp -d /tmp/arshy_measure.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

cat > "$WORK/probe.py" <<'PY'
raise ValueError("arshy efficiency probe")
PY
cat > "$WORK/probe.rs" <<'RS'
fn main() { let x: i32 = "wrong"; println!("{}", x); }
RS
cat > "$WORK/probe.js" <<'JS'
throw new Error("arshy efficiency probe");
JS
WORKLOADS="$WORK/workloads"
: > "$WORKLOADS"
command -v python3 >/dev/null 2>&1 && printf 'python\tpython3 %s/probe.py\n' "$WORK" >> "$WORKLOADS"
command -v rustc >/dev/null 2>&1 && printf 'rustc\trustc %s/probe.rs -o %s/probe-bin\n' "$WORK" "$WORK" >> "$WORKLOADS"
command -v node >/dev/null 2>&1 && printf 'node\tnode %s/probe.js\n' "$WORK" >> "$WORKLOADS"
if command -v go >/dev/null 2>&1; then
    cat > "$WORK/probe.go" <<'GO'
package main
func main() { undefined() }
GO
    printf 'go\tgo build %s/probe.go\n' "$WORK" >> "$WORKLOADS"
fi
[[ -s "$WORKLOADS" ]] || { echo "error: no supported ecosystem tool found" >&2; exit 1; }

if [[ "$JSON" != "1" ]]; then
    printf '%-10s | %-8s | %-7s | %-8s | %-8s\n' "ECOSYSTEM" "STATUS" "EVENTS" "RAW(B)" "ERRORS"
    printf -- '-----------+----------+---------+----------+--------\n'
fi
ROWS="[]"
while IFS=$'\t' read -r ecosystem command; do
    output="$WORK/${ecosystem}.json"
    "$ARSHY" run "$command" --cwd "$WORK" --format json > "$output" 2>/dev/null || true
    row="$(python3 - "$ecosystem" "$output" <<'PY'
import json, sys
eco, path = sys.argv[1:]
try: d = json.load(open(path))
except Exception: d = {}
print(json.dumps({"ecosystem": eco, "status": d.get("status", "unknown"),
                  "events": d.get("event_count", 0),
                  "raw_output_bytes": d.get("raw_output_bytes", 0) or 0,
                  "errors": d.get("error_count", 0)}, separators=(",", ":")))
PY
)"
    ROWS="$(python3 - "$ROWS" "$row" <<'PY'
import json, sys
items = json.loads(sys.argv[1]); items.append(json.loads(sys.argv[2])); print(json.dumps(items))
PY
)"
    if [[ "$JSON" != "1" ]]; then
        python3 - "$ecosystem" "$output" <<'PY'
import json, sys
eco, path = sys.argv[1:]
try: d = json.load(open(path))
except Exception: d = {}
print("{:10} | {:8} | {:7} | {:8} | {:8}".format(eco, d.get("status", "unknown"),
    d.get("event_count", 0), d.get("raw_output_bytes", 0) or 0, d.get("error_count", 0)))
PY
    fi
done < "$WORKLOADS"

stats="$WORK/stats.json"
"$ARSHY" stats --format json > "$stats" 2>/dev/null || echo '{}' > "$stats"
python3 - "$JSON" "$ROWS" "$stats" <<'PY'
import json, sys
as_json, rows_json, stats_path = sys.argv[1:]
rows = json.loads(rows_json)
try: stats = json.load(open(stats_path))
except Exception: stats = {}
eff = stats.get("efficiency") or {}
report = {"schema_version": eff.get("schema_version", "quality-v1"),
          "components": eff.get("components", {}), "counters": eff.get("counters", {})}
if as_json == "1":
    print(json.dumps({"per_ecosystem": rows, "efficiency": report}, indent=2))
else:
    print("\n=== Arshy Quality Components ===")
    print("schema: {}".format(report["schema_version"]))
    for key, value in report["components"].items():
        print("{}: {}".format(key, "n/a" if value is None else "{:.1f}%".format(value)))
    print("\nUse JSON=1 for a machine-readable report.")
PY
