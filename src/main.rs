// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The Scatterhoard MCP server, speaking JSON-RPC over stdio.
//!
//! Deliberately thin. Every decision lives in [`scatterhoard::mcp`], which is a pure
//! request-to-response function; this file is the transport and the startup wiring, which is the
//! part `STANDARDS.md` names as legitimately exempt from test-first development.

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::Arc;

use scatterhoard::cli::{self, Invocation};
use scatterhoard::embedder::StaticEmbedder;
use scatterhoard::mcp::Server;
use scatterhoard::memory::{Author, Scope};
use scatterhoard::store::{Store, StoreLayout};

fn main() -> std::process::ExitCode {
    // Dispatched before anything touches a store or a model, so a subcommand like `scan` creates no
    // home and opens no set the MCP servers hold. No arguments is the one case that falls through to
    // serving, which is how every client launches this binary.
    match cli::parse(std::env::args_os().skip(1)) {
        Invocation::Serve => {}
        Invocation::Scan(files) => {
            return std::process::ExitCode::from(cli::scan(
                &files,
                &mut std::io::stdin(),
                &mut std::io::stdout(),
                &mut std::io::stderr(),
            ));
        }
        // Unlike `scan`, this one does touch a store -- it is the store it exists to write out --
        // and it opens the same files a running server may hold. SQLite in WAL mode is what makes
        // that safe between processes; see `Store::prepare_connection`.
        Invocation::Export { scope, destination } => {
            return match export(&scope, &destination) {
                Ok(report) => {
                    println!("{report}");
                    std::process::ExitCode::SUCCESS
                }
                // Non-zero, because a scheduled backup is watched by its exit status and read by
                // nobody. The message goes to stderr so a cron mail carries it.
                Err(message) => {
                    eprintln!("scatterhoard: {message}");
                    std::process::ExitCode::from(cli::COULD_NOT_RUN)
                }
            };
        }
        // Touches no store and loads no model, like `scan`: it merges three files and writes one.
        Invocation::Merge { base, ours, theirs } => {
            return std::process::ExitCode::from(cli::merge_file(
                &base,
                &ours,
                &theirs,
                &mut std::io::stderr(),
            ));
        }
        Invocation::Help => {
            print!("{}", cli::USAGE);
            return std::process::ExitCode::SUCCESS;
        }
        Invocation::Version => {
            println!("scatterhoard {}", env!("CARGO_PKG_VERSION"));
            return std::process::ExitCode::SUCCESS;
        }
        Invocation::Refused(reason) => {
            eprint!("scatterhoard: {reason}\n{}", cli::USAGE);
            return std::process::ExitCode::from(cli::COULD_NOT_RUN);
        }
    }

    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(message) => {
            // stderr, never stdout: stdout is the JSON-RPC channel, and a stray line on it
            // desynchronises the client.
            eprintln!("scatterhoard: {message}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut server = build_server()?;
    serve(&mut server)
}

/// Write one set's memory file and return the report, as JSON.
///
/// **The same server every session gets**, built the same way, so the export a schedule runs is the
/// export a session runs -- including the model, which is what lets the file carry a description of
/// the set rather than nothing.
fn export(scope: &Scope, destination: &scatterhoard::mcp::Destination) -> Result<String, String> {
    let mut server = build_server()?;
    let report = server.export_to(scope, destination)?;
    serde_json::to_string_pretty(&report).map_err(|error| format!("could not render: {error}"))
}

/// The wiring every invocation that needs stores shares.
fn build_server() -> Result<Server, String> {
    let mut server = Server::new();
    if let Some(name) = account_name() {
        server.set_account_name(name);
    }

    let layout = StoreLayout::new(data_directory()?, common_directory());
    for directory in [layout.home(), layout.common()] {
        std::fs::create_dir_all(directory)
            .map_err(|error| format!("could not create {}: {error}", directory.display()))?;
    }

    // The whole layout, not just a directory: `create` needs the home, and `status` needs to report
    // what scanning found. Machine-local state either way, so it comes from here rather than from a
    // caller — a set's name travels between machines, a path never does.
    server.set_layout(layout.clone());
    // Where `hoard_sets` action `backup` writes. Machine configuration, so a call can never choose it.
    if let Some(directory) = std::env::var_os("SCATTERHOARD_BACKUPS") {
        server.set_backup_directory(PathBuf::from(directory));
    }

    // Set on the server, which gives it to every store it holds now or is given later. Nothing here
    // has to remember to embed a store, which is what made `project-stores-get-no-embedder`
    // possible: this file gave the model to the local store and not to the shared ones, silently.
    //
    // Not fatal if absent. Keyword search, direct lookup and traversal do not need a model, and
    // recall announces the missing capability rather than pretending memory was consulted.
    match load_embedder() {
        Ok(embedder) => server.set_embedder(Arc::new(embedder)),
        Err(message) => eprintln!("scatterhoard: {message}"),
    }

    // One scan decides every store this server holds, including which copy wins for a set that is in
    // both directories. Nothing here chooses a path, so there is no second rule to keep in step with
    // the one under test — the local store is in the list like anything else, and is created on
    // demand if its file does not exist yet.
    for (scope, path) in layout.discover().stores {
        match Store::open_at(scope.clone(), &path) {
            Ok(store) => server.add_store(store),
            Err(error) if scope == Scope::Local => {
                return Err(format!("local store: {error}"));
            }
            Err(error) => eprintln!("scatterhoard: could not open {scope:?}: {error}"),
        }
    }

    Ok(server)
}

/// One JSON-RPC message per line, in and out.
fn serve(server: &mut Server) -> Result<(), String> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let line = line.map_err(|error| format!("could not read stdin: {error}"))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(response) = server.handle(&line) {
            writeln!(stdout, "{response}").map_err(|error| format!("could not write: {error}"))?;
            stdout
                .flush()
                .map_err(|error| format!("could not flush: {error}"))?;
        }
    }
    Ok(())
}

