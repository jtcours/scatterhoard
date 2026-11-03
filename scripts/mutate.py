#!/usr/bin/env python3
"""Run recorded mutations against the test suite and report which ones the tests catch.

Mutation testing has repeatedly been worth it here -- it has found tests that asserted only that
construction failed rather than *how*, and guards that no test exercised. But running it as ad-hoc
shell one-liners produced a false negative twice in one session, from two different causes:

  * a mutation that did not compile (no test ran, and grepping for failures found none)
  * a `timeout` command that does not exist on macOS (the whole command failed to start)

Both read as "no test failed", which is indistinguishable from "the mutation survived" if all you
do is look for failure lines. This script exists to make those outcomes distinguishable, and to
keep the mutations themselves -- which are the valuable part -- as a reviewable artifact that can
be re-run after the code changes, rather than scrollback that is gone next session.

Usage:
    python3 scripts/mutate.py                 # run every mutation in scripts/mutations.py
    python3 scripts/mutate.py --only cycle    # run those whose name contains "cycle"
    python3 scripts/mutate.py --start-at 55   # resume from the 55th, numbered as --list shows
    python3 scripts/mutate.py --start-at wal  # or from the first whose name contains "wal"
    python3 scripts/mutate.py --list
    python3 scripts/mutate.py --restore       # recover from a run that was killed

Exit status is non-zero if any mutation's outcome disagrees with what scripts/mutations.py expects.

# How long a pass takes, and why this file does not tell you

A full pass rebuilds once per mutation, so its duration is dominated by build and process-launch
cost -- which depends on the machine, the filesystem, and above all on how far the `deps` directory
under `target-mutation/` has silted up. **No figure quoted here would stay true**, and one previously did not: this docstring
claimed "one to two hours" for months after that stopped being the healthy case.

What is worth recording is the *ratio*, because it is what tells you the number is not a property of
the work. The same 88-mutation pass, same machine, same mutations, measured about 129 minutes and
about 2 minutes; the only thing that differed was the state of the build directory. The finding and
its controls are with the budget constants below, where they justify the thresholds -- not repeated
here, since a second copy is how the first claim went stale.

So: **establish your own baseline on your own machine and compare against it**, rather than against
anything written down. Every line carries the wall-clock time it was printed and how long that
mutation took, which is what a baseline is made of. Two adjacent timestamps are the cadence, and a
stalled run is one whose last line is old rather than one that merely feels slow.

**Every run clears the build directory itself**, before its first build and every
`CLEAR_CHECK_EVERY` mutations after that, whenever it holds more than `CROWDED` files -- a whole
pass since 2026-09-20, and `--only` and `--start-at` runs since 2026-09-29. A silted directory makes
every freshly linked binary launch slowly, which looks exactly like slow tests; clearing it costs a
from-scratch build of about fifteen seconds, paid roughly once per 190 mutations' worth of runs.

**There is no end-of-run crowding warning any more**, and that is deliberate
(`the-mutation-runners-crowding-note-fires-on-every-pass`). It needed a mean pace above an absolute
`SLOW_MUTATION` *and* a crowded `deps`; the suite outgrew the pace constant, so the conjunction
collapsed into the file count alone, and once every run clears, a crowded directory at the end of
one is only ever the last stretch before the next check -- a reading that cannot vary tells the
reader nothing. Pace is watched relative to the run instead: a step far slower than its neighbours
is photographed while it is still running.

**Turning incremental compilation off was tried first and is the wrong fix**, which is recorded here
because it is the obvious one and looks right on paper. Each rebuild leaves 512 `.rcgu.o` codegen
units behind with it on and none with it off, so it does stop the silt exactly -- but the same 88
mutations from a cleared directory took 535s with it off against 340s with it on. A pass is almost
entirely rebuilds, and the ~1.7s each one loses to a cold build swamps what the files cost.

To check by hand anyway, `ls target-mutation/debug/deps | wc -l`. A silted build directory looks
exactly like a slow suite, so rule it out before concluding anything about the tests.

**A pass builds into `target-mutation/`, never into `target/`.** That is what stops a pass from
publishing a server built from mutated source to the path every MCP client spawns from, and it is
also why the directory that wants clearing is never the one the servers depend on. See
`MUTATION_TARGET` below.

Output is flushed per line, which matters more than it looks: Python block-buffers stdout when it
is redirected to a file, so without the flush a redirected run shows nothing at all for the first
hour and then everything at once -- exactly the shape that makes a healthy run look hung.

`--start-at` is for the other half of that problem. A pass interrupted at the 54th should not have
to redo the first 53 to check the rest.

# Surviving a hard kill

Holding the pristine text in memory and writing it back in a `finally` is enough for an ordinary
error, and not enough for `kill -9` or a closed laptop: the only copy dies with the process, and
the mutated file is left on disk looking like ordinary source. That happened on 2026-08-03 and left
a centroid guard silently stripped out of `src/embedding.rs`.

So the pristine copy goes to disk first, under `.mutation-archive/`, mirroring the file's path from
the repo root so the original location is recoverable from the archive alone. Restoring *moves* it
back rather than copying, which means the archive is empty exactly when no mutation is in flight.
A non-empty archive at startup is therefore proof that a previous run died, and this script refuses
to do anything until that is sorted out -- because the alternative is overwriting the pristine copy
with an already-mutated file and losing the original for good.

The mutated file also gets a marker comment at the top. That is belt-and-braces: it makes an
abandoned mutation obvious in a diff, an editor, or a code review, without needing this script.
"""

from __future__ import annotations

import argparse
import atexit
import os
import pathlib
import shutil
import signal
import subprocess
import sys
import statistics
import time
from collections import deque
from dataclasses import dataclass, field

ROOT = pathlib.Path(__file__).resolve().parent.parent

#: Where pristine copies live while their originals are mutated. Empty when nothing is in flight.
ARCHIVE = ROOT / ".mutation-archive"

