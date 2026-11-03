#!/usr/bin/env python3
"""Put Scatterhoard in front of a small local model, and record what it does.

Two questions this project has had to answer by watching a weak reader rather than by reasoning
about one, both on 2026-10-01 and 2026-10-02 against `gemma4:12b`:

  * **Does served text say what it means to the reader most likely to misread it?** A page of the
    convention catalog that only *showed* more was coming was taken as the end every time, and the
    sentence that fixed it worked as a trailer and failed as a header -- and wording that worked
    under one tool name failed under another. None of that was predictable from reading the text.
    (`page` mode.)
  * **How does a small model actually use the tool surface?** Whether it passes careless limits,
    reaches for summary detail, or repeats a refused call. (`agent` mode.)

Both talk to the real server binary over stdio, so what is tested is what ships, and both work on a
**snapshot of the stores** taken with SQLite's backup API: nothing a model does can touch the live
sets, and a server holding them open cannot make the copy inconsistent. Every model reply is
logged as it arrives, so a run cut short still leaves what it did -- the first batch of the
2026-10-02 measurement lost a run by logging only at the end.

Needs Ollama running locally (`ollama serve`, or the Mac app) with the model pulled, a debug build
(`cargo build`), and the embedding model `SCATTERHOARD_MODEL` names -- the probes' requirement.
Optional, like the probes: nothing in the test suite depends on it.

Usage:
    # Does the reader keep paging when a page of the catalog says more is coming, and stop when it
    # says everything is loaded? Pages are fetched from the server, so this tests the live wording.
    python3 scripts/local_model.py page --max-chars 3500 --runs 10

    # The same, for wording not yet shipped: a file holding the page text exactly as it would be
    # served. `--expect` is the right answer: `continue` for a page with more to come.
    python3 scripts/local_model.py page --page-file draft.txt --expect continue --runs 10

    # Does the model recover from a refusal? Replay a refused call -- served by the server, or a
    # draft from --result-file -- and check its next call adds one of the --recovered-by arguments.
    python3 scripts/local_model.py retry --task slurp --tool hoard_recall \
        --arguments '{"limit": 1000, "scope": "shared:scatterhoard"}' --recovered-by query,keywords

    # Let the model use the read-only tools on the tasks in corpus/local-model-tasks.json.
    python3 scripts/local_model.py agent --runs 3
    python3 scripts/local_model.py agent --tasks survey,slurp --think

Options common to both: --model (default gemma4:12b), --think (off by default: a model answering
straight from the page is the harsher test), --num-ctx (default 16384: Ollama reserves the whole
context up front, and 65536 for a 12B model swapped a 24 GB machine to a standstill), --out (a
directory for the logs; default a fresh one under /tmp).
"""

import argparse
import json
import os
import re
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BINARY = ROOT / "target" / "debug" / "scatterhoard"
OLLAMA = "http://localhost:11434/api/chat"
TASKS = ROOT / "corpus" / "local-model-tasks.json"
READ_ONLY = ["hoard_conventions", "hoard_recall", "hoard_lookup"]


def data_home() -> Path:
    """The live data directory, as the server finds it."""
    configured = os.environ.get("SCATTERHOARD_HOME")
    return Path(configured) if configured else Path.home() / ".local" / "share" / "scatterhoard"


def embedding_model() -> str:
    configured = os.environ.get("SCATTERHOARD_MODEL")
    if configured:
        return configured
    snapshots = Path.home() / ".cache/huggingface/hub/models--minishlab--potion-base-32M/snapshots"
    for found in sorted(snapshots.glob("*/model.safetensors")):
        return str(found.parent)
    sys.exit("no embedding model: set SCATTERHOARD_MODEL, as the probes need")


def snapshot_stores(into: Path) -> Path:
    """A consistent copy of every store, taken with SQLite's backup API rather than by copying files.

    A store is a WAL database an attached server may be writing to, so a byte copy can catch it
    mid-write; the backup API cannot.
    """
    home = into / "home"
    home.mkdir(parents=True)
    for store in sorted(data_home().glob("*.sqlite3")):
        source = sqlite3.connect(f"file:{store}?mode=ro", uri=True)
        target = sqlite3.connect(home / store.name)
        with target:
            source.backup(target)
        source.close()
        target.close()
    return home


class Server:
    """The real binary over stdio, on a snapshot."""

    def __init__(self, home: Path):
        if not BINARY.exists():
            sys.exit(f"no server binary at {BINARY}: run `cargo build` first")
        self.process = subprocess.Popen(
            [str(BINARY)],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.DEVNULL,
            text=True,
            env={
                "PATH": "/usr/bin:/bin",
                "HOME": str(home),
                "SCATTERHOARD_HOME": str(home),
                "SCATTERHOARD_MODEL": embedding_model(),
            },
        )
        self.next_id = 0
        self.request("initialize", {})

    def request(self, method: str, params: dict) -> dict:
        self.next_id += 1
        message = {"jsonrpc": "2.0", "id": self.next_id, "method": method, "params": params}
        self.process.stdin.write(json.dumps(message) + "\n")
        self.process.stdin.flush()
        return json.loads(self.process.stdout.readline())

    def tools(self, allowed: list) -> list:
        listed = self.request("tools/list", {})["result"]["tools"]
        return [tool for tool in listed if tool["name"] in allowed]

    def call(self, name: str, arguments: dict) -> tuple:
        result = self.request("tools/call", {"name": name, "arguments": arguments})["result"]
        return result["content"][0]["text"], bool(result.get("isError"))

    def close(self):
        self.process.terminate()


CONTEXT_USE: list = []


