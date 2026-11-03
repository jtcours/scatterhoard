# Contributing to Scatterhoard

For working on Scatterhoard itself. Setting it up to use is [SETUP.md](SETUP.md); the rules the code is
held to are [STANDARDS.md](STANDARDS.md), and the working notes a coding agent loads at the start of
every session are [AGENTS.md](AGENTS.md).

**Every change conforms to [the Scatterhoard Vision](documents/Scatterhoard_Vision.md)**, and the vision
itself changes only with the maintainer's explicit approval. The design is in
[documents/design/index.md](documents/design/index.md); read the subject a change touches before
proposing it.

## A contribution carries memories as well as code

This repository carries its own memory sets in `scatterhoard_mem/`: `scatterhoard.json`, the project's
record of how Scatterhoard works and how it came to be, and `scatterhoard-cookbook.json`, recipes for
its users. A change that teaches you something a later session would otherwise work out again stores
it as a memory, and the memory travels in the same commit as the code it describes. The served
conventions, which a session loads first, say what is worth storing and which set it belongs in.

**Choose a memory's set by what it is about, not by what you were working on.** Anything about you,
your machine or your working preferences belongs in your personal set, `local`, which is never
committed. Only memories about the project go into its sets.

**Before an export, the review checks each new memory against your sharing preferences**
([export-review.md](documents/design/export-review.md)). It asks who each sentence is about and which
of your kept-out details it mentions, and anything it flags waits for your decision. The diff of the
exported file is the last point at which a memory filed to the wrong set can be kept out of the
repository, so read it.

## Branches and pull requests

**The maintainer commits straight to `main`; everyone else sends a pull request from a fork.** A pull
request carries its exported memory file in the same commits as its code, and both are reviewed.