#: Where a pass builds. **Deliberately not `target/`**, and this is the one line keeping a pass from
#: publishing a broken server: every MCP client entry -- here, a project `.mcp.json`, the Claude
#: Desktop config -- names `target/debug/scatterhoard` by absolute path, and a pass relinks the
#: binary it builds once per mutation. Sharing the directory meant that for the length of a pass any
#: client that spawned got a server with a guard deliberately removed, aimed at whichever one that
#: mutation took out, running against the operator's real sets. The end-of-pass rebuild below closed
#: the *end state* and could not close the window, which is measured in tens of minutes.
#:
#: A sibling of `target/` rather than a child, so the two clean independently: a pass leaves roughly
#: 35,000 files behind and this is where they land, so `rm -rf target-mutation` is free at any time
#: and never touches what the servers spawn from.
MUTATION_TARGET = ROOT / "target-mutation"

#: The environment every cargo invocation here runs under. Set once, at the one place commands are
#: launched, because a second launcher that forgot it would reopen the hole silently.
#: **Incremental compilation is left on, and that was measured rather than assumed.** Turning it off
#: does stop the silt exactly -- a rebuild adds 512 `.rcgu.o` codegen units with it on and none with
#: it off, over four cycles -- but it costs more than the silt does: the same 88 mutations from a
#: cleared directory ran 535s with it off and 340s with it on, 2026-09-20. A mutation pass is almost
#: entirely rebuilds, so the ~1.7s each one loses swamps the launch penalty the files cause. The silt
#: is handled by clearing instead, below.
ENVIRONMENT = os.environ | {"CARGO_TARGET_DIR": str(MUTATION_TARGET)}

#: Prepended to a mutated file so an abandoned one is obvious without running anything.
MARKER = (
    "// MUTATED by scripts/mutate.py -- this is not the real source.\n"
    "// If you are reading this in your working tree, a mutation run was killed:\n"
    "// run `python3 scripts/mutate.py --restore` to put the original back.\n"
)

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from mutations import MUTATIONS  # noqa: E402  (needs the path set above)

# A hang is a real outcome, not a hazard to be avoided: recall traverses a graph that may contain
# cycles, so a mutation removing a termination guard *should* hang.
#
# Tests and builds get separate budgets, because they are bounded for opposite reasons. The build
# budget is not a detector at all -- a mutation cannot make the compiler loop -- it is only there so
# a wedged process cannot stall the run forever. The test budget *is* a hang detector.
#
# The test budget was 180s, on the reasoning that the whole suite runs in under three seconds so
# anything near the limit is pathological. Two mutations ran over it anyway, and the reason had
# nothing to do with the tests, the build, or the code under test.
#
# **The first execution of a freshly linked binary cost 33-45 seconds; the second cost 0.0.** Not a
# property of the binary: an identical copy of the same bytes launched in 0.21s from an empty
# directory. What it tracked was the *contents of the directory the binary sat in*. `target/debug/
# deps` had accumulated 877,420 files and 3.9 GB of stale `.rcgu.o` codegen units, which cargo never
# garbage-collects. Controls, all with the same binary: an empty directory inside `target/`, 0.21s;
# a directory holding 877,000 *empty* files, 4.02s; `deps` itself, 38.23s. So a few seconds of it is
# the file count and the rest tracks the bytes.
#
# `cargo clean` removed 886,099 files and 31.1 GiB. First launch went to 0.3s, a full `cargo test`
# after a source change went from roughly ten minutes to eleven seconds, and a single-binary
# mutation from about fifty seconds to one.
#
# The mechanism is not identified and is deliberately not guessed at here: two confident
# attributions to malware scanners were made from CPU samples and both were wrong, and neither
# excluding the build tree from the third-party scanner nor granting the parent app Developer Tools
# rights changed the number at all. What is established is the correlation, by varying one thing at
# a time.
#
# So the budget covers process launch, not just test execution, and it has to tolerate a target
# directory that has begun to silt up again -- which it will, since nothing prunes it. Five minutes
# is far beyond a healthy machine's needs and still catches a genuine hang promptly. **If mutations
# start taking minutes each, run `cargo clean` before believing anything about the code.**
TIMEOUT_SECONDS = 300
BUILD_TIMEOUT_SECONDS = 900

#: How long to wait for a killed process group's output after the kill.
#:
#: Small on purpose. Everything holding the pipe has just been sent `SIGKILL`, so this is the time
#: the operating system needs rather than the time any program needs, and the alternative to giving
#: up is the unbounded wait that made a hang unreportable in the first place.
KILL_GRACE_SECONDS = 10

CAUGHT = {"failed", "compile_error", "timeout"}

#: Outcomes that count as caught but prove nothing about the *test suite*.
#:
#: **A mutation that will not compile has not been noticed by a test.** The compiler stops it, which
#: is a real answer where the type system is genuinely the guard being proved -- but it is almost
#: never what the author meant, and it is indistinguishable in a summary from a mutation a test
#: caught by name. Seven of 243 were found this way on 2026-08-26, every one of them a broken
#: record rather than a deliberate one: replacement text naming a function that never existed, and
#: replacements that stopped compiling when a struct they construct grew a field.
#:
#: **`--stale` structurally cannot see this.** It checks that a mutation's `find` text still matches
#: its file, and for all seven it did. What had rotted was the `replace` text, which nothing looks
#: at until it is applied. So the only instrument for this class is running the corpus and reading
#: what came back -- which is what this set exists to make legible.
#:
#: So an outcome here is a **disagreement** unless the record asked for it by name, with
#: `expect = "compiler"`. That is the opt-in for the case where the type system really is the guard.
BY_THE_COMPILER = {"compile_error"}

#: How many failing test names a report lists before summarising the rest. Enough to show that the
#: test written for a mutation is among the catchers, which is the judgement the list exists for,
#: without letting one line run away when a mutation trips a whole suite.
LISTED_FAILURES = 8

#: Outcomes that are neither "caught" nor "survived" but "we learned nothing". They always count as
#: a disagreement, whatever the mutation expected, because the alternative is reporting a run that
#: never happened as a result.
INCONCLUSIVE = {"no_result", "blocked", "build_timeout"}

#: What cargo prints while another cargo process holds the build directory. An editor running
#: `cargo check` in the background is enough, and it is why this is worth detecting rather than
#: assuming: a blocked run reads as a hang, and a hang is classified as *caught*, so lock contention
#: silently turns every `expect = "caught"` mutation into a pass that proved nothing.
BLOCKED = "Blocking waiting for file lock"

