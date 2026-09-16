#!/usr/bin/env bash
# Quick demo / smoke test for `dap-cli debug repl`.
#
# Usage:
#   ./scripts/test-debug-repl.sh              # automated NDJSON demo (default, fake adapter)
#   ./scripts/test-debug-repl.sh auto         # same as default
#   ./scripts/test-debug-repl.sh plain        # interactive gdb-style REPL (fake adapter)
#   ./scripts/test-debug-repl.sh debugpy      # interactive REPL with real debugpy
#   ./scripts/test-debug-repl.sh ndjson       # pipe scripts/repl-demo.ndjson
#   ./scripts/test-debug-repl.sh ndjson path/to/commands.ndjson
#
# Fake adapter: no Python deps. debugpy mode needs:
#   python3 -m pip install -r scripts/fixtures/requirements.txt
# Optional: PYTHON=/path/to/venv/bin/python ./scripts/test-debug-repl.sh debugpy

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DAP_CLI_ROOT="${ROOT}/../dap-cli"
FIXTURE_DIR="${ROOT}/scripts/fixtures"
DEMO_PROGRAM="${FIXTURE_DIR}/main.py"
COMMANDS_FILE="${ROOT}/scripts/repl-demo.ndjson"
MODE="${1:-auto}"
PYTHON_BIN="${PYTHON:-python3}"

log() {
  printf '\033[1;34m==>\033[0m %s\n' "$*"
}

die() {
  printf '\033[1;31merror:\033[0m %s\n' "$*" >&2
  exit 1
}

print_repl_tips() {
  cat <<'EOF'
Controls:
  Up/Down  Previous / next command in history
  Ctrl+C   Cancel startup or exit the REPL (preferred)
  q        Quit once the dap> prompt is visible
  Ctrl+Z   Pauses the debugger — looks frozen; use Ctrl+C instead

If you see "suspended (tty input)" in zsh:
  fg       Resume (then wait for dap>, or Ctrl+C and restart)
  # or in another terminal:
  pkill -f 'dap-cli debug repl'
  pkill -f 'debugpy.adapter'

If the terminal stops responding (debugpy on Python 3.14 can take ~30s on first launch):
  Open another terminal and run:
    pkill -f 'dap-cli debug repl'
    pkill -f 'debugpy.adapter'
EOF
}

warn_stale_sessions() {
  local count
  count="$(pgrep -fc 'dap-cli debug repl|debugpy\.adapter' 2>/dev/null || echo 0)"
  if [[ "$count" -gt 0 ]]; then
    log "Warning: ${count} stale debug session process(es) found."
    log "Run: $0 cleanup   (or pkill -f 'dap-cli debug repl')"
  fi
}

run_cleanup() {
  local before after
  before="$(pgrep -fc 'dap-cli debug repl|debugpy\.adapter' 2>/dev/null || echo 0)"
  pkill -f 'dap-cli debug repl' 2>/dev/null || true
  pkill -f 'debugpy.adapter' 2>/dev/null || true
  sleep 0.5
  after="$(pgrep -fc 'dap-cli debug repl|debugpy\.adapter' 2>/dev/null || echo 0)"
  log "Cleaned up debug sessions (${before} -> ${after} processes remaining)."
}

ensure_built() {
  log "Building dap-cli and fake-dap-adapter..."
  (cd "$ROOT" && env -u CARGO_TARGET_DIR cargo build -q -p fake-dap-adapter)
  (cd "$DAP_CLI_ROOT" && env -u CARGO_TARGET_DIR cargo build -q)
}

ensure_built_cli() {
  log "Building dap-cli..."
  (cd "$DAP_CLI_ROOT" && env -u CARGO_TARGET_DIR cargo build -q)
}

core_target_dir() {
  if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
    echo "${CARGO_TARGET_DIR}"
  else
    echo "${ROOT}/target"
  fi
}

cli_target_dir() {
  if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
    echo "${CARGO_TARGET_DIR}"
  else
    echo "${DAP_CLI_ROOT}/target"
  fi
}

