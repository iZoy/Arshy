#!/usr/bin/env bash
# Measure arshy's token-savings metric across mainstream ecosystems.
#
# Why this script exists:
#   1. The "estimated_token_savings_pct" metric mixes measured values with a
#      10% fallback heuristic (see docs/reference/metrics.md for the contract).
#   2. Dogfooding data is biased toward Rust tooling (cargo = ~90% of calls).
#
# This script runs a battery of representative commands across Python /
# Node / Go / Rust / npm and prints a table you can inspect (and copy into
# marketing material) with **honest** savings_basis labels.
#
# Usage:
#   scripts/measure-savings.sh                     # uses ./target/release/arshy
#   ARSHY=./target/debug/arshy scripts/measure-savings.sh
#   JSON=1 scripts/measure-savings.sh             # machine-readable output
#
# Tools that aren't installed are skipped (NOT counted as failures).

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
# Wait for the daemon socket to become connectable. `daemon start` returns
# immediately after fork, but the release binary needs ~1s to bind UDS;
# a fast follow-up `run` otherwise produces empty JSON.
SOCK="${HOME}/.local/share/arshy/arshyd.sock"
for i in 1 2 3 4 5 6 7 8 9 10; do
    if [[ -S "$SOCK" ]] && python3 -c "import socket,sys; s=socket.socket(socket.AF_UNIX); s.settimeout(0.5); s.connect(sys.argv[1]); s.close()" "$SOCK" 2>/dev/null; then
        break
    fi
    sleep 0.3
done

WORK="$(mktemp -d /tmp/arshy_measure.XXXXXX)"
trap 'rm -rf "$WORK"' EXIT

# Helper script: read JSON from a file and emit key=value lines.
# Using a file avoids shell-quoting pitfalls with embedded newlines.
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
    # 6+ words forces the structured (long) path.
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

# ── Print header ───────────────────────────────────────────────────────────
if [[ "$JSON" != "1" ]]; then
    printf "%-10s | %-10s | %-10s | %-8s | %-6s | %s\n" \
        "ECOSYSTEM" "STATUS" "PARSER" "EVENTS" "SHORT" "ERRORS"
    printf -- "-----------+------------+------------+----------+--------+--------\n"
fi

RESULTS_JSON="["
first=1

# ── Run each workload ──────────────────────────────────────────────────────
while IFS= read -r line; do
    [[ -z "$line" ]] && continue
    # awk -F splits on multi-char delimiter (bash IFS only supports single char).
    eco=$(awk -F'::' '{print $1}' <<<"$line")
    parser=$(awk -F'::' '{print $2}' <<<"$line")
    cmd=$(awk -F'::' '{print $3}' <<<"$line")

    OUT="$WORK/run_${eco}.json"
    # Note: do NOT `|| echo "{}"` here — failing commands are expected
    # to produce structured error events, and their non-zero exit code would
    # trigger the `||` and wipe the successful JSON output.
    "$ARSHY" run "$cmd" --format json >"$OUT" 2>/dev/null
    [[ -s "$OUT" ]] || echo '{}' > "$OUT"

    task_status="n/a"; task_events=0; task_short="n/a"; task_err=0; task_parser="n/a"
    while IFS='=' read -r key val; do
        [[ -z "$key" ]] && continue
        case "$key" in
            status)  task_status="$val" ;;
            events)  task_events="$val" ;;
            short)   task_short="$val" ;;
            errors)  task_err="$val" ;;
            parser)  task_parser="$val" ;;
        esac
    done < <(python3 "$HELPER" run "$OUT" 2>/dev/null)

    if [[ "$JSON" == "1" ]]; then
        if [[ $first -eq 0 ]]; then RESULTS_JSON+=","; fi
        first=0
        RESULTS_JSON+="$(printf '{"ecosystem":"%s","status":"%s","expected_parser":"%s","parser":"%s","event_count":%s,"short_command":"%s","error_count":%s}' \
            "$eco" "$task_status" "$parser" "$task_parser" "$task_events" "$task_short" "$task_err")"
    else
        printf "%-10s | %-10s | %-10s | %-8s | %-6s | %s\n" \
            "$eco" "$task_status" "$task_parser" "$task_events" "$task_short" "$task_err"
    fi
done < "$WORKLOADS_FILE"

RESULTS_JSON+="]"

# ── Aggregate stats ────────────────────────────────────────────────────────
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
    echo "  \"workloads\": $RESULTS_JSON,"
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
echo "═══ Aggregate stats ═══"
printf "Total raw output bytes:           %s\n" "$agg_raw"
printf "Total agent-delivered bytes:      %s\n" "$agg_deliv"
printf "Estimated token savings:          %s%%\n" "$agg_savings_pct"
printf "Noise (skipped events):           %s%%\n" "$agg_noise_pct"
printf "Savings basis:                    %s\n" "$agg_basis"
printf "Fallback task count:              %s\n" "$agg_fallback"

echo
echo "─── Honest reading ───"
if [[ "$agg_basis" == "measured" ]]; then
    echo "✓ Every task had an exact agent_delivered_bytes recorded."
    echo "  The ${agg_savings_pct}% figure is safe to cite in marketing."
elif [[ "$agg_basis" == "estimated" ]]; then
    echo "⚠ $agg_fallback task(s) used the 10% fallback heuristic."
    echo "  The ${agg_savings_pct}% figure is an upper-bound estimate, not a measurement."
    echo "  To get a measured basis: run only long commands that produce"
    echo "  structured events, then check savings_basis == measured."
else
    echo "— No raw output yet (daemon just started?)."
fi

echo
echo "─── Re-running ───"
echo "  JSON output:    JSON=1 $0"
echo "  Custom binary:  ARSHY=/path/to/arshy $0"
echo "  Full snapshot:  $ARSHY analyze --format json | jq ."