#: The `deps` directory where cargo drops codegen-unit objects and never collects them. Watched
#: because letting it silt up cost this project two hours per full pass and nobody noticed the
#: cause for a week -- see the note on the budgets above. Under `MUTATION_TARGET`, since that is
#: where a pass builds and therefore the only one a pass silts up.
DEPS = MUTATION_TARGET / "debug" / "deps"

#: File count in `deps` above which a run clears the build directory.
#:
#: **Where clearing starts to pay, not how big a pass is.** Measured: 45,000 files ran at clean pace
#: (2026-09-20), and 159,000 ran at twice it (2026-08-09), so the cost appears between them and this
#: sits in that range. Too low costs a fifteen-second rebuild more often; too high lets every
#: mutation launch slowly for a while. Neither is a correctness problem, so this can be roughly right.
#:
#: **If it needs recalibrating**, measure pace against file count, not the size of a pass: a pass
#: grows with the corpus, and a threshold tied to it drifted into uselessness once already. The
#: longer-term replacement is a baseline derived from the directory itself, with no number to drift
#: -- recorded under `the-mutation-runners-crowding-note-fires-on-every-pass`.
CROWDED = 100_000

#: How often a whole pass re-counts the directory. A `readdir` over 100,000 entries costs about
#: 50ms, which is nothing occasionally and 25 seconds if asked once per mutation.
CLEAR_CHECK_EVERY = 25


@dataclass
class Outcome:
    kind: str
    detail: str = ""
    #: What else was running when a command was killed, or when it ran far longer than its
    #: neighbours. Empty unless one of those happened.
    diagnosis: str = ""


#: How many times its neighbours' median a step may take before the machine is photographed.
#:
#: Added 2026-09-28, after a full pass averaged 4.9s per mutation against 2-5s for the same
#: mutations run alone, with three entries of 26-36s that took under 4s when repeated. Nothing in the
#: log could say what the machine was doing, because a snapshot was taken only at a timeout, five
#: minutes in. **A snapshot is a record, not a verdict**: it changes no outcome and counts nothing.
SLOW_FACTOR = 4.0

#: The least a step must take before it counts as slow, whatever its neighbours took. Well above a
#: healthy step, measured 2026-09-28 at 1.3-2.4s to build and 0.4-2.6s to test, so ordinary jitter
#: in a fast phase photographs nothing.
SLOW_FLOOR_SECONDS = 6.0

#: Samples of a phase needed before its neighbours are trusted to say what slow is. Below this no
#: snapshot is taken at all: the first build of a pass can be cold, and a cold build is slow for a
#: reason already known.
SLOW_MIN_SAMPLES = 5

#: How many of a phase's most recent durations make up its neighbours.
SLOW_WINDOW = 25


@dataclass
class Pace:
    """How long each phase of a mutation has recently taken, to say when one is running long.

    Kept per phase because a build and a test run differ in cost and in what slows them down, and
    by *median* so the stalls this exists to photograph do not raise the bar for the next one.
    """

    recent: dict[str, deque] = field(default_factory=dict)

    def slow_after(self, phase: str) -> float | None:
        """Seconds after which this phase is worth a snapshot, or `None` while too few are known."""
        durations = self.recent.get(phase)
        if durations is None or len(durations) < SLOW_MIN_SAMPLES:
            return None
        return max(SLOW_FLOOR_SECONDS, SLOW_FACTOR * statistics.median(durations))

    def record(self, phase: str, seconds: float) -> None:
        self.recent.setdefault(phase, deque(maxlen=SLOW_WINDOW)).append(seconds)


def say(line: str) -> None:
    """Print a line and push it out now.

    The flush is the whole reason this exists. Redirected stdout is block-buffered, so a long run
    writes nothing for an hour and then everything -- which is indistinguishable from a hang by the
    only means available to someone watching the file.
    """
    print(line, flush=True)


def stamped(line: str, seconds: float | None = None) -> str:
    """Prefix a line with the time it was printed, and how long its mutation took."""
    elapsed = "" if seconds is None else f"{int(seconds) // 60:d}m{int(seconds) % 60:02d}s"
    return f"{time.strftime('%H:%M:%S')} {elapsed:>7s}  {line}"


def clear_if_crowded() -> None:
    """Count the build directory and clear it if it has silted past `CROWDED`.

    One helper for the start of a pass and for the checks during it, because two copies of *count,
    compare, remove* would be free to disagree about the threshold -- and the one that got it wrong
    would simply be slow, which is the symptom nobody attributes to a rule.
    """
    try:
        count = len(os.listdir(DEPS))
    except OSError:
        return
    if not should_clear(count):
        return
    say(
        f"clearing {MUTATION_TARGET.relative_to(ROOT)}: {count:,} files, above {CROWDED:,}. A "
        f"freshly linked binary launches about nine times slower from a crowded directory and "
        f"every mutation links one, so this costs about fifteen seconds now and saves more than "
        f"that before the pass ends."
    )
    shutil.rmtree(MUTATION_TARGET, ignore_errors=True)


def should_clear(count: int) -> bool:
    """Whether to clear the build directory before the next build.

    **On every run, `--only` included**, since 2026-09-29. It used to be a whole pass only, because
    `--only` exists to be fast and a from-scratch build costs about fifteen seconds. But this fires
    only above `CROWDED`, which at about 520 files per mutation is roughly once per 190 mutations'
    worth of runs, while a crowded directory slows every mutation of every run: for a run of twenty
    or thirty, the occasional fifteen seconds is the fast choice.

    **At `CROWDED`**, which is right on the evidence. Measured 2026-09-20, an 88-mutation pass took a
    directory from empty to 45,000 files with no degradation in pace at all -- 4.55s over the first
    eighth, which is the from-scratch build, then 2.6s to 3.6s with no upward trend. The 2026-08-09
    investigation puts a pass at 159,000 files running at twice clean speed, and 543,000 files cost
    nine times on launch. So the cost appears somewhere above 45,000 and well below 159,000.

    **Checked during a run as well as before it**, because with incremental compilation on the
    directory silts as the run goes: 512 files per mutation, so a 500-mutation pass crosses this
    around its middle and the back half would pay for the front half's builds.
    """
    return count > CROWDED