resolve_bin() {
  local name="$1"
  if [[ "$name" == "dap-cli" ]]; then
    echo "$(cli_target_dir)/debug/${name}"
  else
    echo "$(core_target_dir)/debug/${name}"
  fi
}

run_dap_cli() {
  PATH="$(core_target_dir)/debug:$(cli_target_dir)/debug:${PATH}" RUST_LOG=off "$(cli_target_dir)/debug/dap-cli" "$@"
}

require_debugpy() {
  if ! "$PYTHON_BIN" -c "import debugpy" 2>/dev/null; then
    die "$(cat <<EOF
debugpy is not installed for ${PYTHON_BIN}.

Install it with:
  ${PYTHON_BIN} -m pip install -r ${FIXTURE_DIR}/requirements.txt

Or point PYTHON at a venv that already has debugpy:
  PYTHON=/path/to/venv/bin/python $0 debugpy
EOF
)"
  fi

  local py_version
  py_version="$("$PYTHON_BIN" -c 'import sys; print(f"{sys.version_info.major}.{sys.version_info.minor}")')"
  if [[ "$py_version" == "3.14" ]]; then
    log "Note: Python ${py_version} + debugpy can be slow to start; first launch may take up to a minute."
  fi
}

run_auto() {
  local input="${1:-$COMMANDS_FILE}"
  [[ -f "$input" ]] || die "commands file not found: $input"

  ensure_built

  log "Running NDJSON demo from $input"
  log "Program: fake adapter / ${DEMO_PROGRAM}"
  echo

  while IFS= read -r line || [[ -n "$line" ]]; do
    [[ -z "${line// }" ]] && continue
    printf '\033[1;33m>>\033[0m %s\n' "$line"
  done < "$input"
  echo

  log "Responses:"
  echo

  run_dap_cli debug repl --ndjson --adapter fake "$DEMO_PROGRAM" < "$input" 2>/dev/null | while IFS= read -r line; do
    if command -v jq >/dev/null 2>&1; then
      printf '\033[1;32m<<\033[0m '
      jq -c . <<<"$line"
    else
      printf '\033[1;32m<<\033[0m %s\n' "$line"
    fi
  done

  echo
  log "Done. All commands were sent via NDJSON."
}

run_plain() {
  ensure_built
  log "Starting interactive plain-text REPL (gdb-style)."
  log "Adapter: fake"
  log "Program: ${DEMO_PROGRAM}"
  log "Try: show, bt, n, locals, p 1+1, q"
  echo
  print_repl_tips
  echo
  run_repl_interactive fake
}

run_debugpy() {
  require_debugpy
  warn_stale_sessions
  ensure_built_cli
  log "Starting interactive REPL with debugpy."
  log "Python: ${PYTHON_BIN}"
  log "Program: ${DEMO_PROGRAM}"
  log "Try: show, bt, n, s, b 50, c, locals, p total, q"
  echo
  print_repl_tips
  echo
  run_repl_interactive python
}

run_repl_interactive() {
  local adapter="$1"
  (
    cd "$FIXTURE_DIR"
    env PATH="$(target_dir)/debug:${PATH}" PYTHON="${PYTHON_BIN}" RUST_LOG=warn \
      dap-cli debug repl --adapter "$adapter" "$DEMO_PROGRAM"
  )
}

case "$MODE" in
  auto | demo | test)
    run_auto "$COMMANDS_FILE"
    ;;
  plain | interactive | gdb | fake)
    run_plain
    ;;
  debugpy | python | py)
    run_debugpy
    ;;
  cleanup | kill)
    run_cleanup
    ;;
  ndjson | json | pipe)
    run_auto "${2:-$COMMANDS_FILE}"
    ;;
  help | -h | --help)
    sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
    echo
    print_repl_tips
    ;;
  *)
    die "unknown mode: $MODE (try: auto, plain, debugpy, ndjson, help)"
    ;;
esac
