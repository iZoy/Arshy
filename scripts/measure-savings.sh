#!/usr/bin/env bash
# Measure arshy's token-savings metric across mainstream ecosystems.
#
# Why this script exists:
#   1. The "estimated_token_savings_pct" metric mixes measured values with a
#      10% fallback heuristic (see docs/reference/metrics.md for the contract).
#   2. Dogfooding data is biased toward Rust tooling (cargo = ~90% of calls).
#
# So this script runs a battery of representative commands across Python /
# Node / Go / Rust / npm and prints the headline as PER-ECOSYSTEM savings
# (one row per ecosystem with its own savings %). Aggregate is secondary.
#
# Usage:
#   scripts/measure-savings.sh                     # uses ./target/release/arshy
#   ARSHY=./target/debug/arshy scripts/measure-savings.sh
#   JSON=1 scripts/measure-savings.sh             # machine-readable output
#
# Tools that aren't installed are skipped with a note (NOT counted as failures).

set -u

ARSHY="${ARSHY:-./target/release/arshy}"
JSON="${JSON:-0}"

if [[ ! -x "$ARSHY" ]]; then
    if [[ -x ./target/debug/arshy ]]; then
        ARSHY=./target/debug/arshy
    else
        echo "error: arshy binary not found. Build first: cargo build --release" >&2
        exit 1
    fi
fi

# Ensure the daemon is up so we get stats from a stable store.
"$ARSHY" daemon start >/dev/null 2>&1 || true

WORK="$(mktemp -d /tmp/arshy_measure.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

# Wait for daemon socket to be ready (see scripts/restart.sh).
SOCK="${HOME}/.local/share/arshy/arshyd.sock"
for i in 1 2 3 4 5 6 7 8 9 10; do
    if [[ -S "$SOCK" ]] && python3 -c "import socket,sys; s=socket.socket(socket.AF_UNIX); s.settimeout(0.5); s.connect(sys.argv[1]); s.close()" "$SOCK" 2>/dev/null; then
        break
    fi
    sleep 0.3
done

# Helper script: read JSON from a file and emit key=value lines.
HELPER="$WORK/extract.py"
cat > "$HELPER" <<'PYEOF'
import json, sys
mode = sys.argv[1]
with open(sys.argv[2]) as f:
    d = json.load(f)
if mode == "run":
    parts = [
        ("status", d.get("status", "n/a")),
        ("events", d.get("event_count", 0)),
        ("short", "yes" if d.get("short_command") else "no"),
        ("errors", d.get("error_count", 0)),
        ("parser", d.get("parser_name") or "n/a"),
        ("raw_bytes", d.get("raw_output_bytes") or 0),
        ("delivered_bytes", d.get("agent_delivered_bytes") or 0),
    ]
elif mode == "stats":
    raw = d.get("total_raw_output_bytes") or 0
    deliv = d.get("total_agent_delivered_bytes") or 0
    visible = d.get("total_agent_visible_events") or 0
    skipped = d.get("total_agent_skipped_events") or 0
    basis = d.get("savings_basis") or "none"
    fallback = d.get("savings_fallback_task_count") or 0
    savings = ((raw - deliv) / raw * 100) if raw > 0 else 0.0
    total_events = visible + skipped
    noise = (skipped / total_events * 100) if total_events > 0 else 0.0
    parts = [
        ("raw", raw), ("deliv", deliv),
        ("visible", visible), ("skipped", skipped),
        ("basis", basis), ("fallback", fallback),
        ("savings", f"{savings:.1f}"), ("noise", f"{noise:.1f}"),
    ]
else:
    sys.exit(2)
for k, v in parts:
    print(f"{k}={v}")
PYEOF

# Workload specs as <ecosystem>::<parser>::<command> lines.
WORKLOADS_FILE="$WORK/workloads.txt"
add_workload() {
    printf '%s::%s::%s\n' "$1" "$2" "$3" >> "$WORKLOADS_FILE"
}

# ── Python ─────────────────────────────────────────────────────────────────
if command -v python3 >/dev/null 2>&1; then
    cat > "$WORK/probe.py" <<'PY'
def main():
    raise ValueError("measure-savings-probe: invalid value")
main()
PY
    add_workload "python" "python" \
        "python3 $WORK/probe.py arg1 arg2 arg3 arg4 arg5"
fi

# ── Go ─────────────────────────────────────────────────────────────────────
if command -v go >/dev/null 2>&1; then
    cat > "$WORK/main.go" <<'GO'