def archived_for(path: pathlib.Path) -> pathlib.Path:
    """Where this file's pristine copy lives, mirroring its path from the repo root."""
    return ARCHIVE / path.relative_to(ROOT)


def abandoned() -> list[pathlib.Path]:
    """Pristine copies left behind by a run that did not finish."""
    if not ARCHIVE.exists():
        return []
    return sorted(path for path in ARCHIVE.rglob("*") if path.is_file())


def put_back(archived: pathlib.Path) -> pathlib.Path:
    """Move a pristine copy back over its original. The move is what empties the archive."""
    original = ROOT / archived.relative_to(ARCHIVE)
    original.parent.mkdir(parents=True, exist_ok=True)
    archived.replace(original)
    # Mark it modified *now*. The archived copy carries the original's timestamp, so restoring it
    # leaves the source looking older than the artifacts cargo just built from the mutated version
    # -- and cargo, which decides by timestamp, then skips the rebuild. The next `cargo test` after
    # a run would silently exercise mutated code and report whatever that produces, in either
    # direction. Found the hard way: five tests "failed" against source that was already correct.
    original.touch()
    for directory in archived.parents:
        if directory == ARCHIVE or ARCHIVE not in directory.parents:
            break
        if not any(directory.iterdir()):
            directory.rmdir()
    return original


def run(
    command: list[str], budget: int = TIMEOUT_SECONDS, slow_after: float | None = None
) -> tuple[int | None, str, str]:
    """Run a command, returning (exit status, output, diagnosis). Status is `None` on timeout.

    **A command still running at `slow_after` is photographed and left to finish**, within the same
    budget. The snapshot has to be taken during the stall, not after: by the time a slow step
    returns, whatever was in its way has usually gone too. Retrying `communicate` after its timeout
    loses no output, which is what makes waiting in two stages safe.

    Two things this does that `subprocess.run(timeout=...)` cannot, both learned from one timeout
    nobody could explain afterwards.

    **The output of a killed command is kept.** What it managed to say before running out of time is
    the only evidence for *why*, and discarding it is what made that timeout unexplainable.

    **The machine is photographed while the command is still hung.** `subprocess.run` kills the
    child before raising, so by the time a caller learns of the timeout the interesting state is
    already gone -- including whatever the child was waiting *for*. Driving `Popen` directly means
    the snapshot happens first and the kill second, which is the difference between "something was
    in the way" as a hypothesis and as a fact.

    **The whole process group is killed, not just the command.** `cargo test` is a parent: it
    launches the test binaries, and they inherit this pipe. Killing cargo alone leaves a spinning
    test binary holding the write end open, so the `communicate()` that follows blocks forever --
    the timeout fires, the budget is ignored, and the runner waits on a pipe nothing will ever
    close. **The timeout classification this script documents was unreachable because of it**: a
    mutation that hangs the tests hung the runner instead of being reported as caught. Observed
    2026-08-26 with a deliberately non-terminating mutation, and the four-hour wedged pass recorded
    in the `coding-lessons` set on 2026-08-18 was the same bug read as a problem with the test
    helpers. `start_new_session` makes this command a group leader so one `killpg` reaches every
    descendant.
    """
    process = subprocess.Popen(  # noqa: S603
        command,
        cwd=ROOT,
        env=ENVIRONMENT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        start_new_session=True,
    )
    started = time.monotonic()
    # Read at the start so a snapshot can say what changed *during* this step: the counters are
    # totals since boot, and a total cannot tell a stall from swapping that happened hours ago.
    baseline = vm_counters()
    noted = ""
    if slow_after is not None and slow_after < budget:
        try:
            output = process.communicate(timeout=slow_after)[0]
            return process.returncode, output or "", ""
        except subprocess.TimeoutExpired:
            noted = (
                f"  `{' '.join(command[:2])}` still running after {slow_after:.1f}s, far past "
                f"its neighbours; the machine at that moment:\n{machine_snapshot(baseline)}"
            )
    try:
        remaining = max(budget - (time.monotonic() - started), 0.1)
        output = process.communicate(timeout=remaining)[0]
        return process.returncode, output or "", noted
    except subprocess.TimeoutExpired:
        # Order matters: look before killing.
        diagnosis = "\n".join(part for part in (noted, machine_snapshot(baseline)) if part)
        kill_group(process)
        try:
            # Bounded, because the whole failure this replaced was an unbounded wait on a pipe. If
            # something still holds it after the group has been killed, losing the output is better
            # than losing the pass.
            output = process.communicate(timeout=KILL_GRACE_SECONDS)[0] or ""
        except subprocess.TimeoutExpired:
            output = ""
        return None, output, diagnosis


def kill_group(process: subprocess.Popen) -> None:
    """Kill a command and everything it started.

    `SIGKILL` rather than `SIGTERM`: what is being killed here is either a hung test binary or a
    build that overran, and neither has cleanup worth waiting for. A terminate-then-kill dance would
    reintroduce the wait this exists to remove.
    """
    try:
        os.killpg(os.getpgid(process.pid), signal.SIGKILL)
    except (ProcessLookupError, PermissionError):
        # Already gone, or not ours to signal. Fall back to the single process so the caller is
        # never left with a live child because the group call was refused.
        process.kill()


#: The `vm_stat` counters a snapshot reports the change in: paging to and from disk, and swapping.
VM_COUNTERS = ("Pageins", "Pageouts", "Swapins", "Swapouts")


def vm_counters() -> dict[str, int]:
    """The machine's paging and swapping totals since boot, or nothing where `vm_stat` is absent.

    macOS only, and cheap -- a few milliseconds -- so it is read at the start of every step rather
    than only when one is slow, which is the only way a snapshot can report a *change*.
    """
    try:
        finished = subprocess.run(  # noqa: S603
            ["vm_stat"], capture_output=True, text=True, timeout=5, check=False
        )
    except (OSError, subprocess.SubprocessError):
        return {}
    counters = {}
    for line in finished.stdout.splitlines():
        name, _, value = line.partition(":")
        if name.strip() in VM_COUNTERS:
            try:
                counters[name.strip()] = int(value.strip().rstrip("."))
            except ValueError:
                pass
    return counters


