#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"
rm -rf .git src
mkdir -p src
git init -q
git config user.email "bench@example.com"
git config user.name "Bench"
echo "base" > src/app.ts
git add src/app.ts
git commit -qm "base"
git checkout -qb feature
echo "feature" > src/app.ts
git commit -am "feature" -q
git checkout -q main 2>/dev/null || git checkout -q master
echo "local dirty" > src/app.ts
# leave uncommitted change; merge will refuse
