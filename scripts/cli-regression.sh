#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN_PATH="${SCODEX_TEST_BIN:-$ROOT_DIR/target/debug/scodex}"
TMP_ROOT="${SCODEX_TEST_TMPDIR:-$(mktemp -d /tmp/scodex-cli-regression.XXXXXX)}"
STATE1="$TMP_ROOT/state1"
STATE2="$TMP_ROOT/state2"
STATE3="$TMP_ROOT/state3"
STATE4="$TMP_ROOT/state4"
HOME1="$TMP_ROOT/codex-home1"
HOME2="$TMP_ROOT/codex-home2"
HOME3="$TMP_ROOT/codex-home3"
IMPORT_HOME="$TMP_ROOT/import-home"
REMOTE="$TMP_ROOT/remote.git"
UPDATE_BIN="$TMP_ROOT/scodex-update"
POOL_KEY="test-pool-key-123"

log() {
  printf '[cli-regression] %s\n' "$*"
}

fail() {
  printf '[cli-regression] FAIL: %s\n' "$*" >&2
  exit 1
}

assert_contains() {
  local haystack="$1"
  local needle="$2"
  if ! printf '%s' "$haystack" | rg -F "$needle" >/dev/null; then
    fail "missing expected text: $needle"
  fi
}

assert_matches() {
  local haystack="$1"
  local regex="$2"
  if ! printf '%s' "$haystack" | rg "$regex" >/dev/null; then
    fail "missing expected pattern: $regex"
  fi
}

section() {
  local name="$1"
  local status="$2"
  local output="$3"
  printf '## %s\n' "$name"
  printf 'exit=%s\n' "$status"
  printf '%s\n\n' "$output"
}

run_capture() {
  local output
  set +e
  output="$("$@" 2>&1)"
  local status=$?
  set -e
  printf '%s\n__SCODEX_STATUS__=%s\n' "$output" "$status"
}

extract_status() {
  printf '%s' "$1" | tail -n 1 | sed 's/^__SCODEX_STATUS__=//'
}

extract_output() {
  printf '%s' "$1" | sed '$d'
}

run_ok() {
  local name="$1"
  shift
  local raw
  raw="$(run_capture "$@")"
  local status
  status="$(extract_status "$raw")"
  local output
  output="$(extract_output "$raw")"
  section "$name" "$status" "$output"
  if [ "$status" -ne 0 ]; then
    fail "$name returned non-zero"
  fi
  RUN_OUTPUT="$output"
}

run_fail() {
  local name="$1"
  shift
  local raw
  raw="$(run_capture "$@")"
  local status
  status="$(extract_status "$raw")"
  local output
  output="$(extract_output "$raw")"
  section "$name" "$status" "$output"
  if [ "$status" -eq 0 ]; then
    fail "$name unexpectedly succeeded"
  fi
  RUN_OUTPUT="$output"
}

ensure_binary() {
  if [ ! -x "$BIN_PATH" ]; then
    log "building target/debug/scodex"
    cargo build --manifest-path "$ROOT_DIR/Cargo.toml" >/dev/null
  fi
  [ -x "$BIN_PATH" ] || fail "binary not found: $BIN_PATH"
}

make_fake_jwt() {
  local payload="$1"
  local header
  header="$(printf '%s' '{"alg":"none"}' | base64 | tr '+/' '-_' | tr -d '=\n')"
  local body
  body="$(printf '%s' "$payload" | base64 | tr '+/' '-_' | tr -d '=\n')"
  printf '%s.%s.sig' "$header" "$body"
}

prepare_fixture_files() {
  mkdir -p "$STATE1" "$STATE2" "$STATE3" "$STATE4" "$HOME1" "$HOME2" "$HOME3" "$IMPORT_HOME"
  git init --bare "$REMOTE" >/dev/null
  cp "$BIN_PATH" "$UPDATE_BIN"

  local sub_jwt
  sub_jwt="$(make_fake_jwt '{"email":"sub@example.com","https://api.openai.com/auth":{"chatgpt_plan_type":"plus"}}')"
  cat >"$TMP_ROOT/sub-auth.json" <<JSON
{"tokens":{"id_token":"$sub_jwt","account_id":"acct-sub-1"}}
JSON

  local known_jwt
  known_jwt="$(make_fake_jwt '{"email":"known@example.com","https://api.openai.com/auth":{"chatgpt_plan_type":"pro"}}')"
  cat >"$IMPORT_HOME/auth.json" <<JSON
{"tokens":{"id_token":"$known_jwt","account_id":"acct-known-1"}}
JSON
}