package main
import "fmt"
func main() { fmt.PrintfX("x"); undefined_function() }
GO
    add_workload "go" "go" "go build $WORK/main.go"
fi

# ── Rust ───────────────────────────────────────────────────────────────────
if command -v rustc >/dev/null 2>&1; then
    cat > "$WORK/probe.rs" <<'RS'
fn main() {
    let x: i32 = "string";
    println!("{}", x);
}
RS
    add_workload "rustc" "cargo" "rustc $WORK/probe.rs -o $WORK/probe-bin"
fi

# ── Node.js ────────────────────────────────────────────────────────────────
if command -v node >/dev/null 2>&1; then
    cat > "$WORK/break.js" <<'JS'
function broken() { throw new Error("measure-savings-probe"); }
broken();
JS
    add_workload "node" "node" \
        "node $WORK/break.js arg1 arg2 arg3 arg4 arg5"
fi

# ── npm ────────────────────────────────────────────────────────────────────
if command -v npm >/dev/null 2>&1; then
    mkdir -p "$WORK/bad_npm"
    cat > "$WORK/bad_npm/package.json" <<'JSON'
{"name":"bad","dependencies":{"left-pad":"1.0.0","foo":"999.999.999"}}
JSON
    add_workload "npm" "npm" \
        "npm install --prefix $WORK/bad_npm --no-audit --offline --no-fund"
fi

# ── Cargo (control) ────────────────────────────────────────────────────────
if command -v cargo >/dev/null 2>&1; then
    add_workload "cargo" "cargo" \
        "sh -c 'echo error: measure-savings-probe; echo undefined_symbol >&2; exit 1'"
fi

# ── Run each workload ──────────────────────────────────────────────────────
# Collect per-ecosystem metrics into a CSV-like structure.
PER_ECO_CSV="$WORK/per_eco.csv"
echo "ecosystem,parser,status,events,errors,raw,delivered,savings_pct" > "$PER_ECO_CSV"

# Pretty header (printed before any rows so consumers know the format)
if [[ "$JSON" != "1" ]]; then
    printf "%-10s | %-10s | %-7s | %-6s | %-7s | %-9s | %-9s | %s\n" \
        "ECOSYSTEM" "PARSER" "STATUS" "EVENTS" "ERRORS" "RAW(B)" "DELIV(B)" "SAVINGS"
    printf -- "-----------+------------+---------+--------+---------+-----------+-----------+----------\n"
fi

JSON_ROWS="["
first_row=1

while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    eco=$(awk -F'::' '{print $1}' <<<"$line")
    parser=$(awk -F'::' '{print $2}' <<<"$line")
    cmd=$(awk -F'::' '{print $3}' <<<"$line")

    OUT="$WORK/run_${eco}.json"
    # --cwd $WORK pins cwd to a clean non-git dir so the metric stays
    # comparable across invocations (Phase B Q-3 fix).
    "$ARSHY" run "$cmd" --cwd "$WORK" --format json >"$OUT" 2>/dev/null
    [[ -s "$OUT" ]] || echo '{}' > "$OUT"

    task_status="n/a"; task_events=0; task_short="n/a"; task_err=0
    task_parser="n/a"; task_raw=0; task_deliv=0
    while IFS='=' read -r key val; do
        [[ -z "$key" ]] && continue
        case "$key" in
            status)         task_status="$val" ;;
            events)         task_events="$val" ;;
            short)          task_short="$val" ;;
            errors)         task_err="$val" ;;
            parser)         task_parser="$val" ;;
            raw_bytes)      task_raw="$val" ;;
            delivered_bytes) task_deliv="$val" ;;
        esac
    done < <(python3 "$HELPER" run "$OUT" 2>/dev/null)

    # Per-ecosystem savings from this task's own raw/delivered bytes.
    # This is the honest number — not contaminated by aggregate fallback.
    savings="n/a"
    if [[ "$task_raw" -gt 0 ]]; then
        savings=$(python3 -c "print(f'{($task_raw - $task_deliv) / $task_raw * 100:.1f}')")
    fi

    echo "$eco,$parser,$task_status,$task_events,$task_err,$task_raw,$task_deliv,$savings" \
        >> "$PER_ECO_CSV"

    if [[ "$JSON" == "1" ]]; then
        if [[ $first_row -eq 0 ]]; then JSON_ROWS+=","; fi
        first_row=0
        JSON_ROWS+="$(printf '{"ecosystem":"%s","parser":"%s","status":"%s","event_count":%s,"error_count":%s,"raw_bytes":%s,"delivered_bytes":%s,"savings_pct":%s}' \
            "$eco" "$task_parser" "$task_status" "$task_events" "$task_err" "$task_raw" "$task_deliv" "$savings")"
    else
        printf "%-10s | %-10s | %-7s | %-6s | %-7s | %-9s | %-9s | %s%%\n" \
            "$eco" "$task_parser" "$task_status" "$task_events" "$task_err" "$task_raw" "$task_deliv" "$savings"
    fi