/// Where stores and the model live. Overridable, because a single fixed path is the first thing to
/// go wrong on a machine that puts things somewhere else.
fn data_directory() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("SCATTERHOARD_HOME") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").ok_or("neither SCATTERHOARD_HOME nor HOME is set")?;
    Ok(PathBuf::from(home).join(".local/share/scatterhoard"))
}

/// Where machine-bound stores live, when they are not to live in the home.
///
/// Optional, and absent is the ordinary case: set `SCATTERHOARD_HOME` per checkout and leave this
/// pointing at one place, and several servers get independent shared sets over one personal memory
/// and one copy of any set that belongs to no single checkout. Set neither and nothing about the
/// layout changes.
fn common_directory() -> Option<PathBuf> {
    std::env::var_os("SCATTERHOARD_COMMON").map(PathBuf::from)
}

/// The model is never fetched — this build cannot reach the network at all. See
/// `documents/design/embeddings.md` on why that is structural rather than a policy.
fn load_embedder() -> Result<StaticEmbedder, String> {
    let path = match std::env::var_os("SCATTERHOARD_MODEL") {
        Some(path) => PathBuf::from(path),
        None => data_directory()?.join("model"),
    };
    StaticEmbedder::load(&path).map_err(|error| error.to_string())
}

/// The operating system's name for the account this server runs as, if it supplies one.
///
/// **`local`'s default author, and nothing else's**, since `set-settled-at-first-write` (decided
/// 2026-09-24): a shared set asks who you are at its first write, because the name is how you are
/// known where that set is published, which varies by set. `local` is never published, so the account
/// name costs nothing there and saves every new user a step.
///
/// `USERNAME` on Windows, which does not normally set `USER` -- the reason a server there used to
/// attribute everything to `unknown` (`the-default-author-is-unknown-on-windows`). `USER`, then
/// `LOGNAME`, elsewhere. **No fallback name**: where the system supplies none, `local`'s first write
/// is refused like any other set's, rather than recorded under a name nobody chose.
/// `SCATTERHOARD_AUTHOR` is gone: every set now either asks or reads the system.
fn account_name() -> Option<Author> {
    let variables: &[&str] = if cfg!(windows) {
        &["USERNAME"]
    } else {
        &["USER", "LOGNAME"]
    };
    variables
        .iter()
        .filter_map(|variable| std::env::var(variable).ok())
        .find_map(|name| Author::new(name).ok())
}
