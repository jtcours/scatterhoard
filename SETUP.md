# Setting up Scatterhoard

Steps to install Scatterhoard and attach it to an MCP client, written so an AI agent can follow them.
Each step ends with a check. **If a check fails, stop and report what you saw** rather than continuing:
a later step cannot repair an earlier one.

Below, `<checkout>` means the absolute path of the cloned repository. Commands are for macOS and Linux;
Windows differences are called out where they occur.

## 1. Rust

```bash
cargo --version
```

If that fails, install Rust with the user's agreement. On macOS or Linux:

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && . "$HOME/.cargo/env"
```

On Windows, ask the user to run the installer from <https://rustup.rs>.

**Check:** `cargo --version` prints a version.

## 2. Clone and build

```bash
git clone https://github.com/jtcours/scatterhoard.git scatterhoard && cd scatterhoard && cargo build --release
```

The first build compiles every dependency and takes a few minutes.

**Check:** `<checkout>/target/release/scatterhoard` exists.

## 3. The embedding model

Scatterhoard never downloads anything itself, so fetch its model once, about 130 MB, into the `model`
folder of its data directory. It needs all three files: `config.json`, `tokenizer.json` and
`model.safetensors`.

On macOS and Linux:

```bash
D="$HOME/.local/share/scatterhoard/model"
mkdir -p "$D" && for f in config.json tokenizer.json model.safetensors; do curl -sSfL -o "$D/$f" "https://huggingface.co/minishlab/potion-base-32M/resolve/main/$f"; done
```

**On Windows**, Scatterhoard's data directory is whatever `SCATTERHOARD_HOME` names, because Windows
does not set `HOME`. Choose it now — such as `C:\Users\<you>\AppData\Roaming\scatterhoard` — since step
5 names it to every client as well. In PowerShell, writing `curl.exe` rather than `curl`, which
PowerShell treats as a different command (not yet verified on a Windows machine):

```powershell
$D = "C:\Users\<you>\AppData\Roaming\scatterhoard\model"
New-Item -ItemType Directory -Force $D | Out-Null; foreach ($f in "config.json","tokenizer.json","model.safetensors") { curl.exe -sSfL -o "$D\$f" "https://huggingface.co/minishlab/potion-base-32M/resolve/main/$f" }
```

- **For memories in a language other than English**, fetch `potion-multilingual-128M` (531 MB) into the
  same folder instead, from `https://huggingface.co/minishlab/potion-multilingual-128M/resolve/main/`.
  It ranks English less well. Once attached, `hoard_sets` action `status` scores each language a set
  declares, 0 to 10, against the model in use.
- **A model kept elsewhere** can be named with `SCATTERHOARD_MODEL`, set to the directory holding the
  three files.

**Check:** the folder holds `config.json`, `tokenizer.json` and `model.safetensors`.

## 4. Smoke test

```bash
printf '%s\n%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}' '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | <checkout>/target/release/scatterhoard
```

**Check:** `<checkout>/target/release/scatterhoard --version` prints the version you built, and the
command above returns two JSON lines, a `serverInfo` naming `scatterhoard` and a `tools` list of six,
and **nothing is printed on stderr**. A stderr line beginning `scatterhoard: no embedding model at`
names where the server looked; put the model there, or set `SCATTERHOARD_MODEL`.

## 5. Attach it to the client

**The command must be an absolute path**: the client does not start it from the checkout.

**Claude Code:**

```bash
claude mcp add --transport stdio --scope user scatterhoard -- <checkout>/target/release/scatterhoard
```

`--scope user` makes it available in every project. **Check:** `claude mcp list` shows `scatterhoard`
as connected. A session that was already running cannot see a newly added server, so start a new one.

**Other clients** take an entry in their MCP configuration file, under `mcpServers`:

```json
"scatterhoard": {
  "command": "<checkout>/target/release/scatterhoard"
}
```

For Claude Desktop that file is `~/Library/Application Support/Claude/claude_desktop_config.json` on
macOS, and `%APPDATA%\Claude\claude_desktop_config.json` on Windows. **Quit the app completely before
editing it, then relaunch**: the app rewrites the file from its own copy while it runs, which can undo
an edit.

**On Windows, every client needs `SCATTERHOARD_HOME`**, naming the directory chosen in step 3. Name the
same directory in every client, written out in full, since a client configuration does not expand
variables like `%APPDATA%`. In a configuration file:

```json
"scatterhoard": {
  "command": "C:\\path\\to\\scatterhoard\\target\\release\\scatterhoard.exe",
  "env": { "SCATTERHOARD_HOME": "C:\\Users\\you\\AppData\\Roaming\\scatterhoard" }
}
```

With Claude Code, add `--env SCATTERHOARD_HOME=<that directory>` before `scatterhoard` in the command
above.

## 6. Tell the project to load the conventions

Add this to the project's `AGENTS.md`, or `CLAUDE.md`, or whatever file its client loads at the start
of a session:

> **When the Scatterhoard tools are available in this conversation, call `hoard_conventions` before
> producing any visible text.** It is the first action of the session. Take no arguments, and the
> whole catalog arrives in one page; if you do pass `max_chars`, keep calling as each page's last line
> says until it says every convention is loaded. **Load them again whenever the conversation has been
> summarized or compacted since you last did**: a summary standing in for earlier turns means the
> catalog is no longer in front of you.