def memory_since(baseline: dict[str, int]) -> list[str]:
    """How much paging and swapping happened since `baseline`, as report lines.

    Added 2026-09-28 to tell a stall caused by memory pressure from one caused by anything else.
    The totals alone cannot: this machine had 48,592 swap-outs since boot on a quiet afternoon.
    """
    now = vm_counters()
    if not baseline or not now:
        return ["  memory: paging counters unavailable"]
    changed = ", ".join(
        f"{name.lower()} +{now[name] - baseline[name]}"
        for name in VM_COUNTERS
        if name in now and name in baseline
    )
    return [f"  memory: during this step {changed}"]


def machine_snapshot(baseline: dict[str, int] | None = None) -> str:
    """What else the machine was doing, captured at the moment something failed to finish.

    Written because a timeout was explained away as lock contention from an editor, and the
    explanation did not survive the machine's owner saying the machine had been idle. Neither the
    hypothesis nor its refutation had any evidence behind it, which is the part worth fixing:
    a guess about the environment is checkable, and only at the moment it happens.

    **Memory as well as CPU, since 2026-09-28**: a process consuming memory can barely register on
    CPU while the machine swaps, so the busiest list alone cannot rule swapping out.
    """
    sections = (
        ("load", ["uptime"]),
        ("rust processes", ["pgrep", "-fl", "cargo|rustc|rust-analyzer|mutate.py"]),
        ("busiest", ["ps", "-Ao", "pid,ppid,%cpu,%mem,etime,comm", "-r"]),
        ("largest", ["ps", "-Ao", "pid,rss,%mem,comm", "-m"]),
        ("swap", ["sysctl", "vm.swapusage"]),
        ("pressure", ["memory_pressure"]),
    )
    lines = memory_since(baseline) if baseline is not None else []
    for label, command in sections:
        try:
            finished = subprocess.run(  # noqa: S603
                command, capture_output=True, text=True, timeout=15, check=False
            )
        except (OSError, subprocess.SubprocessError) as error:
            lines.append(f"  {label}: unavailable ({error})")
            continue
        # Bounded, because this goes in the report and a full process table would bury the finding
        # it exists to surface.
        found = [line.rstrip() for line in finished.stdout.splitlines() if line.strip()]
        # `memory_pressure` ends with the one line worth keeping: the free percentage.
        found = found[-1:] if label == "pressure" else found[:8]
        lines.append(f"  {label}:")
        lines.extend(f"    {line[:160]}" for line in found or ["(nothing)"])
    return "\n".join(lines)


def last_words(output: str) -> str:
    """What a killed command had most recently said, for the report line.

    A timeout is the one outcome that says nothing about *why* on its own, and the difference
    between causes is the difference between a real finding and a busy machine. `cargo test` prints
    `running N tests` and then a line per test as each finishes, so the tail distinguishes a run
    that never started, one that stalled partway, and one that was still compiling.
    """
    lines = [line.strip() for line in output.splitlines() if line.strip()]
    if not lines:
        return "it had produced no output at all"
    tail = " | ".join(lines[-3:])
    return f"last output: {tail[:300]}"


def _decode(stream: str | bytes | None) -> str:
    """`TimeoutExpired` carries whatever was captured, which may be bytes or nothing at all."""
    if stream is None:
        return ""
    if isinstance(stream, bytes):
        return stream.decode("utf-8", errors="replace")
    return stream


def evaluate(targets: list[str], pace: Pace | None = None) -> Outcome:
    """Build, then test, distinguishing every way this can go wrong.

    A step that ran far longer than its neighbours leaves its snapshot on the outcome, whatever the
    outcome was: a slow step that then succeeded is the case the runner could never explain.
    """
    pace = pace if pace is not None else Pace()
    stalls: list[str] = []
    outcome = _evaluate(targets, pace, stalls)
    if stalls and not outcome.diagnosis:
        outcome.diagnosis = "\n".join(stalls)
    return outcome


def _timed(phase: str, command: list[str], budget: int, pace: Pace, stalls: list[str]):
    """Run one phase, recording how long a completed run took and any stall it was photographed in."""
    started = time.monotonic()
    status, output, diagnosis = run(command, budget, pace.slow_after(phase))
    if status is not None:
        pace.record(phase, time.monotonic() - started)
        if diagnosis:
            stalls.append(diagnosis)
    return status, output, diagnosis


