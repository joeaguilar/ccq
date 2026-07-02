#!/usr/bin/env bash
# Integration test suite for ccq. Runs the built binary against committed
# fixtures and asserts on output shape, filters, robustness, and exit codes.
#
# Usage:
#   ./tests/integration.sh                 # test target/release/ccq
#   ./tests/integration.sh ./target/debug/ccq
#   ./tests/integration.sh --smoke <bin>   # quick release-gate smoke test

set -euo pipefail

cd "$(dirname "$0")/.."

SMOKE=0
if [ "${1:-}" = "--smoke" ]; then
    SMOKE=1
    shift
fi
BIN="${1:-./target/release/ccq}"
ROOT="tests/fixtures/root"

PASS=0
FAIL=0

fail() {
    echo "✗ $1" >&2
    FAIL=$((FAIL + 1))
}

ok() {
    PASS=$((PASS + 1))
}

check() {
    local desc="$1"; shift
    if "$@" > /dev/null 2>&1; then
        ok
    else
        fail "$desc"
    fi
}

check_eq() {
    local desc="$1" expected="$2" actual="$3"
    if [ "$expected" = "$actual" ]; then
        ok
    else
        fail "$desc (expected '$expected', got '$actual')"
    fi
}

check_contains() {
    local desc="$1" needle="$2" haystack="$3"
    if printf '%s' "$haystack" | grep -qF "$needle"; then
        ok
    else
        fail "$desc (missing '$needle')"
    fi
}

check_not_contains() {
    local desc="$1" needle="$2" haystack="$3"
    if printf '%s' "$haystack" | grep -qF "$needle"; then
        fail "$desc (unexpectedly found '$needle')"
    else
        ok
    fi
}

# ---- Smoke mode (used by the release workflow on every target) --------------

if [ "$SMOKE" = 1 ]; then
    "$BIN" --version
    "$BIN" --help > /dev/null
    OUT=$("$BIN" projects --root "$ROOT" -f tsv)
    printf '%s\n' "$OUT" | grep -q "proj-alpha" || { echo "smoke: projects missing proj-alpha" >&2; exit 1; }
    echo "smoke OK: $("$BIN" --version)"
    exit 0
fi

[ -x "$BIN" ] || { echo "binary not found: $BIN (build first)" >&2; exit 1; }

echo "Testing $BIN against $ROOT"

# ---- projects / sessions -----------------------------------------------------

OUT=$("$BIN" projects --root "$ROOT" -f tsv 2>/dev/null)
check_eq "projects: two projects listed" 2 "$(printf '%s\n' "$OUT" | tail -n +2 | wc -l | tr -d ' ')"
check_contains "projects: alpha present" "proj-alpha" "$OUT"
check_not_contains "projects: memory dir excluded" "memory" "$OUT"

OUT=$("$BIN" sessions --root "$ROOT" -p alpha -f tsv 2>/dev/null)
check_contains "sessions: session A listed" "aaaa1111" "$OUT"
check_not_contains "sessions: empty session excluded" "aaaa2222" "$OUT"
ROW=$(printf '%s\n' "$OUT" | grep aaaa1111)
check_eq "sessions: user_msgs counts genuine prompts only" "2" "$(printf '%s' "$ROW" | cut -f6)"
check_eq "sessions: cwd captured" "/Users/test/proj/alpha" "$(printf '%s' "$ROW" | cut -f9)"

# ---- bash --------------------------------------------------------------------

OUT=$("$BIN" bash --root "$ROOT" -f tsv 2>/dev/null)
check_eq "bash: two commands" 2 "$(printf '%s\n' "$OUT" | tail -n +2 | wc -l | tr -d ' ')"
OUT=$("$BIN" bash --root "$ROOT" --complex -f tsv 2>/dev/null)
check_eq "bash --complex: only the for-loop" 1 "$(printf '%s\n' "$OUT" | tail -n +2 | wc -l | tr -d ' ')"
check_contains "bash --complex: loop command" "for f in *" "$OUT"
OUT=$("$BIN" bash --root "$ROOT" --grep 'cargo' -f tsv 2>/dev/null)
check_eq "bash --grep: regex filter" 1 "$(printf '%s\n' "$OUT" | tail -n +2 | wc -l | tr -d ' ')"

