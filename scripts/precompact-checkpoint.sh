#!/usr/bin/env bash
# Write the mechanical half of a session checkpoint, and tell the model to write the rest.
#
# Wired to the PreCompact hook so that compaction -- which is where a session's working
# state is otherwise summarised away -- first lands the part of AGENTS.md's close-out that
# a script can do honestly: the counts, re-derived by running the commands, plus the git
# state. It cannot do the other half. What was decided, what is still open and what the next
# step is are the model's to write, so this emits `additionalContext` asking for them rather
# than pretending a script can supply them.
#
# Deliberately does NOT block the compaction. Auto-compact fires because the context window
# is full; refusing it would wedge the session at exactly the moment it has no room to argue.
#
# Deliberately does NOT run `cargo test`. A full suite is minutes and a hook has seconds, so
# the test line is left unfilled with its command beside it -- an absent number a human will
# fill is safer than a stale one nobody re-checks, which is the failure AGENTS.md records.
#
# Reads the hook payload on stdin (session_id, trigger) and ignores everything else.

set -uo pipefail

repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo" || exit 0

payload="$(cat 2>/dev/null || true)"
trigger="$(printf '%s' "$payload" | /usr/bin/python3 -c \
  'import json,sys
try: print(json.load(sys.stdin).get("trigger","unknown"))
except Exception: print("unknown")' 2>/dev/null || echo unknown)"

mkdir -p checkpoints
stamp="$(date -u +%Y-%m-%d-%H%M)"
out="checkpoints/${stamp}-precompact.md"

# Every count below is produced by running its command now, never carried forward.
mutations="$(python3 scripts/mutate.py --list 2>/dev/null \
  | awk '{print $2}' | grep -E '^(caught|survives|compiler)$' | sort | uniq -c \
  | awk '{printf "%s %s, ", $1, $2}' | sed 's/, $//')"
stale="$(python3 scripts/mutate.py --stale 2>&1 | tail -1)"
links="$(python3 scripts/checklinks.py 2>&1 | tail -1)"

# One call covers every region TRACKING.md declares, so a region added later is reported here
# without this script being told about it -- which is what the three hard-coded pairs could not do.
index_lines=""
if [ -f TRACKING.md ]; then
  index_lines="$(python3 scripts/genindex.py TRACKING.md --check 2>&1 \
    | sed -e 's/^TRACKING.md /  - `TRACKING.md` /' -e 's/^/  /' | head -20)
"
fi

{
  printf '# Checkpoint — %s (written by the PreCompact hook, trigger: %s)\n\n' "$stamp" "$trigger"
  printf 'The counts here were re-derived by running their commands at %s. The prose sections\n' "$(date -u '+%H:%M UTC')"
  printf 'are empty because a script cannot write them — see the note at the end.\n\n'

  printf '## Repository state\n\n'
  printf -- '- HEAD: `%s`\n' "$(git log --oneline -1 2>/dev/null)"
  printf -- '- Branch: `%s`\n' "$(git rev-parse --abbrev-ref HEAD 2>/dev/null)"
  dirty="$(git status --short 2>/dev/null)"
  if [ -n "$dirty" ]; then
    printf -- '- **Working tree is dirty**:\n\n```\n%s\n```\n\n' "$dirty"
  else
    printf -- '- Working tree clean.\n\n'
  fi

  printf '## Numbers, each re-derived just now\n\n'
  printf -- '- Mutations: %s\n' "${mutations:-could not be derived}"
  printf -- '- Stale check: %s\n' "${stale:-could not be derived}"
  printf -- '- Links: %s\n' "${links:-could not be derived}"
  [ -n "$index_lines" ] && printf -- '- Generated indexes:\n%s' "$index_lines"
  printf -- '- Tests: **not derived** — a full suite is minutes and a hook has seconds. Run:\n\n'
  printf '  ```bash\n  cargo test 2>&1 | grep -E "^test result" | awk -F'"'"'[ ;]+'"'"' '"'"'{p+=$4; f+=$6; i+=$8} END {print p" passed, "f" failed, "i" ignored"}'"'"'\n  ```\n\n'

  printf '## What was decided\n\n_To be written._\n\n'
  printf '## What is still open\n\n_To be written._\n\n'
  printf '## The concrete next step\n\n_To be written._\n\n'
  printf '## Worth remembering from this stretch\n\n_To be written, then stored._\n\n'
  printf -- '---\n\n'
  printf 'Written automatically before compaction (trigger: %s). The four sections above are the\n' "$trigger"
  printf 'half a script cannot supply; if they are still empty, the close-out did not finish.\n'
} > "$out"

# The instruction half. The script has done what it can; the rest is the model'"'"'s.
/usr/bin/python3 - "$out" "$trigger" <<'PY'
import json, sys
path, trigger = sys.argv[1], sys.argv[2]
print(json.dumps({
  "systemMessage": f"PreCompact ({trigger}): wrote {path} with the re-derived counts. The prose half is Claude's to fill in.",
  "hookSpecificOutput": {
    "hookEventName": "PreCompact",
    "additionalContext": (
      "SESSION CLOSE-OUT, not the default summarisation. A checkpoint stub has been written to "
      f"{path} carrying the git state and every count re-derived by running its command. "
      "Before this context is summarised away, carry the following into whatever survives, and "
      "then finish the stub:\n"
      "1. Fill in the four empty sections of that file — what was decided (with links to the "
      "documents that changed), what is still open, the concrete next step to pick up with, and "
      "what is worth remembering. Do not carry any count forward by hand; the derived ones are "
      "already in the file and the test line names its own command.\n"
      "2. Apply `capture-before-ending`: say out loud what is worth remembering from this "
      "stretch, then store it in the same turn. 'Nothing this time' is a valid answer.\n"
      "3. If any memory set has unexported writes, export it before the next commit, per "
      "`export-before-commit`, and read the export diff.\n"
      "4. Preserve, in the summary itself: the task in progress, the decisions taken this "
      "session and their reasons, and anything measured — a number that has to be re-derived "
      "is worse than one nobody wrote down."
    )
  }
}))
PY