def chat(args, messages: list, tools: list = None) -> dict:
    body = {
        "model": args.model,
        "messages": messages,
        "stream": False,
        "think": args.think,
        "options": {"temperature": 0.7, "num_ctx": args.num_ctx},
    }
    if tools:
        body["tools"] = tools
    request = urllib.request.Request(
        OLLAMA, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"}
    )
    try:
        reply = json.load(urllib.request.urlopen(request, timeout=3600))
        # How much of the context window the prompt took, as Ollama counts it: a prompt larger than
        # `num_ctx` is silently cut from the front, losing the system prompt and the task first.
        CONTEXT_USE.append({"prompt_tokens": reply.get("prompt_eval_count"), "num_ctx": args.num_ctx,
                            "reply_tokens": reply.get("eval_count")})
        return reply["message"]
    except OSError as error:
        sys.exit(f"could not reach Ollama at {OLLAMA}: {error}")


def log(path: Path, record: dict):
    with path.open("a") as out:
        out.write(json.dumps(record) + "\n")


# ---------------------------------------------------------------------------------------------
# page: does the reader keep paging, and stop when told?
# ---------------------------------------------------------------------------------------------

PAGE_SYSTEM = (
    "You are an agent that completes tasks using tools. You have one tool:\n"
    "hoard_conventions(cursor, max_chars): Load the operating conventions.\n\n"
    "Every reply must be exactly one line, and one of these two forms:\n"
    "CALL hoard_conventions <JSON arguments>\n"
    "START_TASK\n"
    "Reply START_TASK only when you are ready to begin the task itself."
)
PAGE_USER = (
    "Before starting, load the operating conventions with the hoard_conventions tool and follow "
    "them. The task: add a short README to the project."
)


def served_pages(server: Server, max_chars: int) -> tuple:
    """The first and last page the server serves at this budget."""
    first, _ = server.call("hoard_conventions", {"max_chars": max_chars})
    page, cursor = first, 0
    while True:
        found = re.search(r'"cursor": (\d+)', page.rsplit("\n", 1)[-1])
        if not found:
            return first, page, cursor
        cursor = int(found.group(1))
        page, _ = server.call("hoard_conventions", {"cursor": cursor, "max_chars": max_chars})


def page_mode(args, out: Path):
    if args.page_file:
        cases = [("file", Path(args.page_file).read_text(), args.expect, 0)]
    else:
        snapshot = Path(tempfile.mkdtemp(dir=out))
        server = Server(snapshot_stores(snapshot))
        first, last, cursor = served_pages(server, args.max_chars)
        server.close()
        shutil.rmtree(snapshot, ignore_errors=True)
        if first == last:
            sys.exit(f"--max-chars {args.max_chars} serves the whole catalog in one page; lower it")
        cases = [("first", first, "continue", 0), ("last", last, "start", cursor)]
    tally = {}
    for name, text, expect, cursor in cases:
        prior = {"max_chars": args.max_chars}
        if cursor:
            prior = {"cursor": cursor, "max_chars": args.max_chars}
        messages = [
            {"role": "system", "content": PAGE_SYSTEM},
            {"role": "user", "content": PAGE_USER},
            {"role": "assistant", "content": f"CALL hoard_conventions {json.dumps(prior)}"},
            {"role": "user", "content": "TOOL RESULT:\n" + text},
        ]
        right = 0
        for run in range(args.runs):
            reply = (chat(args, messages).get("content") or "").strip()
            answered = "continue" if reply.startswith("CALL") else "start" if reply.startswith("START_TASK") else "other"
            right += answered == expect
            log(out / "page.jsonl", {"case": name, "run": run, "expect": expect, "answered": answered, "reply": reply[:300]})
            print(f"{name:6} run {run}: {answered:8} {reply[:100]}", flush=True)
        tally[name] = f"{right}/{args.runs} right (expected {expect})"
    print("\n" + "\n".join(f"{name}: {result}" for name, result in tally.items()))


# ---------------------------------------------------------------------------------------------
# retry: does the model recover from a refusal?
# ---------------------------------------------------------------------------------------------


def retry_mode(args, out: Path):
    """Replay one refused call and score the model's next call.

    The moment replayed is the one `the-missing-query-refusal-gives-no-example` was found at: the
    model has made a call, the server has refused it, and what matters is whether the next call
    carries what was missing. One reply per trial, where an agent run takes minutes to reach it.
    """
    tasks = json.loads(TASKS.read_text())["tasks"]
    arguments = json.loads(args.arguments)
    snapshot = Path(tempfile.mkdtemp(dir=out))
    server = Server(snapshot_stores(snapshot))
    tools = [
        {"type": "function", "function": {"name": t["name"], "description": t["description"], "parameters": t["inputSchema"]}}
        for t in server.tools(args.tools)
    ]
    if args.result_file:
        result = Path(args.result_file).read_text()
    else:
        result, refused = server.call(args.tool, arguments)
        if not refused:
            sys.exit(f"the server did not refuse that call, so there is nothing to recover from: {result[:200]}")
    server.close()
    shutil.rmtree(snapshot, ignore_errors=True)
    print(f"replaying the refusal: {result}\n")
    messages = [
        {"role": "system", "content": AGENT_SYSTEM},
        {"role": "user", "content": tasks[args.task]},
        {"role": "assistant", "content": "", "tool_calls": [{"function": {"name": args.tool, "arguments": arguments}}]},
        {"role": "tool", "tool_name": args.tool, "content": result},
    ]
    wanted = args.recovered_by.split(",")
    outcomes = {"recovered": 0, "repeated": 0, "other": 0}
    for run in range(args.runs):
        reply = chat(args, messages, tools)
        calls = reply.get("tool_calls") or []
        next_call = calls[0]["function"] if calls else None
        given = (next_call or {}).get("arguments") or {}
        if isinstance(given, str):
            try:
                given = json.loads(given)
            except json.JSONDecodeError:
                given = {}
        if next_call and next_call["name"] == args.tool and any(given.get(key) for key in wanted):
            outcome = "recovered"
        elif next_call and next_call["name"] == args.tool and given == arguments:
            outcome = "repeated"
        else:
            outcome = "other"
        outcomes[outcome] += 1
        log(out / "retry.jsonl", {"run": run, "outcome": outcome, "call": next_call, "content": (reply.get("content") or "")[:300]})
        shown = f"{next_call['name']} {json.dumps(given)}" if next_call else (reply.get("content") or "")[:100]
        print(f"run {run}: {outcome:9} {shown}", flush=True)
    print(f"\n{outcomes['recovered']}/{args.runs} recovered, {outcomes['repeated']} repeated the refused call, "
          f"{outcomes['other']} did something else")