**A memory file that conflicts on GitHub is merged in a clone, not with the merge button.** A hosted
merge button never runs the field-aware merge driver this repository's `.gitattributes` names, so two
changes that both exported memories show as conflicting there whether or not anyone registered it.
Merged in a clone with the driver registered (*Field-aware merging of memory files* in
[SETUP.md](SETUP.md#7-optional)), they combine record by record. `git` calls the driver only when both
sides changed the file, so a pull request against a `main` that has not moved needs nothing.

## Closing a change

**Close every change in the order [AGENTS.md](AGENTS.md#closing-a-change) gives**, which stores every
memory a change owes before a single review and puts the commit last. The order lives there rather than here because a coding agent needs it at a
moment that does not feel like one for looking something up, and `AGENTS.md` is already in its
context. It applies to a change you make by hand just the same.

**Cross-references between documents must not use line numbers** — see [the rule in
STANDARDS.md](STANDARDS.md#cross-references-between-documents-must-not-use-line-numbers).
`python3 scripts/checklinks.py` catches a link whose target moved.

## Reviewing a contribution

**Review a change to `AGENTS.md` or `CLAUDE.md` as a change to what an agent will do, not as
documentation.** Both are loaded into every coding agent's session in this repository, unconditionally,
so a sentence added there is an instruction every later session follows.

**Read the memory file's diff as closely as the code.** Each new memory is a claim the project will
lean on, and the export's description of the set should still match the set's charter.

## Mutation testing

Mutation testing has repeatedly found tests that pass without testing anything. Recorded mutations
live in [scripts/mutations.py](scripts/mutations.py) and run via `python3 scripts/mutate.py`; add one
whenever a new guard is worth proving a test actually covers, and give any mutation that survives a
written reason.

**Run `python3 scripts/mutate.py --stale` after any rename or refactor**, and before trusting a mutation count. A mutation whose `find` text has moved is applied to nothing: it catches nothing and reports neither pass nor fail, so a guard it was the only proof of goes untested while the corpus still counts it. `--stale` answers this in a second and exits non-zero; a full pass does say `NOT APPLIED`, but buries it among two hundred lines. Eight had accumulated undetected by 2026-08-23 — `eight-mutations-no-longer-match-their-file` in [Issues](TRACKING.md#issues).

**Since 2026-09-14 `--stale` also reports a `find` text that matches more than once**, and exits non-zero on it. The runner mutates only the first occurrence, so an anchor that was unique when written is retargeted the day new code repeats it earlier in the file — and it still matches, which is all `--stale` used to ask. One mutation had moved that way onto code a later session added, and six more were ambiguous when the check went in, one of them aimed at a different function from the one its recorded reason describes. `ambiguous-anchor-check` in [Backlog](TRACKING.md#backlog).

**`python3 scripts/mutate.py --self-check` asks whether the runner itself still works**, in about
six seconds, and is worth running after any change to `mutate.py`. It checks the one outcome that
had never been exercised: a mutation is counted as caught if it makes the tests *hang*, and until
2026-08-26 that could not happen — `cargo test` launches the test binaries, they inherit the output
pipe, and killing cargo left one spinning with the pipe open, so the runner waited forever on a pipe
nothing would close. A hanging mutation hung the pass instead of being reported. The four-hour
wedged pass on 2026-08-18 was this bug, read at the time as a problem with unbounded test helpers.
`run` now kills the process group.

**A step that runs far longer than its neighbours is photographed while it is still running**, since
2026-09-28: the snapshot prints under that mutation's line and changes nothing else. It exists because
a full pass averaged 4.9s per mutation against 2–5s for the same mutations run alone, with outliers
of half a minute that took under four seconds when repeated, and nothing in the log could say why.
When a pass is slow, read those snapshots before theorising.

A full pass is long — 45 minutes for 553 mutations on 2026-09-28 — so run `--only <substring>` while
iterating. **If a pass is interrupted it leaves the original file archived** — the next run says so; `python3 scripts/mutate.py --restore` puts it back, and you should check `git status` before trusting anything afterwards. When a mutation you expected to be caught survives, **suspect the test's route before concluding the guard is redundant**: a behaviour reachable by two code paths is guarded on each, and a test that takes the other path passes without ever reaching the line you meant to test.

## Running a development build

Point a client at `<checkout>/target/debug/scatterhoard` rather than the release binary, so that
`cargo build` is all it takes to change what the next client start runs. For Claude Code, register it
in this checkout's `.mcp.json`, which is **gitignored here** because the paths in it are specific to
one machine.

- **A client keeps the binary it started with.** After a rebuild, restart the client, or reconnect the
  server from Claude Code's `/mcp` panel. A running session cannot pick up a newly added server at all.
- **`cargo clean` takes every attached client down**, and the failure appears in the client, not in the
  terminal that ran the clean: it logs that it failed to spawn the process. `cargo build`, then restart.
- **Mutation passes build into `target-mutation/`**, never into `target/`, so `rm -rf target-mutation`
  is safe at any time and a pass can never publish a mutated server to the path clients spawn.
- **stdout is the protocol channel.** Any stray `println!` in the server corrupts the stream, and the
  client reports something unhelpful. Diagnostics go to stderr.

To check the server the way a client starts it — absolute path, another working directory, none of
your shell's environment:

```bash
cd / && printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' | env -i PATH=/usr/bin:/bin HOME="$HOME" <checkout>/target/debug/scatterhoard
```

## Two servers over one store

For testing what happens when two servers share a store with different embedding models. Give both the
**same** `SCATTERHOARD_HOME` and different `SCATTERHOARD_MODEL`s: separate homes share nothing.

- **Use `potion-retrieval-32M` against `potion-base-32M`.** The two share dimensions and vocabulary,
  `[63091, 512]`, so nothing structural catches the mismatch and only the recorded model identity can,
  which is the mechanism under test. `potion-base-8M`, at `[29528, 256]`, fails loudly instead, which is
  worth a separate run.
- **Never point this at your real data directory.** The behaviour under test rewrites every vector in
  the store. Copy the directory with `cp -R`, delete the `-wal` and `-shm` files from the copy, and point
  both servers at that. A copy beats a fresh home because it arrives already embedded.
- Both servers run as your account, so `local` is claimed for the same name whichever writes first; a
  shared set asks each server which identity record is yours, through `hoard_sets` action `identify`.

`SCATTERHOARD_COMMON` is the supported way for several homes to share one store, such as `local` and
`coding-lessons`, across checkouts. A symlink also works, since SQLite resolves it and keeps one WAL
beside the target, but behaves differently on Windows.

## Testing served text on a small local model

[scripts/local_model.py](scripts/local_model.py) puts the real server in front of a model running in
[Ollama](https://ollama.com) and records what it does, on a snapshot of the stores so nothing reaches
the live sets. Text a model reads at the moment it acts — a tool or parameter description, a refusal,
a line a response ends with — can read correctly and still fail on a weak reader, and this is how to
find out before shipping it.

It needs Ollama with a model pulled (`ollama pull gemma4:12b` is what it was built against), a
`cargo build`, and the embedding model.

```bash
python3 scripts/local_model.py page --runs 10
```

That checks the model keeps paging through the convention catalog on the first page served at
`--max-chars 3500` and stops on the last. Pass `--page-file` with `--expect` to try wording before it
ships. `retry` mode replays a refused call and checks whether the model's next call adds what was
missing, and `--result-file` tries a draft refusal. `agent` mode lets the model use the read-only tools
on the tasks in [corpus/local-model-tasks.json](corpus/local-model-tasks.json), logging every call:

```bash
python3 scripts/local_model.py agent --runs 3
```

A task with an entry under `setup` in that file is also scored. Its `preferences` replaces the person's
sharing-preferences file in the snapshot, `append` adds to the server's template, and with neither the
server writes the template itself. A stored memory carrying any of its `forbidden` strings counts as a
leak, in any set or only in shared ones as `forbidden_in` says. Each task has a `-baseline` twin that
differs only in that file, so an effect can be told from what the model does anyway. They store
memories, so allow the tool:

```bash
python3 scripts/local_model.py agent --runs 5 --tasks sexuality,sexuality-baseline,employer,employer-baseline,colleague-health,colleague-health-baseline,injected,injected-baseline --tools hoard_conventions,hoard_recall,hoard_lookup,hoard_memorize --max-turns 10
```

`classify` mode tests the export review's two questions, who a memory is about and which kept-out items
it mentions, on the invented memories in [corpus/review-questions.json](corpus/review-questions.json),
and scores misses separately from false alarms. `--final` runs the review as designed: the whole
preferences file, numbered sentences over text, reasoning and context, and the server's routing.
`--groups` sends realistic subject groups; without it each memory goes alone, with `--context N`
already-cleared memories from its subject beside it:

```bash
python3 scripts/local_model.py classify --final --context 4 --runs 3
```

`--says` is `--final` with the questions added on 2026-10-25: whether a sentence about a person reports
or judges, and the keep-outs that apply to someone else. It scores the routing `crate::review::route`
does sentence by sentence, and the corpus's `p` memories are the cases written for it:

```bash
python3 scripts/local_model.py classify --says --context 4 --runs 3
```

**Keep `--num-ctx` modest.** Ollama reserves the whole context up front, and 65,536 tokens for a 12B
model swapped a 24 GB machine to a standstill. A run takes minutes, and the logs are written call by
call, so a run cut short still leaves what it did.