done < "$WORKLOADS_FILE"

JSON_ROWS+="]"

# ── Per-ecosystem aggregate (sum across all tasks of same ecosystem, since
#    measure-savings runs each workload exactly once we have 1 row per eco).
#    For multi-run scripts, change this to GROUP BY ecosystem.
# ── Aggregate stats from daemon (the fallback-tainted aggregate, for context) ─
STATS="$WORK/stats.json"
"$ARSHY" stats --format json >"$STATS" 2>/dev/null || echo '{}' > "$STATS"

agg_raw=0; agg_deliv=0; agg_visible=0; agg_skipped=0
agg_basis="none"; agg_fallback=0; agg_savings_pct="0.0"; agg_noise_pct="0.0"
while IFS='=' read -r key val; do
    [[ -z "$key" ]] && continue
    case "$key" in
        raw)      agg_raw="$val" ;;
        deliv)    agg_deliv="$val" ;;
        visible)  agg_visible="$val" ;;
        skipped)  agg_skipped="$val" ;;
        basis)    agg_basis="$val" ;;
        fallback) agg_fallback="$val" ;;
        savings)  agg_savings_pct="$val" ;;
        noise)    agg_noise_pct="$val" ;;
    esac
done < <(python3 "$HELPER" stats "$STATS" 2>/dev/null)

if [[ "$JSON" == "1" ]]; then
    echo "{"
    echo "  \"per_ecosystem\": $JSON_ROWS,"
    echo "  \"aggregate\": {"
    printf '    "total_raw_output_bytes": %s,\n' "$agg_raw"
    printf '    "total_agent_delivered_bytes": %s,\n' "$agg_deliv"
    printf '    "estimated_token_savings_pct": %s,\n' "$agg_savings_pct"
    printf '    "noise_pct": %s,\n' "$agg_noise_pct"
    printf '    "agent_visible_events": %s,\n' "$agg_visible"
    printf '    "agent_skipped_events": %s,\n' "$agg_skipped"
    printf '    "savings_basis": "%s",\n' "$agg_basis"
    printf '    "savings_fallback_task_count": %s\n' "$agg_fallback"
    echo "  }"
    echo "}"
    exit 0
fi

echo
echo "═══ Aggregate (secondary, from daemon stats) ═══"
printf "Total raw:                  %s bytes\n" "$agg_raw"
printf "Total agent-delivered:     %s bytes\n" "$agg_deliv"
printf "Aggregate savings:         %s%%\n" "$agg_savings_pct"
printf "Savings basis:             %s   (fallback tasks: %s)\n" "$agg_basis" "$agg_fallback"

# Honest reading — what's safe to cite
echo
echo "─── Honest reading ───"
case "$agg_basis" in
    measured)
        if [[ "$agg_fallback" == "0" ]]; then
            echo "✓ Every task had an exact agent_delivered_bytes recorded."
            echo "  Per-ecosystem table above is the authoritative headline."
            echo "  Aggregate ${agg_savings_pct}% is the cross-ecosystem mean."
        else
            echo "⚠ $agg_fallback task(s) used the 10% fallback heuristic."
            echo "  Aggregate ${agg_savings_pct}% is approximate; prefer per-ecosystem rows."
        fi
        ;;
    estimated)
        echo "⚠ Aggregate used the 10% fallback for some tasks."
        echo "  Prefer per-ecosystem rows (each is measured from the task itself)."
        ;;
    *)
        echo "— No raw output yet (daemon just started?)."
        ;;
esac

echo
echo "─── Re-running ───"
echo "  JSON output:    JSON=1 $0"
echo "  Custom binary:  ARSHY=/path/to/arshy $0"
echo "  Full snapshot:  $ARSHY analyze --format json | jq ."