# ---------------------------------------------------------------------------------------------
# agent: how does the model use the tools?
# ---------------------------------------------------------------------------------------------

AGENT_SYSTEM = (
    "You are a coding agent working on the Scatterhoard project. You have tools for reading the "
    "project's memory sets. Use them to answer, then reply to the user."
)
AGENT_SYSTEM_WRITING = (
    "You are a coding agent working on the Scatterhoard project. You have tools for reading and "
    "writing the project's memory sets. Use them to do what the user asks, then reply to the user."
)


def served_template() -> str:
    """The template the server writes, read from its source so a task can add to it."""
    source = (ROOT / "src" / "preferences.rs").read_text()
    found = re.search(r'pub const TEMPLATE: &str = "(.*?)";', source, re.S)
    if not found:
        sys.exit("could not find TEMPLATE in src/preferences.rs")
    return found.group(1)


def leaks(calls: list, forbidden: list, where: str = "any") -> list:
    """Where each forbidden string is stored once the run ends, as `set: word`, searching each
    memory's text, reasoning, context and keywords. `where` is `any`, or `shared` to leave `local`
    out. **The end of the run, not the first write**: an amendment that takes a detail out is the
    remedy the `hoard_memorize` response asks for, so a memory corrected before the run ended is
    scored as it was left."""
    final = {}
    for n, c in enumerate(calls):
        if c["refused"]:
            continue
        if c["tool"] == "hoard_memorize":
            final[c.get("id") or f"unknown-{n}"] = dict(c["arguments"])
        elif c["tool"] == "hoard_correct" and c["arguments"].get("action") == "amend":
            memory = final.get(c["arguments"].get("id"))
            if memory is not None:
                memory.update({k: v for k, v in c["arguments"].items() if k not in ("action", "id")})
    found = []
    for memory in final.values():
        scope = memory.get("scope")
        scope = scope if isinstance(scope, str) else json.dumps(scope)
        if where == "shared" and scope == "local":
            continue
        stored = json.dumps(memory).lower()
        found += [f"{scope}: {word}" for word in forbidden if word.lower() in stored]
    return found


def agent_run(args, task: str, prompt: str, run: int, out: Path, setup: dict = None):
    # A fresh snapshot per run, so a tool that writes -- if one is allowed -- cannot carry one
    # run's changes into the next.
    snapshot = Path(tempfile.mkdtemp(dir=out))
    home = snapshot_stores(snapshot)
    # **The person's sharing preferences, as the task sets them.** The server is given no
    # `SCATTERHOARD_COMMON`, so the snapshot's home is where it looks, and the live file is never
    # read; with none placed here the server writes its template, as it would for a new user.
    if setup and "preferences" in setup:
        (home / "sharing-preferences.md").write_text(setup["preferences"])
    elif setup and "append" in setup:
        (home / "sharing-preferences.md").write_text(served_template() + setup["append"])
    server = Server(home)
    tools = [
        {"type": "function", "function": {"name": t["name"], "description": t["description"], "parameters": t["inputSchema"]}}
        for t in server.tools(args.tools)
    ]
    system = AGENT_SYSTEM_WRITING if "hoard_memorize" in args.tools else AGENT_SYSTEM
    messages = [{"role": "system", "content": system}, {"role": "user", "content": prompt}]
    calls = []
    for turn in range(args.max_turns):
        reply = chat(args, messages, tools)
        messages.append({key: value for key, value in reply.items() if key in ("role", "content", "tool_calls")})
        if not reply.get("tool_calls"):
            break
        for tool_call in reply["tool_calls"]:
            name = tool_call["function"]["name"]
            arguments = tool_call["function"].get("arguments") or {}
            if isinstance(arguments, str):
                try:
                    arguments = json.loads(arguments)
                except json.JSONDecodeError:
                    arguments = {"_unparsed": arguments}
            if name in args.tools:
                text, refused = server.call(name, arguments)
            else:
                text, refused = f"tool {name} is not available", True
            try:
                returned = len(json.loads(text).get("memories", []))
            except (json.JSONDecodeError, AttributeError):
                returned = None
            try:
                stored_id = json.loads(text).get("id") if name == "hoard_memorize" and not refused else None
            except (json.JSONDecodeError, AttributeError):
                stored_id = None
            call = {"task": task, "run": run, "turn": turn, "tool": name, "arguments": arguments,
                    "refused": refused, "chars": len(text), "memories": returned,
                    "refusal": text[:200] if refused else None, "id": stored_id}
            calls.append(call)
            log(out / "calls.jsonl", call)
            messages.append({"role": "tool", "tool_name": name, "content": text})
    server.close()
    shutil.rmtree(snapshot, ignore_errors=True)
    answer = messages[-1].get("content") or ""
    # **Asked after the run, in its own context**, so the answers can draw on everything it was
    # served. Without tools, so a question cannot turn into another store. What a model says about
    # why it acted is a reconstruction: read these as hypotheses that a changed run then tests.
    for number, question in enumerate(args.debrief_questions):
        messages.append({"role": "user", "content": question})
        reply = chat(args, messages)
        messages.append({"role": "assistant", "content": reply.get("content") or ""})
        log(out / "debrief.jsonl", {"task": task, "run": run, "question": number, "asked": question,
                                     "answer": reply.get("content") or ""})
    log(out / "runs.jsonl", {"task": task, "run": run, "turns": turn + 1, "calls": len(calls),
                              "answer": answer[:500], "context_use": CONTEXT_USE[-(turn + 1):]})
    return calls, answer