# Golden: exact TSV output is pinned
"$BIN" bash --root "$ROOT" -p alpha -f tsv 2>/dev/null > /tmp/ccq-bash-golden.$$
if diff -q tests/golden/bash.tsv /tmp/ccq-bash-golden.$$ > /dev/null 2>&1; then
    ok
else
    fail "bash: golden TSV mismatch"
    diff tests/golden/bash.tsv /tmp/ccq-bash-golden.$$ >&2 || true
fi
rm -f /tmp/ccq-bash-golden.$$

# ---- writes ------------------------------------------------------------------

OUT=$("$BIN" writes --root "$ROOT" -f tsv 2>/dev/null)
check_eq "writes: Write + Edit found" 2 "$(printf '%s\n' "$OUT" | tail -n +2 | wc -l | tr -d ' ')"
OUT=$("$BIN" writes --root "$ROOT" --scratch -f tsv 2>/dev/null)
check_eq "writes --scratch: only /tmp path" 1 "$(printf '%s\n' "$OUT" | tail -n +2 | wc -l | tr -d ' ')"
check_contains "writes --scratch: scratch path" "/tmp/scratch.py" "$OUT"

# ---- tools -------------------------------------------------------------------

OUT=$("$BIN" tools --root "$ROOT" -f tsv 2>/dev/null)
check_contains "tools: Bash rollup" "Bash	2" "$OUT"
OUT=$("$BIN" tools --root "$ROOT" --errors-only -f tsv 2>/dev/null)
check_contains "tools --errors-only: only failed Bash" "Bash	1" "$OUT"
check_not_contains "tools --errors-only: Write excluded" "Write" "$OUT"

# ---- prompts -----------------------------------------------------------------

OUT=$("$BIN" prompts --root "$ROOT" -p alpha --sidechain exclude -f tsv 2>/dev/null)
check_eq "prompts: two genuine prompts" 2 "$(printf '%s\n' "$OUT" | tail -n +2 | wc -l | tr -d ' ')"
check_contains "prompts: string content" "Fix the login bug" "$OUT"
check_contains "prompts: array content" "Second prompt" "$OUT"
check_not_contains "prompts: command wrapper excluded" "command-name" "$OUT"
check_not_contains "prompts: reminder-only excluded" "only a reminder" "$OUT"
check_not_contains "prompts: task-notification excluded" "task-notification" "$OUT"

# ---- errors ------------------------------------------------------------------

OUT=$("$BIN" errors --root "$ROOT" -f tsv 2>/dev/null)
check_eq "errors: one failed result" 1 "$(printf '%s\n' "$OUT" | tail -n +2 | wc -l | tr -d ' ')"
check_contains "errors: tool resolved from paired use" "Bash" "$OUT"
check_contains "errors: path stripped in signature" "Permission denied: <path>" "$OUT"

# ---- slash -------------------------------------------------------------------

OUT=$("$BIN" slash --root "$ROOT" -f tsv 2>/dev/null)
check_contains "slash: command-name wrapper" "/blitz" "$OUT"
check_contains "slash: Skill tool_use" "/sprint" "$OUT"

# ---- sidechain scoping ---------------------------------------------------------

OUT=$("$BIN" prompts --root "$ROOT" --sidechain only -f tsv 2>/dev/null)
check_contains "sidechain only: subagent prompt" "subagent task prompt" "$OUT"
check_not_contains "sidechain only: main traffic excluded" "Fix the login bug" "$OUT"
OUT=$("$BIN" prompts --root "$ROOT" --sidechain exclude -f tsv 2>/dev/null)
check_not_contains "sidechain exclude: subagent prompt gone" "subagent task prompt" "$OUT"

# ---- time filters --------------------------------------------------------------

OUT=$("$BIN" prompts --root "$ROOT" --since 2026-01-05T10:05:00Z --until 2026-01-05T10:07:00Z -f tsv 2>/dev/null)
check_contains "time window: second prompt inside" "Second prompt" "$OUT"
check_not_contains "time window: first prompt outside" "Fix the login bug" "$OUT"

# ---- grep ----------------------------------------------------------------------

OUT=$("$BIN" grep --root "$ROOT" 'needle_xyz' --in thinking -f tsv 2>/dev/null)
check_contains "grep --in thinking: match found" "bbbb1111" "$OUT"
OUT=$("$BIN" grep --root "$ROOT" 'needle_xyz' --in tool-use -f tsv 2>/dev/null || true)
check_not_contains "grep --in tool-use: thinking not matched" "bbbb1111" "$OUT"