The conventions say which memories are worth storing, which set each belongs in, and when to search
before acting. A tool's own description is read only when a model reaches for the tool, which is too
late for the first store. If the project already names something to do first, say which comes first.

**Check:** in a new session, the agent's first action is `hoard_conventions`, and `hoard_sets` action
`status` lists the `local` set with `semantic_search_available: true`.

**Then read your sharing preferences.** The first time a session loads the conventions, Scatterhoard
writes `sharing-preferences.md` beside your personal memories: in `~/.local/share/scatterhoard/`, or in
the directory `SCATTERHOARD_COMMON` names, or `SCATTERHOARD_HOME` if only that is set. It lists what
about you is kept out of memory sets: some details out of shared sets only, such as your employer, and
some out of every set, such as your health. It can also list words never to be stored at all, values
that are always fine to share, and sets you never publish. Edit it to suit you: each item is a line
starting with `- `, and a line Scatterhoard does not read as an item is pointed out to your session,
by line number, at the start of the next one. It is served with the conventions at the start of every
session, so an edit takes effect at the next one, and Scatterhoard never changes the file again. If
you delete it, the template is written again at the next session start. It governs only your own details: what may be recorded about other
people stays with the conventions.

**Before a shared set is exported, its new memories are reviewed.** Export refuses memories nobody has
reviewed; `hoard_sets` action `review` shows them to your session, which checks each against your
preferences and brings you anything it flags. The review is done by your AI model and needs a capable
one: a small local model can miss details, or fail to finish a review while saying it has. If a missed
detail would be costly for you, read new memories yourself before you publish a set. A project can add to the review in `scatterhoard_mem/config/sharing-preferences.json`,
including `"audience": "private"` while its repository is not public.

**Backups.** `hoard_sets` action `backup` writes a safety copy of any set, never refused for want of a
review. Copies go under `backups/` in the same directory as your preferences unless you name another
with `SCATTERHOARD_BACKUPS` in the client's configuration, beside `SCATTERHOARD_HOME`, for example a
folder that is copied off the machine. Old backups are removed by hand.

## 7. Optional

**The cookbook.** `scatterhoard-cookbook` is a read-only set of recipes for using Scatterhoard: moving
your memories, recovering from a mistake, recording a model that suits a set. Take it on from the
checkout, in a session with the Scatterhoard tools:

```
hoard_sets  action: "create", name: "scatterhoard-cookbook", mode: "read-only"
hoard_sets  action: "import", scope: "shared:scatterhoard-cookbook", directory: "<checkout>"
```

The `create` response's note about `mint` and a charter does not apply to a read-only set; skip it.
**Check:** `hoard_sets` action `status` lists `shared:scatterhoard-cookbook` as `read-only`, with its
memories. After pulling a newer Scatterhoard, run the import again.

**Lessons across projects.** The [`coding-lessons`](https://github.com/jtcours/coding-lessons) set holds engineering lessons with triggers that
surface them before a mistake is written. Its repository's `README.md` gives the steps and the
paragraph for `AGENTS.md`.

**Field-aware merging of memory files.** In each repository that carries a memory set, tell `git`
which files to merge record by record, then register the driver for that clone:

```bash
echo '**/scatterhoard_mem/*.json merge=scatterhoard' >> .gitattributes
git config merge.scatterhoard.driver '<checkout>/target/release/scatterhoard merge-file %O %A %B; s=$?; [ $s -eq 127 ] || exit $s; echo "scatterhoard: merge driver not found; falling back to git merge-file" >&2; git merge-file -L ours -L base -L theirs %A %O %B'
```

Use the command exactly as given: the fallback is what stops a missing binary from silently dropping
the other branch's memories. A hosted service's merge button never runs the driver, so when a pull
request's memory file conflicts, merge the target branch into it locally and push.

**Check:** `git check-attr merge scatterhoard_mem/<set>.json` prints `merge: scatterhoard`.

**Scanning memory files before each commit.** `<checkout>/target/release/scatterhoard scan <file>`
exits `1` if a memory file holds something shaped like a credential, which matters for a file edited by
hand rather than exported. To run it on every commit, save this as `.git/hooks/pre-commit` in the
repository carrying the set, and make it executable:

```sh
#!/bin/sh
SCATTERHOARD=<checkout>/target/release/scatterhoard
staged=$(git diff --cached --name-only --diff-filter=ACM | grep -E '(^|/)scatterhoard_mem/[^/]*\.json$')
status=0
while IFS= read -r path; do
  [ -n "$path" ] || continue
  git show ":$path" | "$SCATTERHOARD" scan - || { echo "scatterhoard: in $path" >&2; status=1; }
done <<EOF
$staged
EOF
exit $status
```

## Updating

```bash
cd <checkout> && git pull && cargo build --release
```

Then restart every client, since each keeps running the binary it started with.

## If something goes wrong

- **The tools do not appear:** start a new session, and look for an approval prompt for the new server
  that may have been missed or declined. In Claude Code, `claude mcp list` shows whether it connected.
- **The client reports the server failed to start after a rebuild or a `cargo clean`:** `cargo clean`
  deletes the binary the client runs. Build again, then restart the client.
- **Your personal memories seem to have disappeared:** the `scatterhoard-cookbook` set has a recipe for
  it. `hoard_sets` action `status` names the file each set is read from.