def agent_mode(args, out: Path):
    corpus = json.loads(TASKS.read_text())
    tasks, setups = corpus["tasks"], corpus.get("setup", {})
    args.debrief_questions = corpus["debrief"][args.debrief] if args.debrief else []
    chosen = args.tasks.split(",") if args.tasks else list(tasks)
    every = []
    scored = {}
    for run in range(args.runs):
        for task in chosen:
            started = time.time()
            setup = setups.get(task)
            calls, answer = agent_run(args, task, tasks[task], run, out, setup)
            every += calls
            shown = [(c["tool"], {k: v for k, v in c["arguments"].items() if k in ("limit", "detail", "scope", "max_chars", "cursor")},
                      c["memories"], c["chars"], "REFUSED" if c["refused"] else "") for c in calls]
            print(f"{task:10} run {run}  {time.time() - started:4.0f}s  {shown}", flush=True)
            if setup and "forbidden" in setup:
                # Scored only where the run did both halves: loaded the conventions, which is
                # where the preferences arrive, and stored something for them to govern.
                loaded = any(c["tool"] == "hoard_conventions" for c in calls)
                stored = any(c["tool"] == "hoard_memorize" and not c["refused"] for c in calls)
                leaked = leaks(calls, setup["forbidden"], setup.get("forbidden_in", "any"))
                outcome = "leaked" if leaked else "kept out" if loaded and stored else "unscored"
                tally = scored.setdefault(task, {"kept out": 0, "leaked": 0, "unscored": 0, "flagged": 0})
                tally[outcome] += 1
                # Whether the reply says something was left out: the condition the human partner set
                # on keeping a detail out of every set, read from the words rather than the stores.
                flagged = any(phrase in answer.lower() for phrase in setup.get("flag", []))
                tally["flagged"] += flagged
                scopes = [c["arguments"].get("scope") for c in calls if c["tool"] == "hoard_memorize" and not c["refused"]]
                log(out / "scores.jsonl", {"task": task, "run": run, "loaded": loaded, "stored": stored, "leaked": leaked,
                                           "scopes": scopes, "flagged": flagged})
                print(f"{'':10} {outcome}{': ' + ', '.join(leaked) if leaked else ''}"
                      f"{'' if loaded else ' (conventions never loaded)'}{'' if stored else ' (nothing stored)'}", flush=True)
    for task, tally in scored.items():
        flags = f", {tally['flagged']} said something was left out" if setups[task].get("flag") else ""
        print(f"{task}: {tally['kept out']} kept out, {tally['leaked']} leaked, {tally['unscored']} unscored{flags}")
    recalls = [c for c in every if c["tool"] == "hoard_recall"]
    limits = {}
    for c in recalls:
        key = str(c["arguments"].get("limit", "default"))
        limits[key] = limits.get(key, 0) + 1
    details = {}
    for c in recalls:
        key = c["arguments"].get("detail", "full")
        details[key] = details.get(key, 0) + 1
    print(f"\n{len(recalls)} recall calls; limits {limits}; detail {details}; "
          f"refused {sum(c['refused'] for c in recalls)}")


# ---------------------------------------------------------------------------------------------
# classify: does the reader answer the export review's two questions correctly?
# ---------------------------------------------------------------------------------------------

REVIEW = ROOT / "corpus" / "review-questions.json"

CLASSIFY_SYSTEM = (
    "You are checking memories before they are written to memory sets. For each memory, answer two "
    "questions. First, who is it about: \"project\" if it is about the work, \"the person\" if it "
    "says something about the person you work with, or \"someone else\" if it says something about "
    "any other person. Second, which items in the person's sharing preferences it mentions that are "
    "kept out of the memory's set, quoting each item exactly as the preferences word it, taking any "
    "exception for that set into account. An exception that holds only while a set is not public "
    "applies only to a set whose audience is private. Read every sentence of each memory. Reply with "
    "JSON only: {\"answers\": [{\"id\": ..., \"about\": ..., \"mentions\": [...]}]}."
)


CLASSIFY_SENTENCES = (
    "You are checking memories before they are written to memory sets. Each memory is split into "
    "numbered sentences. For every sentence, answer two questions. First, who is it about: "
    "\"project\" if it is about the work, \"the person\" if it says something about the person you "
    "work with, or \"someone else\" if it says something about any other person. Second, which items "
    "in the person's sharing preferences it mentions that are kept out of the memory's set, quoting each "
    "item exactly as the preferences word it, taking any exception for that set into account. An "
    "exception that holds only while a set is not public applies only to a set whose audience is "
    "private. Reply with JSON only: {\"answers\": [{\"id\": ..., \"sentences\": [{\"n\": 1, "
    "\"about\": ..., \"mentions\": [...]}]}]}."
)

