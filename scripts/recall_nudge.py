#!/usr/bin/env python3
"""Nudge a session to recall before it changes code, when it has not since the last commit.

Wired to Claude Code's PreToolUse hook (PostToolUse also works, one edit late). On a write to
`src/` or `tests/`, it reads the session's transcript, and if no habit-shaped recall -- the call
AGENTS.md's Lessons section prescribes: a `query`, no `scope`, `ignore_floor`, `detail: summary` --
has been made since the last `git commit`, it adds one line of context asking for it. Once per
commit, so it does not nag.

**Why timing and not meaning.** A content signal read from the recent conversation was tested
offline on 2026-10-05 and could not tell moments that needed a recall from moments that did not.
This rule, replayed over the week before, reached all 20 measurable skipped recalls with 24 nudges.
Both results are in the `scatterhoard` set under `gut-feeling-recall`.

**Why only the habit's shape silences it.** Sessions recall for bookkeeping too, and when any recall
counted, the replay caught 11 skips instead of 20.

**It never blocks, and it logs every decision on a code write**, silent ones included, so that a
quiet week can be told apart from a hook that never ran. The log is JSON lines at
`$SCATTERHOARD_NUDGE_LOG`, by default `~/.local/state/scatterhoard/recall-nudge.log`.
"""

import json
import os
import sys
import tempfile
from datetime import datetime, timezone

CODE = ("src/", "tests/")
SHELL_WRITES = ("sed -i", "open(", "write(", ".replace(", "cat >", "tee ", "> src", ">src",
                "> tests", ">tests", "apply_patch", "perl -i")

NUDGE = (
    "Recall nudge: this is a code change, and no habit recall has been made since the last "
    "commit. Before writing it, write one sentence saying what you are about to write, and "
    "recall it once: `hoard_recall` with that sentence as `query`, no `scope`, "
    "`ignore_floor: true`, `detail: \"summary\"` and `limit: 5`, as the Lessons section of "
    "AGENTS.md says. Where a returned line is a lesson or a trigger for what you are about to "
    "do, look it up with `hoard_lookup` and write accordingly. If the change is not one that "
    "adds or alters behaviour, say so and carry on. This fires once per commit."
)


def is_code_write(tool_name, tool_input, project):
    if tool_name in ("Edit", "Write", "MultiEdit", "NotebookEdit"):
        path = tool_input.get("file_path") or tool_input.get("notebook_path") or ""
        relative = os.path.relpath(path, project) if os.path.isabs(path) else path
        return relative.startswith(CODE)
    if tool_name == "Bash":
        command = tool_input.get("command", "")
        return any(code in command for code in CODE) and any(w in command for w in SHELL_WRITES)
    return False


def is_habit_recall(name, tool_input):
    # By the tool's own name, not the server's: the server is named per machine.
    return (name.endswith("__hoard_recall") and bool(tool_input.get("query"))
            and not tool_input.get("scope") and tool_input.get("ignore_floor") is True
            and tool_input.get("detail") == "summary")


def since_last_commit(transcript):
    """How many commits the session has made, and whether a habit recall followed the last."""
    commits, recalled = 0, False
    with open(transcript) as lines:
        for line in lines:
            try:
                content = (json.loads(line).get("message") or {}).get("content")
            except ValueError:
                continue
            if not isinstance(content, list):
                continue
            for block in content:
                if not isinstance(block, dict) or block.get("type") != "tool_use":
                    continue
                name, tool_input = block.get("name", ""), block.get("input") or {}
                if name == "Bash" and "git commit" in tool_input.get("command", ""):
                    commits, recalled = commits + 1, False
                elif is_habit_recall(name, tool_input):
                    recalled = True
    return commits, recalled


def log(decision, payload):
    path = os.environ.get("SCATTERHOARD_NUDGE_LOG") or os.path.expanduser(
        "~/.local/state/scatterhoard/recall-nudge.log")
    try:
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "a") as out:
            out.write(json.dumps({
                "at": datetime.now(timezone.utc).isoformat(timespec="seconds"),
                "session": payload.get("session_id"), "event": payload.get("hook_event_name"),
                "tool": payload.get("tool_name"), "decision": decision}) + "\n")
    except OSError:
        pass


def decide(payload):
    project = os.environ.get("CLAUDE_PROJECT_DIR") or payload.get("cwd") or os.getcwd()
    if not is_code_write(payload.get("tool_name", ""), payload.get("tool_input") or {}, project):
        return None
    transcript = payload.get("transcript_path") or ""
    if not os.path.exists(transcript):
        log("no-transcript", payload)
        return None
    commits, recalled = since_last_commit(transcript)
    if recalled:
        log("recalled", payload)
        return None

    # Once per commit: remember which commit this session was last nudged after.
    state_dir = os.environ.get("SCATTERHOARD_NUDGE_STATE") or os.path.join(
        tempfile.gettempdir(), "scatterhoard-recall-nudge")
    os.makedirs(state_dir, exist_ok=True)
    state = os.path.join(state_dir, str(payload.get("session_id", "unknown")))
    marker = f"after-commit-{commits}"
    if os.path.exists(state):
        with open(state) as held:
            if held.read() == marker:
                log("already-nudged", payload)
                return None
    with open(state, "w") as out:
        out.write(marker)
    log("nudged", payload)
    return {
        "systemMessage": "Recall nudge: a code change with no habit recall since the last commit.",
        "hookSpecificOutput": {
            "hookEventName": payload.get("hook_event_name", "PreToolUse"),
            "additionalContext": NUDGE,
        },
    }


def main():
    try:
        output = decide(json.loads(sys.stdin.read()))
    except Exception:  # A hook that fails must not fail the edit it is watching.
        return 0
    if output:
        print(json.dumps(output))
    return 0


if __name__ == "__main__":
    sys.exit(main())
