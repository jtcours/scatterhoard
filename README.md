# Scatterhoard

Scatterhoard is a memory system for AI agents. You can easily create sets of memories, load them, and export them as JSON files.

A contributor can clone a project that has a memory set, load the memories, make changes, and push the changes and new memories together in a pull request so maintainers can review both together before merging both into the project. 

# Features:
- Use multiple memory sets simultaneously, each covering a different topic.
- Gather observations and investigate issues across sessions, commits, and contributors.
- Memory sets export as human-reviewable JSON files.
- Fully self-contained: does not need network access to run.
- Multilingual: supports several languages out of the box, and can handle others.
- Supports regular *read-write* memory sets, *read-only* memory sets for distributing documentation (like the [`scatterhoard-cookbook`](scatterhoard_mem/scatterhoard-cookbook.json)) and *evaluation mode* for exploring a memory set you're reviewing.
- Runs as a stdio-based MCP server with low context load and fast lookups.
- Sharing preferences manage things about you that you don't want in the memories. Built-in rules manage memories about other people.

# Quick Start

Requires [Rust](https://rust-lang.org/tools/install/). 

Point your agent at [SETUP.md](SETUP.md), or follow these manual steps if you prefer:
1. Clone this repository:
```
git clone https://github.com/jtcours/scatterhoard.git scatterhoard
cd scatterhoard
```
2. Windows users: decide where your home directory will be and set `SCATTERHOARD_HOME` to point to it in the environment variables the MCP server will receive. For operating systems that define a `HOME`, `SCATTERHOARD_HOME` defaults to `$HOME/.local/share/scatterhoard/`.
3. Download the embedding model. Save these three files into `~/.local/share/scatterhoard/model`
   (on Windows, into a `model` folder inside the directory you set as `SCATTERHOARD_HOME`):
   - [config.json](https://huggingface.co/minishlab/potion-base-32M/resolve/main/config.json)
   - [tokenizer.json](https://huggingface.co/minishlab/potion-base-32M/resolve/main/tokenizer.json)
   - [model.safetensors](https://huggingface.co/minishlab/potion-base-32M/resolve/main/model.safetensors) (129 MB)

   For memories in languages other than English, use [potion-multilingual-128M](https://huggingface.co/minishlab/potion-multilingual-128M/tree/main) instead: the same three files, from its page.
4. Compile Scatterhoard
```
cargo build --release
```
5. Add it as an MCP server to your preferred development environment. You may need to configure the server's absolute path.
6. Add this to your project’s AGENTS.md, or CLAUDE.md for Claude Code, or whatever file your agent loads at the start of a session.:
```
**When the Scatterhoard tools are available in this conversation, call `hoard_conventions` before producing any visible text.** It is the first action of the session. Take no arguments, and the whole catalog arrives in one page; if you do pass `max_chars`, keep calling as each page's last line says until it says every convention is loaded. Load them again whenever the conversation has been summarized or compacted since you last did: a summary standing in for earlier turns means the catalog is no longer in front of you.
```
7. In each repository that carries a memory set, optionally connect Scatterhoard as a `git` merge driver.
```
echo '**/scatterhoard_mem/*.json merge=scatterhoard' >> .gitattributes
git config merge.scatterhoard.driver '/path/to/scatterhoard/target/release/scatterhoard merge-file %O %A %B; s=$?; [ $s -eq 127 ] || exit $s; echo "scatterhoard: merge driver not found; falling back to git merge-file" >&2; git merge-file -L ours -L base -L theirs %A %O %B'
```
8. At the start of your first session, Scatterhoard creates `sharing-preferences.md` in Scatterhoard’s data directory (`~/.local/share/scatterhoard/` by default; on Windows, the directory you set as `SCATTERHOARD_HOME`). Optionally edit it to say what about you may be stored or shared.

# Other Useful Things
- The [`scatterhoard-cookbook`](scatterhoard_mem/scatterhoard-cookbook.json) is a read-only memory set containing Scatterhoard tips and tricks.
- The [`coding-lessons`](https://github.com/jtcours/coding-lessons) project contains broader lessons learned while working on Scatterhoard. It's designed to be useful across many code bases.
- This project currently uses [`TRACKING.md`](TRACKING.md) and [`scripts/genindex.py`](scripts/genindex.py) to track its work. They leverage the memory system. Copy and modify them if you're running a small or solo project and need an issue tracker.

# Reporting Problems
Report problems in Github's issues system. Please include the output of `scatterhoard --version`.

# FAQ
## Why would I want it?
An AI agent starts each context fresh and tries to use reasoning, maybe with some notes in markdown files, to figure out what to do. But reasoning only gets you so far, and markdown files eventually get too large to manage.

A memory set can provide the information that's missing, not only what decisions people made in the past, but *why* they made them, so the agent doesn't accidentally bulldoze that careful compromise you worked out three months ago. And because the memories clone along with the project, a new contributor can learn about those decisions from the agent.

## Does it work for non-code things?
Yes. Absolutely. For example, if you want a set of memories about paleontological research or recipes from the Internet, you can create it and even share it. About the only thing it's not designed to handle is memories about people.

## How does it work?
You connect Scatterhoard to your AI agent and add the bootstrap prompt in [Quick Start](#quick-start). The AI agent manages the memory system for you. Each memory set has a charter that says what sorts of memories belong in it. If you tell the agent, "create a new memory set about recipes from the Internet," it will do that and start capturing whatever you tell it about recipies from the Internet, like the recipes themselves, how well they worked, and which ones required a ton of cleanup.

## No, I mean at a technical level how does it work?
Each memory set loads into a vector database built on [SQLite](https://sqlite.org/) (through [rusqlite](https://github.com/rusqlite/rusqlite)) with an open-weight embedding model you download from a source like [Hugging Face](https://huggingface.co).

A memory is a small block of text. Memories have both keywords and links to other memories. Recall can be semantic (land on a memory through semantic similarity, then walk the links to get adjacent ones), by keyword (useful for broader searches and fancy tricks like gathering observations and flagging recall topics), or directly by memory ID.

Memory sets export to a JSON format that is designed for easy review and merging. You can download the exported set of memories from developing Scatterhoard [here](scatterhoard_mem/scatterhoard.json) (it's too large to view in Github) or see
the Scatterhoard Cookbook memories [here](scatterhoard_mem/scatterhoard-cookbook.json).

The `hoard_conventions` command the agent runs at the start of the session returns a set of prompts telling the agent how to use the tools and when and where to record new memories.

See the [design documents](documents/design/) for details.

## Why isn't there a downloadable installer?

It's still early in the project. Memory server first, then installers if it catches on.

## You vibe coded this!
"Team effort" would be more accurate.

In 2002, a hundred male GM employees dressed up in skirts made from plastic garbage bags, put on high-heeled shoes and long fake fingernails, carried a purse and a baby doll, and spent time around GM's SUVs [trying to make them more appealing to female buyers](https://www.autonews.com/article/20060904/ANE/60901086/how-to-make-big-suvs-more-female-friendly/). Surprisingly, that approach achieved some success, but you have to wonder how much better they would have done if they'd simply had more women on the design team.

Scatterhoard is a tool for AI agents. For me to try to imagine what would work well for an AI agent would have been the equivalent of dressing up in a skirt made from a plastic garbage bag. The better answer was to have an AI agent on the team.

## About the name...?
[Scatter hoarding](https://en.wikipedia.org/wiki/Hoarding_(animal_behavior)#Hoard_distribution_and_size) is how some animals store food for the winter. Instead of putting all their food in one big hoard, they scatter small hoards around, similar to the way Scatterhoard stores memories.
