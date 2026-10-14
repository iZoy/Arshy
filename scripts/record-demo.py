#!/usr/bin/env python3
"""Record the native-vs-Arshy demo as an asciicast v2 file.

Every command really runs; only the typing is simulated. Render to GIF with
`agg docs/assets/arshy-demo.cast docs/assets/arshy-demo.gif`.

Usage: scripts/record-demo.py [OUTPUT.cast]   (arshy must be on PATH and the
daemon running)
"""
import json, os, shutil, subprocess, sys, tempfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
OUT = sys.argv[1] if len(sys.argv) > 1 else os.path.join(REPO, "docs/assets/arshy-demo.cast")
W, H = 104, 26
PROMPT = "\x1b[1;32m$\x1b[0m "

work = os.path.join(tempfile.gettempdir(), "demo-rust-failure")
shutil.rmtree(work, ignore_errors=True)
shutil.copytree(os.path.join(REPO, "docs/tutorials/demo-rust-failure"), work)
subprocess.run(["cargo", "fetch", "-q"], cwd=work)

events, t = [], 0.0
def emit(text, dt=0.0):
    global t
    t += dt
    events.append([round(t, 3), "o", text.replace("\n", "\r\n")])

def comment(text):
    emit(f"\x1b[2m# {text}\x1b[0m\n", 0.6)

def run(cmd, pause=2.2):
    emit(PROMPT, 0.4)
    for ch in cmd:
        emit(ch, 0.035)
    emit("\n", 0.3)
    out = subprocess.run(["bash", "-c", cmd], cwd=work, capture_output=True, text=True,
                         env={**os.environ, "CARGO_TERM_COLOR": "never"})
    text = out.stdout + out.stderr
    if text and not text.endswith("\n"):
        text += "\n"
    emit(text, 0.2)
    t_add = pause
    emit("", t_add)

emit("\x1b[2J\x1b[H")
comment("Native: the agent reads the full compiler output")
run("cargo check")
emit("\x1b[2J\x1b[H", 0.2)
comment("Arshy: the same command, structured for the agent")
run("arshy run 'cargo check' --format pretty")
run("arshy run 'cargo check' --format json | jq -c '{status, exit_code, error_count}, (.primary_diagnostic | {code, message, location})'", 3.5)
emit("\n", 0.2)
comment("Fix only the reported location, then verify")
run("sed -i 's/^    value$/    value.to_string()/' src/main.rs")
run("arshy run 'cargo check' --format pretty", 3.0)

with open(OUT, "w") as f:
    f.write(json.dumps({"version": 2, "width": W, "height": H,
                        "title": "Arshy: native output vs structured diagnostics"}) + "\n")
    for e in events:
        f.write(json.dumps(e) + "\n")
print(OUT)