verify_main_flow() {
  run_ok \
    "add --api" \
    env CODEX_HOME="$HOME1" "$BIN_PATH" --state-dir "$STATE1" add --api \
    --API_TOKEN sk-12345678 --BASE_URL https://api.openai.com/v1 --provider openai
  assert_contains "$RUN_OUTPUT" "Added 345678@openai"
  assert_contains "$RUN_OUTPUT" "Switched to 345678@openai"

  run_ok "list after add" env CODEX_HOME="$HOME1" "$BIN_PATH" --state-dir "$STATE1" list
  assert_contains "$RUN_OUTPUT" "345678@openai"

  run_ok "refresh after add" env CODEX_HOME="$HOME1" "$BIN_PATH" --state-dir "$STATE1" refresh
  assert_contains "$RUN_OUTPUT" "345678@openai"

  run_ok \
    "launch dry-run" \
    env CODEX_HOME="$HOME1" "$BIN_PATH" --state-dir "$STATE1" launch \
    --dry-run --no-login --no-import-known
  assert_contains "$RUN_OUTPUT" "Would select 345678@openai"

  run_ok \
    "launch no-launch" \
    env CODEX_HOME="$HOME1" "$BIN_PATH" --state-dir "$STATE1" launch \
    --no-launch --no-login --no-import-known
  assert_contains "$RUN_OUTPUT" "Switched to 345678@openai"

  run_ok \
    "auto dry-run" \
    env CODEX_HOME="$HOME1" "$BIN_PATH" --state-dir "$STATE1" auto \
    --dry-run --no-login --no-import-known
  assert_contains "$RUN_OUTPUT" "Would select 345678@openai"

  run_ok "use existing api account" env CODEX_HOME="$HOME1" "$BIN_PATH" --state-dir "$STATE1" use 345678@openai
  assert_contains "$RUN_OUTPUT" "Switched to 345678@openai"

  run_ok \
    "push local pool" \
    env CODEX_HOME="$HOME1" SCODEX_POOL_KEY="$POOL_KEY" "$BIN_PATH" --state-dir "$STATE1" push "$REMOTE"
  assert_matches "$RUN_OUTPUT" "account pool"

  run_ok \
    "pull local pool" \
    env CODEX_HOME="$HOME2" SCODEX_POOL_KEY="$POOL_KEY" "$BIN_PATH" --state-dir "$STATE2" pull "$REMOTE"
  assert_matches "$RUN_OUTPUT" "account pool"
  assert_contains "$RUN_OUTPUT" "345678@openai"

  run_ok "list after pull" env CODEX_HOME="$HOME2" "$BIN_PATH" --state-dir "$STATE2" list
  assert_contains "$RUN_OUTPUT" "345678@openai"

  run_ok "use pulled account" env CODEX_HOME="$HOME2" "$BIN_PATH" --state-dir "$STATE2" use 345678@openai
  assert_contains "$RUN_OUTPUT" "Switched to 345678@openai"

  run_ok "rm api account" env CODEX_HOME="$HOME1" "$BIN_PATH" --state-dir "$STATE1" rm -y 345678@openai
  assert_contains "$RUN_OUTPUT" "Removed 345678@openai"

  run_ok "list after rm" env CODEX_HOME="$HOME1" "$BIN_PATH" --state-dir "$STATE1" list
  assert_contains "$RUN_OUTPUT" "No usable accounts found"
}

verify_import_and_alias_flow() {
  run_ok \
    "login --api" \
    env CODEX_HOME="$HOME3" "$BIN_PATH" --state-dir "$STATE3" login --api \
    --API_TOKEN sk-87654321 --BASE_URL https://openrouter.ai/api/v1 --provider openrouter
  assert_contains "$RUN_OUTPUT" "Added 654321@openrouter"
  assert_contains "$RUN_OUTPUT" "Switched to 654321@openrouter"

  run_ok \
    "import-auth subscription auth" \
    env CODEX_HOME="$HOME3" "$BIN_PATH" --state-dir "$STATE3" import-auth "$TMP_ROOT/sub-auth.json"
  assert_contains "$RUN_OUTPUT" "Imported sub@example.com"

  run_ok \
    "import-known from CODEX_HOME" \
    env CODEX_HOME="$IMPORT_HOME" "$BIN_PATH" --state-dir "$STATE4" import-known
  assert_contains "$RUN_OUTPUT" "Imported known@example.com"

  run_ok "use imported known account" env CODEX_HOME="$IMPORT_HOME" "$BIN_PATH" --state-dir "$STATE4" use known@example.com
  assert_contains "$RUN_OUTPUT" "Switched to known@example.com"

  run_fail "deploy invalid target" env CODEX_HOME="$IMPORT_HOME" "$BIN_PATH" --state-dir "$STATE4" deploy bad-target
  assert_contains "$RUN_OUTPUT" "Invalid remote target"

  run_fail "sync invalid target alias" env CODEX_HOME="$IMPORT_HOME" "$BIN_PATH" --state-dir "$STATE4" sync bad-target
  assert_contains "$RUN_OUTPUT" "Invalid remote target"
}

verify_update_flow() {
  run_ok "update copied binary" "$UPDATE_BIN" update
  assert_matches "$RUN_OUTPUT" "Already on the latest installed version|Already up to date|Updated scodex to"
}

main() {
  ensure_binary
  prepare_fixture_files

  log "temporary test root: $TMP_ROOT"
  log "binary under test: $BIN_PATH"

  verify_main_flow
  verify_import_and_alias_flow
  verify_update_flow

  log "PASS: CLI regression checks completed"
}

main "$@"
