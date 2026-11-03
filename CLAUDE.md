@AGENTS.md

# Claude Code specifics

Everything above arrives from [AGENTS.md](AGENTS.md), which holds the project's working notes in a form
any coding agent can read. **This file carries only what is specific to Claude Code**, and it is
deliberately short.

## Keep this file small, and know what that costs

**Before adding anything here, ask whether it would still be true under a different coding agent.** If it
would, it belongs in [AGENTS.md](AGENTS.md) instead. `CLAUDE.md` is read by Claude Code and by nothing
else — the documented cross-agent arrangement is exactly the one used here, a one-line import so that a
single copy serves both — so anything general parked here is invisible to a collaborator using another
tool, and invisible in a way nothing reports.

**The reverse mistake has a longer reach, and is the one this note exists for.** A Scatterhoard *user* has
no copy of this repository at all. Nothing in this file or in [AGENTS.md](AGENTS.md) reaches them: what
reaches them is the served convention catalog, and the single line they put in their own `CLAUDE.md`
telling their session to load the conventions and follow them. So a rule that users need is not "written
down" by being written here. It is written down when it is in the catalog.

The live instance: **the `hoard-proj-` carve-out is served by `keyword-standards`**, and must not be
restated as a rule in either file. What [TRACKING.md](TRACKING.md) legitimately carries is this project's own
*instances* of it — the specific mark names its tooling uses — which are facts about this repository and
belong nowhere else. They moved there with the rest of the tracking prompting on 2026-09-08.

## The `PreCompact` hook

**A `PreCompact` hook can write the mechanical half of a checkpoint for you.**
[scripts/precompact-checkpoint.sh](scripts/precompact-checkpoint.sh) writes a dated stub carrying the git
state and every count named under [Session checkpoints](AGENTS.md#session-checkpoints), each re-derived by
running its command, then asks — through the hook's `additionalContext` — for the four sections a script
cannot supply: what was decided, what is still open, the next step, and what is worth remembering.

It runs in about half a second and **deliberately skips `cargo test`**, leaving that line unfilled with its
command beside it, because an absent number a human will fill beats a stale one nobody re-checks. It does
**not** block the compaction: auto-compact fires because the window is full, and refusing it would wedge
the session.

Wire it up per machine in `.claude/settings.local.json` under `hooks.PreCompact` with matchers `auto` and
`manual`; the script is committed and the wiring is not, because the wiring carries an absolute path and a
collaborator will have their own.

## The recall nudge

**A `PreToolUse` hook can catch the per-change recall that [AGENTS.md](AGENTS.md#lessons) asks for and
sessions skip.** [scripts/recall_nudge.py](scripts/recall_nudge.py) watches writes to `src/` and `tests/`,
by the edit tools or by shell commands. If the transcript shows no habit-shaped recall since the last `git
commit`, it adds one line of context asking for it, once per commit. It never blocks, and it logs every
decision on a code write to `~/.local/state/scatterhoard/recall-nudge.log`, so a quiet log can be told
apart from a hook that never ran.

**It is an experiment, started 2026-10-05**, entered in [EARLY-VALIDATION.md](EARLY-VALIDATION.md). Why
it nudges on timing rather than on what the conversation is about is in the script's opening comment.

Wire it per machine in `.claude/settings.local.json` under `hooks.PreToolUse`, with matcher
`Edit|Write|MultiEdit|Bash` and the script's absolute path as the command. **`PreToolUse` context reaches
the session**, confirmed on 2026-10-06. The Claude Desktop interface may not show the nudge, so read the
log to see it fire. Its tests run with `python3 -m unittest scripts/test_recall_nudge.py`.