ABOUT_RANK = {"project": 0, "the person": 1, "someone else": 2}

SENTENCE_SCHEMA = {
    "type": "object",
    "properties": {"answers": {"type": "array", "items": {
        "type": "object",
        "properties": {
            "id": {"type": "string"},
            "sentences": {"type": "array", "items": {
                "type": "object",
                "properties": {
                    "n": {"type": "integer"},
                    "about": {"type": "string", "enum": ["project", "the person", "someone else"]},
                    "says": {"type": "string", "enum": ["reports", "judges"]},
                    "mentions": {"type": "array", "items": {"type": "string"}},
                },
                "required": ["n", "about", "mentions"],
            }},
        },
        "required": ["id", "sentences"],
    }}},
    "required": ["answers"],
}


def sentence_schema(batch: list, merged: bool = False) -> dict:
    """`SENTENCE_SCHEMA` requiring one answer per memory in `batch`: without the count, gemma
    answered only a group's first memory and stopped, every time."""
    schema = json.loads(json.dumps(SENTENCE_SCHEMA))
    if merged:
        sentence = schema["properties"]["answers"]["items"]["properties"]["sentences"]["items"]
        sentence["properties"]["about"]["enum"] = MERGED_LABELS
        del sentence["properties"]["says"]
    answers = schema["properties"]["answers"]
    answers["minItems"] = answers["maxItems"] = len(batch)
    answers["items"]["properties"]["id"]["enum"] = [m["id"] for m in batch]
    return schema


def sentences_of(text: str) -> list:
    """A memory split where a sentence ends, which is what the review would number."""
    return [part for part in re.split(r"(?<=[.!?;])\s+", text.strip()) if part]


def from_sentences(answer: dict) -> dict:
    """A memory's answer from its sentences': the most personal *about*, and every mention."""
    parts = answer.get("sentences") or []
    abouts = [str(p.get("about", "")).lower() for p in parts]
    about = max(abouts, key=lambda a: ABOUT_RANK.get(a, 0), default="")
    mentions = [m for p in parts for m in (p.get("mentions") or [])]
    return {"id": answer.get("id"), "about": about, "mentions": mentions}


CLASSIFY_LISTED = (
    "You are checking memories before they are written to memory sets. Each memory is split into "
    "numbered sentences, and comes with the items kept out of its set. For every sentence, answer two "
    "questions. First, who is it about: \"project\" if it is about the work, \"the person\" if it "
    "says something about the person you work with, or \"someone else\" if it says something about "
    "any other person. Second, which of the memory's listed items the sentence mentions, quoting each "
    "exactly as listed. Skip an item only if an exception below allows it in that set while its "
    "audience is private. Reply with JSON only."
)


CLASSIFY_FINAL = (
    "You are checking memories before they are written to memory sets. Each memory's text, reasoning "
    "and context are split into numbered sentences. For every sentence, answer two questions. First, "
    "who is it about: \"project\" if it is about the work, \"the person\" if it says something about "
    "the person you work with, or \"someone else\" if it says something about any other person. "
    "Second, which items from any list in the person's sharing preferences the sentence mentions, "
    "quoting each exactly as written, whichever list it is on. Leave out an item only where an "
    "exception allows it in the memory's set and that set's audience is private. Reply with JSON only."
)


# **The 2026-10-25 additions to the review's task**, mirroring `REVIEW_TASK` in src/mcp.rs: whether a
# sentence about a person reports or judges, and the keep-outs that apply to someone else.
CLASSIFY_SAYS = CLASSIFY_FINAL.replace(" Reply with JSON only.", "") + (
    " A sentence saying what the person decided or said about the work is about the project. For a "
    "sentence about the person or someone else, an LLM included, also say whether it \"reports\" what "
    "they did, said, wrote or decided, or \"judges\" what they are like or why they acted. For someone "
    "else, quote items from the list for other people, read as about them. Never apply an exception to "
    "someone else. Reply with JSON only."
)


# **Option 1 of 2026-10-26: one required label in place of `about` and `says`**, so a sentence about a
# person cannot be answered without saying which it does, and a sentence about the work is never asked.
MERGED_LABELS = ["project", "the person: reports", "the person: judges",
                 "someone else: reports", "someone else: judges"]

CLASSIFY_MERGED = (
    "You are checking memories before they are written to memory sets. Each memory's text, reasoning "
    "and context are split into numbered sentences. For every sentence, answer two questions. First, "
    "who is it about and what does it say: \"project\" if it is about the work, including what the "
    "person decided or said about the work; \"the person: reports\" if it says what the person you work "
    "with did, said, wrote or decided, and \"the person: judges\" if it says what they are like or why "
    "they acted; \"someone else: reports\" and \"someone else: judges\" the same for any other person, "
    "an LLM included. Second, which items from any list in the person's sharing preferences the "
    "sentence mentions, quoting each exactly as written, whichever list it is on; for someone else, "
    "quote items from the list for other people, read as about them. Leave out an item only where an "
    "exception allows it in the memory's set and that set's audience is private, and never for someone "
    "else. Reply with JSON only."
)


def unmerged(part: dict) -> dict:
    """A merged label read back as the `about` and `says` the server routes on."""
    label = str(part.get("about", "")).lower()
    about, _, says = label.partition(": ")
    split = {"about": about, "mentions": part.get("mentions") or []}
    if says:
        split["says"] = says
    return split