def _evaluate(targets: list[str], pace: Pace, stalls: list[str]) -> Outcome:
    # Build only the test binaries this mutation is checked against. Building all of them costs
    # more than the run it protects -- and it protects nothing extra, because mutations edit `src/`,
    # so a compile error reaches the library and is caught whichever test target is built.
    build = ["cargo", "build", "--quiet"]
    for target in targets:
        build += ["--test", target]
    if not targets:
        build.append("--tests")

    status, output, diagnosis = _timed("build", build, BUILD_TIMEOUT_SECONDS, pace, stalls)
    if status is None:
        if BLOCKED in output:
            return Outcome(
                "blocked",
                "another cargo process held the build lock; nothing was built",
                diagnosis,
            )
        return Outcome(
            "build_timeout", f"the build did not finish; {last_words(output)}", diagnosis
        )
        # Inconclusive rather than caught, unlike a test timeout. A mutation can legitimately make
        # the *tests* hang -- that is what removing a termination guard does, and it is a real
        # result. It cannot make the *compiler* hang: a build either finishes or the machine is
        # busy. Classifying this as caught would turn a loaded machine into a pass that proved
        # nothing, which is the failure this whole script was written for.
        return Outcome("build_timeout", "the build did not finish; nothing was tested")
    if status != 0:
        first = next(
            (line for line in output.splitlines() if line.startswith("error")), "build failed"
        )
        return Outcome("compile_error", first.strip())

    # Not --quiet: quiet mode prints progress dots instead of test names, and the names are
    # the whole point of the report.
    #
    # `--no-fail-fast` because cargo otherwise stops after the first test binary that fails, and a
    # mutation is usually meant to be caught by a *particular* test. Without it the report can name
    # a coincidental catch in an early binary while the intended guard, in a later one, never ran at
    # all -- and "caught by something else" and "the real guard never executed" look identical from
    # the outside. That is the trap this file already records for the `Keyword` mutation, where a
    # catch by a `MemoryText` test would have read as proof of a guard nothing exercised.
    #
    # Added 2026-08-12 after the report for a `scope` mutation listed four `mcp` tests and omitted
    # the two `memory_model` ones written for it, which had to be confirmed by hand. Same shape as
    # the `coding-lessons` set, *a verification harness must distinguish "nothing failed" from
    # "nothing ran"* --
    # one level in, where the missing run hides inside a result that says `caught`.
    command = ["cargo", "test", "--no-fail-fast"]
    for target in targets:
        command += ["--test", target]

    status, output, diagnosis = _timed("test", command, TIMEOUT_SECONDS, pace, stalls)
    if status is None:
        if BLOCKED in output:
            return Outcome(
                "blocked", "another cargo process held the build lock; no test ran", diagnosis
            )
        return Outcome(
            "timeout",
            f"tests did not finish -- a termination guard may be gone; {last_words(output)}",
            diagnosis,
        )

    # The check the ad-hoc version lacked: confirm the harness actually reported results, so a run
    # that never produced any cannot be mistaken for a clean one.
    if "test result:" not in output:
        return Outcome("no_result", "cargo test produced no results at all")

    if status != 0:
        failures = [
            line.strip().removeprefix("    ")
            for line in output.splitlines()
            if line.strip().startswith("test ") and line.rstrip().endswith("FAILED")
        ]
        named = [line.split()[1] for line in failures if len(line.split()) > 1]
        if not named:
            named = [
                line.strip()
                for line in output.splitlines()
                if line.strip() and not line.startswith(("running", "test result"))
            ][:1]

        # Sorted and deduplicated so two runs of the same mutation read identically, and **the
        # count is stated when the list is cut**.
        #
        # This truncated to four silently until 2026-08-12, which is a bad way for this particular
        # report to be wrong. The list is the evidence for whether a mutation is caught by the test
        # written for it or by a coincidental one somewhere else -- the distinction this file
        # records for the `Keyword` mutation, where a catch by a `MemoryText` test would have read
        # as proof of a guard nothing exercised. A silently short list makes the intended guard look
        # absent: a `scope` mutation reported four `mcp` tests and dropped the two `memory_model`
        # ones written for it, and confirming they had run took a hand-applied patch.
        named = sorted(set(named))
        shown = ", ".join(named[:LISTED_FAILURES])
        if len(named) > LISTED_FAILURES:
            shown += f", (+{len(named) - LISTED_FAILURES} more)"
        return Outcome("failed", shown)

    return Outcome("passed")


def clearing_check() -> int:
    """Prove the clearing decision fires where it should and nowhere else.

    Cheap to state and easy to get wrong in the direction that costs nothing to notice and
    everything to suffer: a rule that never fires leaves every run slow, and one that fires on a
    clean directory turns a three-second check into an eighteen-second one.
    """
    cases = [
        ("a silted directory is cleared", 347_553, True),
        ("a clean one is left alone", 645, False),
        ("45,000 measured no slower, so it is not cleared", 45_000, False),
        ("exactly at the threshold is not above it", CROWDED, False),
        ("just above it is", CROWDED + 1, True),
    ]
    failures = [
        f"{name}: {count:,} files -> expected {expected}"
        for name, count, expected in cases
        if should_clear(count) is not expected
    ]
    for failure in failures:
        print(f"self-check: {failure}", file=sys.stderr)
    if failures:
        return 1
    say(f"self-check: the clearing decision holds ({len(cases)} cases)")
    return 0


def self_check() -> int:
    """Prove this runner survives a command whose children outlive it.

    The shape is `cargo test`'s exactly: a parent that leaves a child running with the inherited
    pipe still open. Before the group kill, this call never returned.
    """
    spinner = "while :; do :; done"
    command = ["sh", "-c", f"sh -c '{spinner}' & echo started; sleep 3600"]

    budget = 3
    started = time.monotonic()
    status, output, _ = run(command, budget=budget)
    elapsed = time.monotonic() - started

    failures = []
    if status is not None:
        failures.append(f"a command that outran its budget reported status {status}, not a timeout")
    if elapsed > budget + KILL_GRACE_SECONDS + 5:
        failures.append(
            f"the timeout took {elapsed:.0f}s for a {budget}s budget: the runner is waiting on "
            f"a pipe held by something it did not kill"
        )
    # The evidence half of the same guarantee: what a killed command managed to say is the only
    # account of why it was killed, and dropping it is what made one timeout unexplainable.
    if "started" not in output:
        failures.append("the output a killed command produced before the kill was discarded")

    survivors = subprocess.run(  # noqa: S603
        ["pgrep", "-f", spinner], capture_output=True, text=True, check=False
    ).stdout.split()
    if survivors:
        failures.append(f"the child outlived the kill: {' '.join(survivors)}")
        for pid in survivors:
            try:
                os.kill(int(pid), signal.SIGKILL)
            except (OSError, ValueError):
                pass

    # **A slow command is photographed and then allowed to finish**: the snapshot is a record, and a
    # step that ran long is not a step that failed.
    status, output, diagnosis = run(
        ["sh", "-c", "echo started; sleep 2; echo finished"], budget=30, slow_after=0.5
    )
    if status != 0 or "finished" not in output:
        failures.append(f"a slow command was not left to finish (status {status}): {output!r}")
    if "still running" not in diagnosis:
        failures.append("a command still running past `slow_after` was not photographed")
    # The swapping half: the snapshot has to report a *change* since the step began, which only a
    # baseline taken at the start can give. Absent `vm_stat` it says so rather than printing nothing.
    if vm_counters():
        if "swapouts +" not in diagnosis:
            failures.append("a snapshot did not say how much swapping happened during the step")
    elif "paging counters unavailable" not in diagnosis:
        failures.append("a snapshot without paging counters did not say they were unavailable")
    _, _, quick = run(["sh", "-c", "true"], budget=30, slow_after=5)
    if quick:
        failures.append("a command that finished before `slow_after` was photographed anyway")
    slow = Pace()
    if slow.slow_after("build") is not None:
        failures.append("a phase with no history was given a threshold")
    for seconds in (1.0, 1.0, 1.0, 1.0, 30.0):
        slow.record("build", seconds)
    if slow.slow_after("build") != SLOW_FLOOR_SECONDS:
        failures.append(
            f"one stall moved the bar to {slow.slow_after('build')}s: the median should hold it at "
            f"the floor"
        )

    for failure in failures:
        say(f"SELF-CHECK  {failure}")
    if failures:
        return 1
    say(stamped(f"self-check: a hung command is killed with its children in {elapsed:.1f}s"))
    say(stamped("self-check: a slow command is photographed and still left to finish"))
    return 0


