#!/usr/bin/env bash
# Run every test suite in the repo and print one summary.
#
#   bash scripts/check.sh            # everything
#   bash scripts/check.sh web agent  # only the named suites
#
# Suites: lint, matchmaker, gateway, scripts, web, agent, integration
#
# Matchmaker and integration tests need Postgres. Start it with
#   (cd matchmaker && docker compose up -d)
# or point TEST_DATABASE_URL at any empty Postgres database. When the
# database cannot be reached the suite is reported as SKIPPED, not passed.

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ALL=(lint matchmaker gateway scripts web agent integration)
if [ "$#" -gt 0 ]; then SUITES=("$@"); else SUITES=("${ALL[@]}"); fi

RESULTS=()
FAILED=0

record() { RESULTS+=("$(printf '%-11s %s' "$1" "$2")"); }

run() { # run <suite> <command...>
  local name="$1"
  shift
  echo
  echo "=== $name: $* ==="
  if "$@"; then
    record "$name" "PASS"
  else
    record "$name" "FAIL"
    FAILED=1
  fi
}

skip() {
  echo
  echo "=== $1: skipped ($2) ==="
  record "$1" "SKIPPED  ($2)"
}

py_for() { # prefer the component's own virtualenv
  if [ -x "$ROOT/$1/.venv/bin/python" ]; then echo "$ROOT/$1/.venv/bin/python"; else command -v python3; fi
}

suite_lint() {
  if command -v ruff >/dev/null 2>&1; then
    run lint ruff check matchmaker gateway guest-images host-agent/scripts
  else
    skip lint "ruff is not installed"
  fi
}

postgres_reachable() { # can the Matchmaker's Python reach TEST_DATABASE_URL?
  local py
  py="$(py_for matchmaker)"
  (cd matchmaker && "$py" - <<'PY'
import os, sys
from pathlib import Path
try:
    from dotenv import load_dotenv
    from sqlalchemy import create_engine
    load_dotenv(Path(".env"))
    url = os.environ.get("TEST_DATABASE_URL")
    if not url:
        sys.exit(1)
    create_engine(url).connect().close()
except Exception:
    sys.exit(1)
PY
  ) >/dev/null 2>&1
}

suite_matchmaker() {
  if ! postgres_reachable; then
    skip matchmaker "no Postgres at TEST_DATABASE_URL; see the top of scripts/check.sh"
    return
  fi
  run matchmaker bash -c "cd matchmaker && '$(py_for matchmaker)' -m pytest -q"
}

# Real Matchmaker + gateway processes over HTTP; see scripts/integration/run.py.
suite_integration() {
  if ! postgres_reachable; then
    skip integration "no Postgres at TEST_DATABASE_URL; see the top of scripts/check.sh"
    return
  fi
  run integration "$(py_for matchmaker)" scripts/integration/run.py
}

suite_gateway() {
  run gateway bash -c "cd gateway && '$(py_for gateway)' -m pytest -q"
}

suite_scripts() {
  run scripts python3 -m pytest -q host-agent/scripts guest-images/tests
}

suite_web() {
  run web bash -c "cd web && npm run --silent typecheck && npm test --silent"
}

suite_agent() {
  run agent bash -c "cd host-agent && npm run --silent typecheck && cd src-tauri && cargo test --lib --quiet"
}

for suite in "${SUITES[@]}"; do
  case "$suite" in
    lint | matchmaker | gateway | scripts | web | agent | integration) "suite_$suite" ;;
    *)
      echo "unknown suite: $suite (choose from: ${ALL[*]})" >&2
      exit 2
      ;;
  esac
done

echo
echo "=== summary ==="
printf '%s\n' "${RESULTS[@]}"
exit "$FAILED"
