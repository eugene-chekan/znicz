#!/usr/bin/env bash
# Claude Code PreToolUse hook: review the branch before `git push`.
#
# Runs a headless Claude review of everything the push would add over
# origin/main. Critical or Important findings block the push (exit 2): the
# findings go back to the agent, which fixes them and pushes again. A review
# that cannot run (no `claude`, timeout, unreadable answer) lets the push go
# through with a warning, so tooling trouble never blocks work.
#
# The reviewer runs without the user's plugins, skills or MCP servers and
# sees only the diff (no tools): letting it open files made a
# large branch take over nine minutes; diff-only takes about four at the
# 100 KB part size. A larger diff is split into parts along file boundaries,
# each reviewed in parallel; any part that blocks blocks the push. Generated
# files (Cargo.lock, THIRD-PARTY-LICENSES.html) are left out.
#
# NOTE for agents: this hook blocks the whole Bash command. Never chain
# `git commit … && git push`: a blocked push also drops the commit.
#
# Skips:
#   - nothing new over origin/main (a tag push from main, a no-op push);
#   - a HEAD that already passed (cached in .git/znicz-review-passed);
#   - `ZNICZ_SKIP_REVIEW=1 git push …`;
#   - nested runs (the reviewer itself never pushes, but be safe).
#
# For trying the splitting by hand:
#   ZNICZ_REVIEW_BASE=<rev>   compare with this instead of origin/main;
#   ZNICZ_REVIEW_DRY_RUN=1    print the parts and their sizes, call no model.
set -uo pipefail

input=$(cat)
command=$(jq -r '.tool_input.command // empty' <<<"$input")

# Only `git push`.
[[ "$command" =~ (^|[[:space:];&|])git[[:space:]]+push([[:space:]]|$) ]] || exit 0
[[ "$command" == *ZNICZ_SKIP_REVIEW=1* ]] && exit 0
[[ -n "${ZNICZ_REVIEW_RUNNING:-}" ]] && exit 0

repo=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
cd "$repo" || exit 0

warn() {
    jq -n --arg msg "$1" '{systemMessage: $msg}'
    exit 0
}

dry_run=${ZNICZ_REVIEW_DRY_RUN:-}

if [[ -n "${ZNICZ_REVIEW_BASE:-}" ]]; then
    base=$(git rev-parse "$ZNICZ_REVIEW_BASE") || warn "pre-push review skipped: bad ZNICZ_REVIEW_BASE"
else
    git fetch --quiet origin main 2>/dev/null || true
    base=$(git merge-base HEAD origin/main 2>/dev/null) || warn "pre-push review skipped: no origin/main to compare with"
fi
head=$(git rev-parse HEAD)
[[ "$base" == "$head" ]] && exit 0

passed_file="$(git rev-parse --git-dir)/znicz-review-passed"
if [[ -z "$dry_run" && -f "$passed_file" && "$(cat "$passed_file")" == "$head" ]]; then
    exit 0
fi

[[ -n "$dry_run" ]] || command -v claude >/dev/null || warn "pre-push review skipped: claude CLI not found"

# Bytes of diff per reviewer, and the most parts worth reviewing at once.
max_bytes=100000
max_parts=8
excludes=(':(exclude)Cargo.lock' ':(exclude)THIRD-PARTY-LICENSES.html')

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# Split the diff into parts of at most max_bytes, whole files at a time. A
# single file over the limit is cut and says so.
parts=0
part_bytes=0
new_part() {
    parts=$((parts + 1))
    part_bytes=0
    : >"$work/part.$parts"
}
new_part
while IFS= read -r -d '' file; do
    file_diff=$(git diff "$base" "$head" -- "$file")
    if ((${#file_diff} > max_bytes)); then
        file_diff="${file_diff:0:max_bytes}

[diff of $file cut at ${max_bytes} bytes; review what is shown]"
    fi
    if ((part_bytes > 0 && part_bytes + ${#file_diff} > max_bytes)); then
        new_part
    fi
    printf '%s\n' "$file_diff" >>"$work/part.$parts"
    part_bytes=$((part_bytes + ${#file_diff} + 1))
done < <(git diff -z --name-only "$base" "$head" -- . "${excludes[@]}")

if [[ -n "$dry_run" ]]; then
    for i in $(seq 1 "$parts"); do
        printf 'part %d/%d: %d bytes\n' "$i" "$parts" "$(wc -c <"$work/part.$i")" >&2
    done
    exit 0
fi

((parts > max_parts)) && warn "pre-push review skipped: the diff needs $parts parts (over $max_parts); review the branch in smaller pushes"

log=$(git log --oneline "$base..$head")
stat=$(git diff --stat "$base" "$head" -- . "${excludes[@]}")

prompt_for() {
    local index=$1
    local scope="The diff against origin/main is on stdin. Review it from the diff alone."
    if ((parts > 1)); then
        scope="The branch is too large for one review, so it is split by file into $parts parts.
You see part $index of $parts on stdin; the other parts are reviewed separately.
The full list of changed files is above."
    fi
    cat <<EOF
You are reviewing a branch of the znicz Rust workspace before it is pushed.
The commits:
$log

Changed files:
$stat

$scope

Code outside what you see exists: helpers called but not shown, files in
other parts, and the rest of each file. Report a finding only when the code
you can see proves it. A finding that depends on code you cannot see (what a
called helper does, whether something is handled elsewhere) is at most Minor:
say which code you would need to check. Never describe code you cannot see as
if you had read it.

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
VERDICT: PASS    (otherwise)
EOF
}

# A plain reviewer: without the user's plugins, skills and MCP servers, which
# made it narrate skill lookups and write out tool calls it cannot make.
system_prompt="You are a code reviewer. You have no tools: read the diff you are given and answer in plain text."

# One reviewer per part, all at once.
for i in $(seq 1 "$parts"); do
    (
        ZNICZ_REVIEW_RUNNING=1 timeout 540 claude -p "$(prompt_for "$i")" \
            --model sonnet \
            --effort medium \
            --tools "" \
            --no-session-persistence \
            --setting-sources project \
            --disable-slash-commands \
            --strict-mcp-config \
            --system-prompt "$system_prompt" \
            <"$work/part.$i" >"$work/review.$i" 2>/dev/null
    ) &
done
wait

blocked=()
unread=0
for i in $(seq 1 "$parts"); do
    verdict=$(grep -Eo 'VERDICT: (PASS|BLOCK)' "$work/review.$i" 2>/dev/null | tail -n 1)
    case "$verdict" in
        "VERDICT: PASS") ;;
        "VERDICT: BLOCK") blocked+=("$i") ;;
        *) unread=$((unread + 1)) ;;
    esac
done

if ((${#blocked[@]} > 0)); then
    {
        echo "Push blocked by the pre-push code review (Critical/Important findings)."
        echo "Fix them, commit, and push again in a separate command;"
        echo "ZNICZ_SKIP_REVIEW=1 git push skips the review."
        for i in "${blocked[@]}"; do
            echo
            ((parts > 1)) && echo "## Part $i of $parts"
            cat "$work/review.$i"
        done
    } >&2
    exit 2
fi

if ((unread > 0)); then
    warn "pre-push review gave no verdict for $unread of $parts part(s); pushing without it"
fi

echo "$head" >"$passed_file"
summary="pre-push review passed for $(git rev-parse --short HEAD)"
((parts > 1)) && summary="$summary ($parts parts)"
jq -n --arg msg "$summary" '{systemMessage: $msg}'
exit 0