def resolve_start(given: str, mutations: list[dict]) -> int | None:
    """Turn a `--start-at` value into an index, or `None` if it names nothing.

    A bare number is a position as `--list` prints it, counting from one; anything else is matched
    against names the same way `--only` is. Numbers win, because every mutation name here is prose
    and none of them could be mistaken for one.
    """
    if given.strip().lstrip("+-").isdigit():
        position = int(given)
        if not 1 <= position <= len(mutations):
            return None
        return position - 1
    needle = given.lower()
    return next(
        (index for index, m in enumerate(mutations) if needle in m["name"].lower()),
        None,
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--only", help="run mutations whose name contains this substring")
    parser.add_argument(
        "--start-at",
        metavar="N-OR-NAME",
        help="skip ahead to this position (as numbered by --list) or to the first name "
        "containing this substring, and run from there to the end",
    )
    parser.add_argument("--list", action="store_true", help="list mutations without running them")
    parser.add_argument(
        "--self-check",
        action="store_true",
        help="check that this runner's own timeout works, without running any mutation",
    )
    parser.add_argument(
        "--stale",
        action="store_true",
        help="report mutations whose `find` text is no longer in its file, without running anything",
    )
    parser.add_argument(
        "--restore",
        action="store_true",
        help="put back originals left behind by a run that was killed, then exit",
    )
    arguments = parser.parse_args()

    stranded = abandoned()
    if arguments.restore:
        if not stranded:
            print("nothing to restore -- no mutation is in flight")
            return 0
        for archived in stranded:
            print(f"restored {put_back(archived).relative_to(ROOT)}")
        return 0
    if stranded:
        # Running now would copy the *mutated* file over the pristine one and lose the original.
        print("a previous mutation run did not finish; these originals are still archived:", file=sys.stderr)
        for archived in stranded:
            print(f"  {archived.relative_to(ROOT)}", file=sys.stderr)
        print("\nrun `python3 scripts/mutate.py --restore` to put them back", file=sys.stderr)
        return 2

    mutations = MUTATIONS
    if arguments.only:
        mutations = [m for m in mutations if arguments.only.lower() in m["name"].lower()]
    if not mutations:
        print("no mutations matched", file=sys.stderr)
        return 1

    # Applied after `--only`, so the numbering `--start-at` takes is always the numbering the
    # matching `--list` just printed. Resolving it against the unfiltered list instead would make
    # the two commands disagree about what "the 55th" means.
    first = 0
    if arguments.start_at is not None:
        first = resolve_start(arguments.start_at, mutations)
        if first is None:
            print(f"--start-at {arguments.start_at!r} matched nothing", file=sys.stderr)
            return 1

    # **Staleness is the failure this whole script was written to make visible, arriving one level
    # up.** A mutation whose `find` text has moved is not applied at all: it runs nothing, catches
    # nothing, and reports neither pass nor fail -- so a guard it was the only proof of becomes
    # untested silently, and the count of recorded mutations keeps saying otherwise. A full pass does
    # say `NOT APPLIED`, but a full pass takes a quarter of an hour and buries the line among two
    # hundred others; this answers the same question in a second, with an exit code, so it can be run
    # after any rename or refactor rather than only when someone thinks to look.
    # **The runner's own timeout, checked directly rather than through a mutation.**
    #
    # `timeout` is one of three ways a mutation counts as caught, and until 2026-08-26 it was
    # unreachable: `cargo test` launches the test binaries, they inherit the output pipe, and killing
    # cargo alone left one spinning with the pipe open — so the wait that follows the timeout never
    # returned. A hanging mutation hung the *runner*, and the four-hour wedged pass recorded in the
    # `coding-lessons` set was that bug read as a problem with the test helpers.
    #
    # Checked here rather than by keeping a deliberately non-terminating mutation in the corpus,
    # which was the first repair tried. That costs a full timeout of wall clock on every pass to
    # exercise a path that belongs to *this script* rather than to the crate — and it proves it only
    # as a side effect. This takes a few seconds and asks the question directly.
    if arguments.self_check:
        # Both, and the timeout one last because it spends three seconds hanging a command on
        # purpose: a failure in the cheap check should not wait for that to finish.
        return clearing_check() or self_check()

    # **Ambiguous as well as stale**, since 2026-09-14 (`ambiguous-anchor-check`). A mutation is applied
    # at the *first* occurrence of its `find` text, so an anchor that was unique when written is
    # retargeted, with nothing reporting it, the day new code repeats it earlier in the file -- and it
    # still matches, which is all this used to ask. `store: treat a cross-set supersession as resolved`
    # had moved onto `check_one_live_charter` that way, and six more were ambiguous the day this was
    # added, one of them aimed at a different function from the one its recorded reason describes.
    # Counting costs nothing over the membership test it replaces.
    if arguments.stale:
        stale = []
        ambiguous = []
        for mutation in mutations:
            text = pathlib.Path(mutation["file"]).read_text(encoding="utf-8")
            occurrences = text.count(mutation["find"])
            if occurrences == 0:
                stale.append(mutation)
            elif occurrences > 1:
                ambiguous.append((mutation, occurrences))
        for mutation in stale:
            print(f"STALE  {mutation['name']}  ({mutation['file']})")
        for mutation, occurrences in ambiguous:
            print(
                f"AMBIGUOUS  {mutation['name']}  ({mutation['file']}: `find` occurs {occurrences} "
                f"times, and only the first is mutated)"
            )
        print(f"{len(stale)} of {len(mutations)} mutation(s) no longer match their file")
        print(f"{len(ambiguous)} of {len(mutations)} mutation(s) match their file more than once")
        return 1 if stale or ambiguous else 0

    if arguments.list:
        for position, mutation in enumerate(mutations, start=1):
            marker = " " if position > first else "-"
            print(
                f"{position:3d}{marker} {mutation['expect']:9s} "
                f"{mutation['name']}  ({mutation['file']})"
            )
        return 0

    mutations = mutations[first:]
    if first:
        say(f"starting at {first + 1} of {first + len(mutations)}; skipping {first}")

    clear_if_crowded()

    disagreements = 0
    uncompilable = 0
    durations: list[float] = []
    pace = Pace()
    width = max(len(m["name"]) for m in mutations)

    for position, mutation in enumerate(mutations):
        # Before this mutation's build rather than after the previous one's, so a pass that is
        # stopped between the two does not leave the next run to inherit the crowding.
        if position and position % CLEAR_CHECK_EVERY == 0:
            clear_if_crowded()

        started = time.monotonic()
        path = ROOT / mutation["file"]
        original = path.read_text()

        if mutation["find"] not in original:
            say(
                stamped(
                    f"{mutation['name']:{width}s}  NOT APPLIED -- "
                    f"`find` text is not in {path.name}"
                )
            )
            disagreements += 1
            continue

        mutated = original.replace(mutation["find"], mutation["replace"], 1)
        archived = archived_for(path)

        # The pristine copy reaches disk before the mutation does, so a kill at any point after
        # this leaves something to recover from rather than nothing.
        if archived.exists():
            say(stamped(f"{mutation['name']:{width}s}  REFUSED -- {archived} already exists"))
            disagreements += 1
            continue
        archived.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(path, archived)

        restore = atexit.register(put_back, archived)
        try:
            path.write_text(MARKER + mutated)
            outcome = evaluate(mutation.get("tests", []), pace)
        finally:
            put_back(archived)
            atexit.unregister(restore)

        expected = mutation["expect"]
        expected_caught = expected in {"caught", "compiler"}
        actually_caught = outcome.kind in CAUGHT
        by_compiler = outcome.kind in BY_THE_COMPILER
        agrees = expected_caught == actually_caught and outcome.kind not in INCONCLUSIVE
        # Both directions. A mutation the compiler stopped without saying it would is a record that
        # proves nothing about the tests; one that declared `compiler` and was caught by a test
        # instead is a record whose reason has gone stale in the good direction, and saying so is
        # what gets it upgraded rather than left describing a state that no longer holds.
        if by_compiler != (expected == "compiler"):
            agrees = False

        mark = "ok  " if agrees else "MISMATCH"
        detail = f" -- {outcome.detail}" if outcome.detail else ""
        elapsed = time.monotonic() - started
        durations.append(elapsed)
        say(
            stamped(
                f"{mutation['name']:{width}s}  {mark}  {outcome.kind}{detail}",
                elapsed,
            )
        )

        if outcome.diagnosis:
            say(outcome.diagnosis)

        if by_compiler:
            uncompilable += 1

        if not agrees:
            disagreements += 1
            if not expected_caught:
                say(f"{'':{width}s}      expected to survive: {mutation.get('reason', '?')}")
            elif by_compiler and expected != "compiler":
                say(
                    f"{'':{width}s}      caught by the compiler, not by a test, so it proves "
                    f"nothing about the suite. Either the replacement has rotted -- check what it "
                    f"constructs -- or the type system really is the guard, in which case record "
                    f'`expect = "compiler"` and say why.'
                )
            elif expected == "compiler":
                say(
                    f"{'':{width}s}      recorded as caught by the compiler and it compiled. The "
                    f"type system is no longer what stops this; re-record it as `caught`."
                )

    say("")
    if durations:
        mean = sum(durations) / len(durations)
        # Reported as a *cadence* rather than a verdict: a run proves nothing about the machine on
        # its own. Slowness alone was twice attributed confidently to the wrong cause on 2026-08-08,
        # from CPU samples rather than from a comparison.
        say(stamped(f"{len(durations)} mutations in {sum(durations):.0f}s, mean {mean:.1f}s each"))

    # Rebuild before leaving, so this directory's artifacts match the restored source.
    #
    # **No longer what keeps a mutated server off a client.** `MUTATION_TARGET` does that, and does
    # it for the whole pass rather than only at the end: nothing here writes to `target/`, so
    # `target/debug/scatterhoard` is never built from mutated source at all. What this still buys is
    # a mutation directory whose artifacts are not the last mutation's, which keeps a later
    # `--only` run from opening on a stale link and makes the first mutation of the next pass cost
    # what the rest do.
    #
    # The history is worth keeping, because it is why the split exists. On 2026-08-11 the smoke test
    # in SETUP.md §1 was run straight after a pass and reported a schema the source had not produced
    # for an hour; the client was restarted onto the same binary, so a live verification checked
    # mutated code. This rebuild was the repair, and it closed the end state while leaving the whole
    # length of a pass open -- a mutated server answers confidently, and nothing says which build it
    # is.
    say(stamped("rebuilding, so the mutation directory matches the restored source"))
    code, _, error = run(["cargo", "build", "--quiet"])
    if code != 0:
        say(
            stamped(
                f"WARNING: the rebuild failed, so {MUTATION_TARGET.relative_to(ROOT)} still holds "
                f"mutated artifacts: {error.strip()[:200]}"
            )
        )

    # Reported whether or not anything disagreed, because the number is the point: a corpus where
    # this is not zero is one whose coverage figure is partly the compiler's rather than the
    # suite's, and the summary is the only place a reader meets that.
    if uncompilable:
        say(
            stamped(
                f"{uncompilable} mutation(s) were stopped by the compiler rather than by a test"
            )
        )

    if disagreements:
        say(stamped(f"{disagreements} mutation(s) did not behave as recorded"))
    else:
        say(stamped(f"all {len(mutations)} mutations behaved as recorded"))
    return 1 if disagreements else 0


if __name__ == "__main__":
    sys.exit(main())