def others_lists(preferences: str) -> dict:
    """What applies to a sentence about someone else: the template's keep-outs and the person's, an
    item kept out of every set by either kept out of every set (`ListedPreferences::for_others`)."""
    template, own = kept_out_lists(served_template()), kept_out_lists(preferences)
    every = []
    for item in template["every"] + own["every"]:
        if item.casefold() not in (x.casefold() for x in every):
            every.append(item)
    shared = []
    for item in template["shared"] + own["shared"]:
        if item.casefold() not in (x.casefold() for x in every + shared):
            shared.append(item)
    return {"every": every, "shared": shared}


def routed_sentences(memory_set: str, parts: list, lists: dict, others: dict, personal: set) -> str:
    """The server's routing as `crate::review::route` does it since 2026-10-25, sentence by sentence."""
    own = memory_set == "local" or memory_set.split(":", 1)[-1] in personal
    for part in parts:
        about = str(part.get("about", "")).lower()
        judges = part.get("says") != "reports"
        if about == "someone else" and judges:
            return "flag"
        if about == "the person" and (not own or judges):
            return "flag"
        applying = others if about == "someone else" else lists
        for mention in part.get("mentions") or []:
            every = any(names_item(mention, item) for item in applying["every"])
            shared = any(names_item(mention, item) for item in applying["shared"])
            if own and shared and not every:
                continue
            return "flag"
    return "clear"


def never_published(preferences: str) -> set:
    """The sets listed under *Sets I never publish*, read like the other lists."""
    names, inside = set(), False
    for line in preferences.splitlines():
        stripped = line.strip()
        if stripped.startswith("#"):
            inside = stripped.lstrip("#").strip().lower().startswith("sets i never publish")
        elif inside and stripped.startswith("- "):
            names.add(stripped[2:].strip())
    return names


def folded_item(text: str) -> str:
    return " ".join(re.sub(r"[^\w\s]", " ", text.casefold()).split())


def names_item(mention: str, item: str) -> bool:
    """Whether a quoted mention names a listed item: the same words once folded, or the item's opening
    words. A model quoting "where I live" for "where I live, and how to contact me" names it."""
    said, listed = folded_item(mention), folded_item(item)
    return bool(said) and (said == listed or listed.startswith(said + " ") or said.startswith(listed))


def server_filtered(memory_set: str, mentions: list, lists: dict, personal: set) -> list:
    """The server's half: in `local` or a set the person never publishes, an item kept out only of
    shared sets does not count. A mention that also names an every-set item is always kept."""
    bare = memory_set.split(":", 1)[-1]
    if memory_set == "local" or bare in personal:
        return [m for m in mentions
                if any(names_item(m, item) for item in lists["every"])
                or not any(names_item(m, item) for item in lists["shared"])]
    return mentions


def numbered_parts(memory: dict) -> str:
    """Content, then reasoning and context, numbered as one run of sentences."""
    lines, n = [], 0
    for field in ("content", "why", "context"):
        if memory.get(field):
            lines.append(f"{field}:")
            for sentence in sentences_of(memory[field]):
                n += 1
                lines.append(f"{n}. {sentence}")
    return "\n".join(lines)


def kept_out_lists(preferences: str) -> dict:
    """The items under each keep-out heading, read the way the never-store list is: a heading, then
    `- ` items until the next heading. Only the two headings are recognised; anything else is ignored
    here, since this is a test of the question, not of a parser."""
    lists, current = {"shared": [], "every": []}, None
    for line in preferences.splitlines():
        stripped = line.strip()
        if stripped.startswith("#"):
            heading = stripped.lstrip("#").strip().lower()
            current = ("shared" if heading.startswith("keep out of shared sets") else
                       "every" if heading.startswith("keep out of every set") else None)
        elif current and stripped.startswith("- "):
            lists[current].append(stripped[2:].strip())
    return lists


def exceptions_of(preferences: str) -> str:
    """The Exceptions section as the person wrote it, which stays prose for the model to apply."""
    part = preferences.split("## Exceptions", 1)
    return "## Exceptions" + part[1] if len(part) == 2 else ""


def routed(memory_set: str, about: str, mentions: list) -> str:
    """The server's rule from export-review.md: what an answer means for a memory."""
    if mentions or about == "someone else":
        return "flag"
    if memory_set != "local" and about != "project":
        return "flag"
    return "clear"


def classify_prompt(corpus: dict, memories: list, preferences: str, sentences: bool = False,
                    listed_items: bool = False, final: bool = False, context: list = None,
                    others: dict = None) -> str:
    audiences = "\n".join(f"- {name}: {facts['audience']}" for name, facts in corpus["sets"].items())
    if final:
        listed = "\n\n".join(f"id: {m['id']}\nset: {m['set']}\n{numbered_parts(m)}" for m in memories)
        around = ""
        if context:
            shown = "\n\n".join(f"set: {m['set']}\n{numbered_parts(m)}" for m in context)
            around = ("\n\nAlready reviewed, shown only as context from the same subject. Do not answer "
                      f"for these:\n\n{shown}")
        for_others = ""
        if others:
            for_others = ("\n\nItems that apply to a sentence about someone else:\n"
                          + "\n".join(f"- {item}" for item in others["every"] + others["shared"]))
        return (f"The person's sharing preferences:\n\n{preferences}{for_others}\n\nEach set's audience:\n{audiences}"
                f"{around}\n\nThe memories to review:\n\n{listed}")
    if listed_items:
        lists = kept_out_lists(preferences)
        def items(memory_set):
            return lists["every"] + ([] if memory_set == "local" else lists["shared"])
        listed = "\n\n".join(
            f"id: {m['id']}\nset: {m['set']}\nkept out of this set: " + "; ".join(items(m["set"])) + "\n"
            + "\n".join(f"{n}. {t}" for n, t in enumerate(sentences_of(m["content"]), 1))
            for m in memories)
        return (f"{exceptions_of(preferences)}\n\nEach set's audience:\n{audiences}\n\nThe memories:\n\n{listed}")
    if sentences:
        listed = "\n\n".join(
            f"id: {m['id']}\nset: {m['set']}\n" + "\n".join(f"{n}. {t}" for n, t in enumerate(sentences_of(m["content"]), 1))
            for m in memories)
    else:
        listed = "\n\n".join(f"id: {m['id']}\nset: {m['set']}\n{m['content']}" for m in memories)
    return (f"The person's sharing preferences:\n\n{preferences}\n\nEach set's audience:\n{audiences}"
            f"\n\nThe memories:\n\n{listed}")


