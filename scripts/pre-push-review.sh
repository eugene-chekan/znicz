#!/usr/bin/env bash
# Claude Code PreToolUse hook: review the branch before `git push`.
#
# Runs a headless Claude review of everything the push would add over
# origin/main. Critical or Important findings block the push (exit 2): the
# findings go back to the agent, which fixes them and pushes again. A review
# that cannot run (no `claude`, timeout, unreadable answer) lets the push go
# through with a warning, so tooling trouble never blocks work.
#
# The reviewer sees only the diff (no tools): letting it open files made a
# large branch take over nine minutes; diff-only takes about five at the
# 150 KB cap.
#
# Skips:
#   - nothing new over origin/main (a tag push from main, a no-op push);
#   - a HEAD that already passed (cached in .git/znicz-review-passed);
#   - `ZNICZ_SKIP_REVIEW=1 git push …`;
#   - nested runs (the reviewer itself never pushes, but be safe).
set -uo pipefail

input=$(cat)
command=$(jq -r '.tool_input.command // empty' <<<"$input")

# Only `git push` (the hook's `if` filter already narrows this).
[[ "$command" =~ (^|[[:space:];&|])git[[:space:]]+push([[:space:]]|$) ]] || exit 0
[[ "$command" == *ZNICZ_SKIP_REVIEW=1* ]] && exit 0
[[ -n "${ZNICZ_REVIEW_RUNNING:-}" ]] && exit 0

repo=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
cd "$repo" || exit 0

warn() {
    jq -n --arg msg "$1" '{systemMessage: $msg}'
    exit 0
}

git fetch --quiet origin main 2>/dev/null || true
base=$(git merge-base HEAD origin/main 2>/dev/null) || warn "pre-push review skipped: no origin/main to compare with"
head=$(git rev-parse HEAD)
[[ "$base" == "$head" ]] && exit 0

passed_file="$(git rev-parse --git-dir)/znicz-review-passed"
[[ -f "$passed_file" && "$(cat "$passed_file")" == "$head" ]] && exit 0

command -v claude >/dev/null || warn "pre-push review skipped: claude CLI not found"

max_bytes=150000
diff=$(git diff --stat "$base" "$head"; echo; git diff "$base" "$head")
if (( ${#diff} > max_bytes )); then
    diff="${diff:0:max_bytes}

[diff cut at ${max_bytes} bytes; review what is shown]"
fi
log=$(git log --oneline "$base..$head")

prompt="You are reviewing a branch of the znicz Rust workspace before it is pushed.
The commits:
$log

The diff against origin/main is on stdin. Review it from the diff alone.

Review two things.

1. Defects: correctness bugs, data loss, crashes or panics, security problems,
   broken behaviour on Windows or Linux, protocol or config incompatibilities,
   tests that cannot fail, and violations of AGENTS.md (version bump, wiki sync).

2. Code quality and architecture. The code must follow good software
   engineering practice and stay maintainable. Check the changed code for:
   - Clean code: functions and types with one clear responsibility; functions
     short enough to follow; names that say what a thing is or does; no
     duplicated logic that should be shared; no dead code, leftover debugging,
     commented-out code or stale comments; comments that explain why, not what;
     no magic numbers without a named constant; errors handled, not swallowed
     or unwrapped where they can happen.
   - Clean architecture: dependencies point the right way (znicz-core and
     znicz-library know nothing of the TUI, MCP or the binary; views render,
     they do not own state or side effects); each module and crate keeps to its
     role; boundaries go through small, explicit interfaces (traits, plain data)
     rather than reaching into another module's internals; no new global or
     hidden shared state; I/O kept at the edges so the logic stays testable.
   - Maintainability: new behaviour has tests that would fail without it;
     tests check behaviour, not implementation details; files and functions
     that grew too large are split where the change touches them; changes
     follow the patterns already used in the codebase.

Do not report formatting or lints (rustfmt and clippy run in CI) or pure
taste with no effect on correctness or maintainability.

Rate each finding:
- Critical: wrong behaviour, data loss, a crash or a security hole.
- Important: a likely bug, or a quality or architecture problem that will make
  the code harder to change or easy to break (a layering violation, duplicated
  logic, a function or type doing several jobs, untested new behaviour,
  swallowed errors).
- Minor: a small improvement that does not threaten correctness or
  maintainability.
Report only findings you are confident about, each with file:line, what is
wrong and why it matters, and a concrete fix.

End with exactly one line:
VERDICT: BLOCK   (if there is at least one Critical or Important finding)
VERDICT: PASS    (otherwise)"

review=$(ZNICZ_REVIEW_RUNNING=1 timeout 540 claude -p "$prompt" \
    --model sonnet \
    --effort medium \
    --tools "" \
    --no-session-persistence \
    <<<"$diff" 2>/dev/null) || warn "pre-push review did not finish; pushing without it"

verdict=$(grep -Eo 'VERDICT: (PASS|BLOCK)' <<<"$review" | tail -n 1)
case "$verdict" in
    "VERDICT: PASS")
        echo "$head" >"$passed_file"
        jq -n --arg msg "pre-push review passed for $(git rev-parse --short HEAD)" '{systemMessage: $msg}'
        exit 0
        ;;
    "VERDICT: BLOCK")
        {
            echo "Push blocked by the pre-push code review (Critical/Important findings)."
            echo "Fix them and push again; ZNICZ_SKIP_REVIEW=1 git push skips the review."
            echo
            echo "$review"
        } >&2
        exit 2
        ;;
    *)
        warn "pre-push review gave no verdict; pushing without it"
        ;;
esac