# ---- show ----------------------------------------------------------------------

OUT=$("$BIN" show aaaa1111 --root "$ROOT" --turn 2 2>/dev/null)
check_contains "show --turn 2: second turn prompt" "Second prompt" "$OUT"
check_not_contains "show --turn 2: first turn excluded" "Fix the login bug" "$OUT"
OUT=$("$BIN" show aaaa1111 --root "$ROOT" --line 3 --around 0 2>/dev/null)
check_contains "show --line: tool_use rendered" "tool_use[Bash]" "$OUT"

# ---- agents --------------------------------------------------------------------

OUT=$("$BIN" agents --root "$ROOT" -f tsv 2>/dev/null)
check_contains "agents: tasks/*.output discovered" "task0001" "$OUT"
check_contains "agents: final text captured" "Final agent answer" "$OUT"
check_contains "agents: token sum" "333" "$OUT"
check_contains "agents: StructuredOutput return" "bugs" "$OUT"
check_contains "agents: journal entry" "review:bugs" "$OUT"

# ---- stats ---------------------------------------------------------------------

OUT=$("$BIN" stats --root "$ROOT" -f tsv 2>/dev/null)
check_contains "stats: project count" "projects	2" "$OUT"
check_contains "stats: bash rollup" "bash_commands	2" "$OUT"

# ---- count/fields/formats -------------------------------------------------------

OUT=$("$BIN" bash --root "$ROOT" --count --by command -f tsv 2>/dev/null)
check_contains "count --by command" "cargo build	1" "$OUT"
OUT=$("$BIN" bash --root "$ROOT" --fields command -f tsv 2>/dev/null)
check_eq "fields: single column header" "command" "$(printf '%s\n' "$OUT" | head -1)"
OUT=$("$BIN" bash --root "$ROOT" -f json 2>/dev/null)
printf '%s' "$OUT" | python3 -c 'import json,sys; json.load(sys.stdin)' 2>/dev/null && ok || fail "json output parses"
OUT=$("$BIN" bash --root "$ROOT" -f jsonl 2>/dev/null | head -1)
printf '%s' "$OUT" | python3 -c 'import json,sys; json.loads(sys.stdin.read())' 2>/dev/null && ok || fail "jsonl output parses"

# ---- robustness ------------------------------------------------------------------

ERR=$("$BIN" stats --root "$ROOT" 2>&1 >/dev/null || true)
check_contains "malformed line reported on stderr" "malformed line" "$ERR"

# 5 MB single line: generated on the fly, must stream without crash
BIGROOT=$(mktemp -d)
mkdir -p "$BIGROOT/-Users-test-proj-big"
python3 - "$BIGROOT" <<'EOF'
import json, sys
root = sys.argv[1]
big = "x" * (5 * 1024 * 1024)
with open(f"{root}/-Users-test-proj-big/cccc1111-0000-0000-0000-000000000021.jsonl", "w") as f:
    f.write(json.dumps({"type":"user","timestamp":"2026-01-05T11:00:00.000Z",
                        "message":{"role":"user","content":big}}) + "\n")
EOF
check "5MB single line survives" "$BIN" prompts --root "$BIGROOT" -f tsv
rm -rf "$BIGROOT"

# ---- exit codes -------------------------------------------------------------------

set +e
"$BIN" bash --root "$ROOT" --grep 'zzz_no_match_zzz' > /dev/null 2>&1
check_eq "exit 1 on no matches" 1 "$?"
"$BIN" bash --root "$ROOT" --since 'not-a-date' > /dev/null 2>&1
check_eq "exit 2 on bad --since" 2 "$?"
"$BIN" bash --root /nonexistent-root-zzz > /dev/null 2>&1
check_eq "exit 2 on missing root" 2 "$?"
"$BIN" prompts --root "$ROOT" -s abc > /dev/null 2>&1
check_eq "exit 2 on short session prefix" 2 "$?"
"$BIN" projects --root "$ROOT" > /dev/null 2>&1
check_eq "exit 0 on matches" 0 "$?"
set -e

# ------------------------------------------------------------------------------------

echo
echo "Results: $PASS passed, $FAIL failed"
[ "$FAIL" = 0 ]