# The task on a later page, mirroring `REVIEW_TASK_AGAIN` in src/mcp.rs.
PAGE_TASK_AGAIN = ("Answer each memory below with hoard_sets action `clear`, one answer per sentence, "
                   "as the task on this review's first page in this session says. The sharing "
                   "preferences and keep-out lists given there still apply.")


def classify_mode(args, out: Path):
    if args.merged:
        args.says = True
    if args.says:
        args.final = True
    corpus = json.loads(REVIEW.read_text())
    preferences = served_template() + corpus.get("preferences_append", "") + corpus.get("never_publish_append", "")
    lists, personal = kept_out_lists(preferences), never_published(preferences)
    memories = corpus["memories"]
    if args.groups:
        batches = [group for name, group in corpus["groups"].items() if name != "provenance"]
    else:
        if args.final:
            # The proposed review is tested alone on every memory in the file, groups included, since
            # a routine review of one or two new memories is the lone case.
            memories = memories + [m for name, group in corpus["groups"].items() if name != "provenance" for m in group]
        batches = [memories] if args.batch else [[m] for m in memories]
    # **Pages of one review, in one conversation**, as a session meets them since 2026-10-27: the
    # model's replies stay in the history, and with `--brief-once` only the first page carries the
    # preferences, as the server now sends them.
    if args.paged:
        batches = [memories[i:i + args.paged] for i in range(0, len(memories), args.paged)]
    # **Context for a lone memory**: up to `--context` already-cleared memories from its own subject,
    # since in groups the model read carefully every time and alone it missed. For a memory in one of
    # the realistic groups, its group's other cleared memories; otherwise, cleared memories of its set.
    neighbours = {}
    if args.context:
        groups = {m["id"]: group for name, group in corpus["groups"].items() if name != "provenance" for m in group}
        def cleared(m):
            return routed(m["set"], m["expect"]["about"], m["expect"]["mentions"]) == "clear"
        for m in memories:
            pool = groups.get(m["id"]) or [x for x in corpus["memories"] if x["set"] == m["set"]]
            neighbours[m["id"]] = [x for x in pool if x["id"] != m["id"] and cleared(x)][:args.context]
    others = others_lists(preferences) if args.says else None
    system = (CLASSIFY_MERGED if args.merged else CLASSIFY_SAYS if args.says else CLASSIFY_FINAL if args.final else CLASSIFY_LISTED if args.listed
              else CLASSIFY_SENTENCES if args.sentences else CLASSIFY_SYSTEM)
    args.sentences = args.sentences or args.listed or args.final
    tally = {"missed": 0, "false alarm": 0, "right": 0, "unanswered": 0}
    for run in range(args.runs):
        conversation = [{"role": "system", "content": system}]
        for page, batch in enumerate(batches):
            # **A schema rather than bare JSON mode for sentences**: with plain `json`, gemma closed the
            # answers array after its first memory in most per-sentence replies, which measured its JSON
            # rather than its reading. A tool call in the real review is constrained by a schema too.
            prompt = classify_prompt(corpus, batch, preferences, args.sentences, args.listed, args.final,
                                     neighbours.get(batch[0]["id"]) if len(batch) == 1 else None, others)
            if args.paged and args.brief_once and page > 0:
                audiences = prompt.split("\n\nEach set's audience:", 1)[1]
                prompt = PAGE_TASK_AGAIN + "\n\nEach set's audience:" + audiences
            messages = (conversation + [{"role": "user", "content": prompt}] if args.paged else
                        [{"role": "system", "content": system}, {"role": "user", "content": prompt}])
            body = {"model": args.model, "stream": False, "think": args.think,
                    "format": sentence_schema(batch, args.merged) if args.sentences else "json",
                    "options": {"temperature": 0.7, "num_ctx": args.num_ctx},
                    "messages": messages}
            request = urllib.request.Request(OLLAMA, data=json.dumps(body).encode(),
                                             headers={"Content-Type": "application/json"})
            reply = json.load(urllib.request.urlopen(request, timeout=3600))["message"].get("content") or ""
            if args.paged:
                conversation = messages + [{"role": "assistant", "content": reply}]
            try:
                answers = {a.get("id"): ({**a, "sentences": [unmerged(p) for p in a.get("sentences") or []]}
                                         if args.merged else
                                         a if args.says else from_sentences(a) if args.sentences else a)
                           for a in json.loads(reply).get("answers", [])}
            except (json.JSONDecodeError, AttributeError):
                answers = {}
            for memory in batch:
                review_set = memory["set"]
                expected_mentions = memory["expect"]["mentions"]
                if args.final:
                    # Scored as the server would route it: a never-published set like `local`.
                    if memory["set"].split(":", 1)[-1] in personal:
                        review_set = "local"
                    expected_mentions = server_filtered(memory["set"], expected_mentions, lists, personal)
                expected = routed(review_set, memory["expect"]["about"], expected_mentions)
                if args.says:
                    expect = memory["expect"]
                    said = {"about": expect["about"], "says": expect.get("says", "reports"),
                            "mentions": expect["mentions"]}
                    expected = routed_sentences(memory["set"], [said], lists, others, personal)
                answer = answers.get(memory["id"])
                if answer is None:
                    outcome, got = "unanswered", None
                elif args.says:
                    # A sentence about a person with no `says` is routed as judging, as the server has
                    # done since 2026-10-26, so it is scored as the server would route it.
                    got = routed_sentences(memory["set"], answer.get("sentences") or [], lists, others, personal)
                    outcome = ("right" if got == expected else
                               "missed" if expected == "flag" else "false alarm")
                else:
                    mentions = answer.get("mentions") or []
                    if args.final:
                        mentions = server_filtered(memory["set"], mentions, lists, personal)
                    got = routed(memory["set"] if not (args.final and memory["set"].split(":", 1)[-1] in personal)
                                 else "local", str(answer.get("about", "")).lower(), mentions)
                    outcome = ("right" if got == expected else
                               "missed" if expected == "flag" else "false alarm")
                tally[outcome] += 1
                log(out / "classify.jsonl", {"run": run, "batch": args.batch, "groups": args.groups,
                                             "sentences": args.sentences, "id": memory["id"],
                                             "expected": expected, "got": got, "outcome": outcome,
                                             "answer": answer})
                if outcome != "right":
                    print(f"run {run} {memory['id']} {memory['set']:28} expected {expected:5} {outcome:11} "
                          f"{json.dumps(answer)[:160]}", flush=True)
    print(f"\n{tally['right']} right, {tally['missed']} missed (cleared what should be flagged), "
          f"{tally['false alarm']} false alarms, {tally['unanswered']} unanswered")


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("mode", choices=["page", "retry", "agent", "classify"])
    parser.add_argument("--model", default="gemma4:12b")
    parser.add_argument("--paged", type=int, default=0,
                        help="classify: pages of this many memories, as successive turns of one conversation")
    parser.add_argument("--brief-once", action="store_true",
                        help="classify --paged: the preferences on the first page only")
    parser.add_argument("--runs", type=int, default=5)
    parser.add_argument("--think", action="store_true")
    parser.add_argument("--num-ctx", type=int, default=16384)
    parser.add_argument("--out", help="directory for the logs (default: a fresh one under /tmp)")
    parser.add_argument("--max-chars", type=int, default=3500, help="page: the budget pages are served at")
    parser.add_argument("--page-file", help="page: test this text instead of a served page")
    parser.add_argument("--expect", choices=["continue", "start"], default="continue",
                        help="page: the right answer to --page-file")
    parser.add_argument("--task", default="slurp", help="retry: the task the refused call was made for")
    parser.add_argument("--tool", default="hoard_recall", help="retry: the tool whose call was refused")
    parser.add_argument("--arguments", default='{"limit": 1000, "scope": "shared:scatterhoard"}',
                        help="retry: the refused call's arguments, as JSON")
    parser.add_argument("--result-file", help="retry: test this refusal text instead of the served one")
    parser.add_argument("--recovered-by", default="query,keywords",
                        help="retry: arguments, any of which in the next call counts as recovering")
    parser.add_argument("--tasks", help="agent: comma-separated task names (default: all)")
    parser.add_argument("--tools", type=lambda s: s.split(","), default=READ_ONLY,
                        help="agent: tools the model may use (default: the read-only three)")
    parser.add_argument("--max-turns", type=int, default=8)
    parser.add_argument("--groups", action="store_true",
                        help="classify: ask about each realistic review group in the tasks file's `groups` together")
    parser.add_argument("--context", type=int, default=0,
                        help="classify --final, one at a time: show up to this many already-cleared memories "
                             "from the same subject beside the one under review")
    parser.add_argument("--final", action="store_true",
                        help="classify: the proposed review, with the whole file, numbered sentences across "
                             "text, reasoning and context, every mentioned item reported, and the server "
                             "dropping shared-only items for local and never-published sets")
    parser.add_argument("--says", action="store_true",
                        help="classify --final with the 2026-10-25 additions: whether a sentence about a person "
                             "reports or judges, the keep-outs for someone else, and the server's routing of both")
    parser.add_argument("--merged", action="store_true",
                        help="classify --says with one required label per sentence, joining who it is about "
                             "and whether it reports or judges")
    parser.add_argument("--listed", action="store_true",
                        help="classify: give each memory the items kept out of its set, read from the headings, "
                             "and ask per numbered sentence which it mentions")
    parser.add_argument("--sentences", action="store_true",
                        help="classify: split each memory into numbered sentences and ask about each")
    parser.add_argument("--batch", action="store_true",
                        help="classify: ask about every memory in one prompt rather than one at a time")
    parser.add_argument("--debrief", help="agent: after each run, ask the questions this names under `debrief` in the tasks file")
    args = parser.parse_args()

    out = Path(args.out) if args.out else Path(tempfile.mkdtemp(prefix="scatterhoard-local-model-"))
    out.mkdir(parents=True, exist_ok=True)
    print(f"logging to {out}\n")
    {"page": page_mode, "retry": retry_mode, "agent": agent_mode, "classify": classify_mode}[args.mode](args, out)


if __name__ == "__main__":
    main()
