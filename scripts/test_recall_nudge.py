"""Tests for scripts/recall_nudge.py. Run with `python3 -m unittest scripts/test_recall_nudge.py`.

Each test writes a small session transcript, feeds the hook a payload naming it, and reads what
the hook printed and logged. The transcript lines carry only the fields the hook reads.
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "recall_nudge.py")
REPO = "/work/project"


def tool_use(name, tool_input):
    return {"type": "assistant", "timestamp": "2026-10-06T00:00:00Z",
            "message": {"content": [{"type": "tool_use", "name": name, "input": tool_input}]}}


def habit_recall(**extra):
    call = {"query": "Make the thing do the other thing.", "ignore_floor": True,
            "detail": "summary", "limit": 5}
    call.update(extra)
    return tool_use("mcp__scatterhoard__hoard_recall", call)


def commit():
    return tool_use("Bash", {"command": "git commit -q -m 'Fix the thing'"})


def edit(path=REPO + "/src/store.rs"):
    return {"file_path": path, "old_string": "a", "new_string": "b"}


class Hook:
    """One session: a transcript on disk, a state directory, and a log."""

    def __init__(self):
        self.dir = tempfile.TemporaryDirectory()
        self.transcript = os.path.join(self.dir.name, "session.jsonl")
        self.log = os.path.join(self.dir.name, "nudge.log")
        open(self.transcript, "w").close()

    def append(self, *lines):
        with open(self.transcript, "a") as out:
            for line in lines:
                out.write(json.dumps(line) + "\n")

    def run(self, tool_name="Edit", tool_input=None, event="PreToolUse", raw=None):
        payload = raw if raw is not None else json.dumps({
            "session_id": "s1", "transcript_path": self.transcript, "cwd": REPO,
            "hook_event_name": event, "tool_name": tool_name,
            "tool_input": edit() if tool_input is None else tool_input,
        })
        env = dict(os.environ, SCATTERHOARD_NUDGE_STATE=self.dir.name,
                   SCATTERHOARD_NUDGE_LOG=self.log, CLAUDE_PROJECT_DIR=REPO)
        done = subprocess.run([sys.executable, SCRIPT], input=payload, capture_output=True,
                              text=True, env=env, timeout=20)
        self.status = done.returncode
        return json.loads(done.stdout) if done.stdout.strip() else None

    def decisions(self):
        if not os.path.exists(self.log):
            return []
        with open(self.log) as lines:
            return [json.loads(line)["decision"] for line in lines]


def context(output):
    return output["hookSpecificOutput"]["additionalContext"]


class RecallNudge(unittest.TestCase):
    def setUp(self):
        self.hook = Hook()

    def tearDown(self):
        self.hook.dir.cleanup()

    def test_a_code_edit_with_no_recall_at_all_is_nudged(self):
        output = self.hook.run()
        self.assertIn("hoard_recall", context(output))
        self.assertEqual(output["hookSpecificOutput"]["hookEventName"], "PreToolUse")

    def test_the_nudge_names_the_habits_settings(self):
        text = context(self.hook.run())
        for setting in ("ignore_floor", "summary", "limit"):
            self.assertIn(setting, text)

    def test_a_habit_recall_since_the_last_commit_silences_it(self):
        self.hook.append(commit(), habit_recall())
        self.assertIsNone(self.hook.run())

    def test_a_habit_recall_before_the_last_commit_does_not(self):
        self.hook.append(habit_recall(), commit())
        self.assertIsNotNone(self.hook.run())

    def test_a_scoped_recall_does_not_count_as_the_habit(self):
        self.hook.append(commit(), habit_recall(scope="shared:scatterhoard"))
        self.assertIsNotNone(self.hook.run())

    def test_a_keyword_recall_does_not_count_as_the_habit(self):
        self.hook.append(commit(), tool_use("mcp__scatterhoard__hoard_recall",
                                            {"keywords": ["x"], "ignore_floor": True,
                                             "detail": "summary"}))
        self.assertIsNotNone(self.hook.run())

    def test_a_recall_under_another_server_name_counts(self):
        self.hook.append(commit(), tool_use("mcp__hoard__hoard_recall",
                                            habit_recall()["message"]["content"][0]["input"]))
        self.assertIsNone(self.hook.run())

    def test_it_nudges_once_per_commit(self):
        self.assertIsNotNone(self.hook.run())
        self.assertIsNone(self.hook.run())
        self.assertEqual(self.hook.decisions(), ["nudged", "already-nudged"])

    def test_a_new_commit_lets_it_nudge_again(self):
        self.hook.run()
        self.hook.append(commit())
        self.assertIsNotNone(self.hook.run())

    def test_an_edit_outside_the_code_paths_is_left_alone(self):
        self.assertIsNone(self.hook.run(tool_input=edit(REPO + "/documents/design/index.md")))
        self.assertEqual(self.hook.decisions(), [])

    def test_a_write_to_tests_is_code(self):
        self.assertIsNotNone(self.hook.run("Write", {"file_path": REPO + "/tests/sets.rs",
                                                     "content": "x"}))

    def test_a_shell_write_into_src_is_code(self):
        self.assertIsNotNone(self.hook.run("Bash", {"command": "sed -i '' 's/a/b/' src/mcp.rs"}))

    def test_a_shell_read_of_src_is_not(self):
        self.assertIsNone(self.hook.run("Bash", {"command": "grep -n fn src/mcp.rs"}))

    def test_post_tool_use_is_answered_in_its_own_name(self):
        output = self.hook.run(event="PostToolUse")
        self.assertEqual(output["hookSpecificOutput"]["hookEventName"], "PostToolUse")

    def test_every_decision_on_a_code_write_is_logged(self):
        self.hook.run()
        self.hook.append(habit_recall())
        self.hook.append(commit(), habit_recall())
        self.hook.run()
        self.assertEqual(self.hook.decisions(), ["nudged", "recalled"])

    def test_a_malformed_payload_never_fails_the_tool_call(self):
        self.assertIsNone(self.hook.run(raw="not json"))
        self.assertEqual(self.hook.status, 0)

    def test_a_missing_transcript_never_fails_the_tool_call(self):
        os.remove(self.hook.transcript)
        self.hook.run()
        self.assertEqual(self.hook.status, 0)
        self.assertEqual(self.hook.decisions(), ["no-transcript"])


if __name__ == "__main__":
    unittest.main()
