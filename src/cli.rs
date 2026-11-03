// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The binary's command line.
//!
//! **A bare invocation serves MCP, and must go on doing so.** Every attached client launches the
//! binary with no arguments, by absolute path, and a wrong default here does not produce an error
//! anyone reads: it produces clients that fail to start, diagnosed as a broken install long before
//! anyone suspects argument parsing. `tests/cli.rs` pins it, and was written before this module.
//!
//! **Subcommands, so the next one has somewhere to go.** `scan` was the first, then `export` and
//! `merge-file`. An argument nothing recognises is refused rather than ignored, because ignoring it
//! would start a server waiting on stdin, which from a shell or a hook looks like a hang.
//!
//! **Two argument styles, and the difference is who is calling.** `export` takes a flagged
//! destination because a person writes it and the two forms are not interchangeable; `merge-file`
//! takes three positional paths because `git` writes it, as `%O %A %B` in a driver line, where order
//! is the convention and a flag is one more thing to get wrong in a config nobody re-reads.
//!
//! Kept in the library rather than in `main.rs`, which is transport and wiring only.

use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::PathBuf;

use crate::mcp::Destination;
use crate::memory::Scope;
use crate::prohibited;

/// How the binary is run, printed for `help` and after a refused invocation.
pub const USAGE: &str = "\
usage: scatterhoard              serve MCP over stdio -- how every client runs it
       scatterhoard scan FILE... scan memory files for prohibited data; `-` reads standard input
       scatterhoard export SCOPE --directory DIR   write the set's memory file under DIR
       scatterhoard export SCOPE --file PATH       write it to exactly PATH, for a backup
                                 SCOPE is `local`, or `shared:<name>`
       scatterhoard merge-file BASE OURS THEIRS    `git` merge driver; result replaces OURS
       scatterhoard --version                      print this build's version
";

/// Exit status when a scan found prohibited data.
pub const FOUND: u8 = 1;

/// Exit status when a merge left conflicts, which is what `git` reads to mark the merge failed.
///
/// **Deliberately the same value as [`FOUND`]**, and named separately rather than reused: both mean
/// *the command ran and the answer is no*, but a scan finding a credential and a merge finding a
/// divergence are different facts, and a constant documented for one of them would be wrong at the
/// other's call site. `git` itself distinguishes only zero from non-zero.
pub const CONFLICTED: u8 = 1;

/// Exit status when the command could not do what was asked: a bad invocation, or a file that could
/// not be read. Distinct from [`FOUND`], so a hook can tell *the scan found something* from *the
/// scan could not run*.
pub const COULD_NOT_RUN: u8 = 2;

/// What an invocation asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Invocation {
    /// No arguments: serve MCP over stdio.
    Serve,
    /// Scan these files, where `-` is standard input.
    Scan(Vec<OsString>),
    /// Write one set's memory file, for a backup nobody has to be in a session for.
    Export {
        /// The set to write.
        scope: Scope,
        /// Where to write it, in the same two forms the tool surface takes.
        destination: Destination,
    },
    /// Merge three versions of a memory file, as `git` invokes a merge driver.
    Merge {
        /// The merge base, `%O`.
        base: OsString,
        /// The receiving side, `%A` — read, and replaced with the result.
        ours: OsString,
        /// The side being merged in, `%B`.
        theirs: OsString,
    },
    /// Print the usage.
    Help,
    /// Print this build's version: what a bug report names, and what `serverInfo` and
    /// `hoard_conventions` report from the same constant.
    Version,
    /// An invocation that cannot be run, and why.
    Refused(String),
}

/// Read the arguments after the program name.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Invocation {
    let mut args = args.into_iter();
    let Some(first) = args.next() else {
        return Invocation::Serve;
    };
    match first.to_str() {
        Some("scan") => {
            let files: Vec<OsString> = args.collect();
            if files.is_empty() {
                Invocation::Refused(
                    "scan needs at least one file, or `-` for standard input".to_owned(),
                )
            } else {
                Invocation::Scan(files)
            }
        }
        Some("export") => parse_export(args),
        Some("merge-file") => parse_merge(args),
        Some("help" | "--help" | "-h") => Invocation::Help,
        Some("version" | "--version" | "-V") => Invocation::Version,
        _ => Invocation::Refused(format!("unknown command {first:?}")),
    }
}

/// `export SCOPE --directory DIR` or `export SCOPE --file PATH`.
///
/// **Every refusal here is a scheduled backup that did not happen**, so each says what was wrong
/// rather than printing usage alone -- a cron log holds the message and nothing else.
///
/// The destination is a flag rather than a second positional, because the two forms are not
/// interchangeable: a directory gets the well-known path appended, a file is written exactly as
/// given. Deciding between them by inspecting the path would guess at the one thing the caller is
/// the only one able to state -- the same reasoning that keeps them separate on the tool surface.
fn parse_export(args: impl Iterator<Item = OsString>) -> Invocation {
    let mut args = args;
    let Some(scope) = args.next() else {
        return Invocation::Refused("export needs a scope: `local`, or `shared:<name>`".to_owned());
    };
    let Some(key) = scope.to_str() else {
        return Invocation::Refused(format!(
            "a scope must be text: {scope:?} is not, and a scope is `local` or `shared:<name>`"
        ));
    };
    let scope = match Scope::from_key(key) {
        Ok(scope) => scope,
        Err(error) => return Invocation::Refused(format!("{key} is not a scope: {error}")),
    };

    let Some(flag) = args.next() else {
        return Invocation::Refused(
            "export needs a destination: `--directory DIR`, or `--file PATH`".to_owned(),
        );
    };
    let Some(path) = args.next() else {
        return Invocation::Refused(format!("{} needs a path after it", flag.to_string_lossy()));
    };
    if args.next().is_some() {
        return Invocation::Refused(
            "export takes one destination: `--directory DIR`, or `--file PATH`".to_owned(),
        );
    }

    match flag.to_str() {
        Some("--directory") => Invocation::Export {
            scope,
            destination: Destination::Directory(PathBuf::from(path)),
        },
        Some("--file") => Invocation::Export {
            scope,
            destination: Destination::File(PathBuf::from(path)),
        },
        _ => Invocation::Refused(format!(
            "unknown option {flag:?}; export takes `--directory DIR` or `--file PATH`"
        )),
    }
}

/// `merge-file BASE OURS THEIRS`, positional and in `git`'s own order.
///
/// **Positional rather than flagged, unlike `export`**, because the caller is `git` rather than a
/// person: the arguments come from a driver line in a `git` config — `%O %A %B` — where order is the
/// convention and a flag would be one more thing to get wrong in a config nobody re-reads.
fn parse_merge(args: impl Iterator<Item = OsString>) -> Invocation {
    let paths: Vec<OsString> = args.collect();
    let [base, ours, theirs] = paths.as_slice() else {
        return Invocation::Refused(format!(
            "merge-file takes exactly three paths -- base, ours, theirs -- and got {}. \
             `git` supplies them as %O %A %B.",
            paths.len()
        ));
    };
    Invocation::Merge {
        base: base.clone(),
        ours: ours.clone(),
        theirs: theirs.clone(),
    }
}

/// Merge three versions of a memory file, writing the result over `ours`, and return the exit status.
///
/// **The result replaces `ours` whether or not it merged cleanly**, which is what a `git` merge
/// driver is required to do: `%A` is where `git` looks for the answer, and the exit status is how it
/// learns whether the answer is final. A conflicted record keeps the receiving side's version, so
/// what lands there is always a memory file that parses — a file with conflict markers in it would
/// be unreadable to every tool here, the pre-commit scan included.
///
/// **A refusal is not a failed merge**: unreadable input exits [`COULD_NOT_RUN`], so a broken
/// invocation is distinguishable from an honest divergence. `git` treats both as failure, and a
/// person reading the terminal should not have to.
pub fn merge_file(base: &OsString, ours: &OsString, theirs: &OsString, err: &mut dyn Write) -> u8 {
    let opener = |path: &OsString| {
        let path = std::path::PathBuf::from(path);
        // The path goes into the error, since a broken driver line is found by which path it named.
        move || -> std::io::Result<Box<dyn std::io::Read>> {
            let file = std::fs::File::open(&path).map_err(|error| {
                std::io::Error::new(error.kind(), format!("{}: {error}", path.display()))
            })?;
            Ok(Box::new(file))
        }
    };
    let (open_base, open_ours, open_theirs) = (opener(base), opener(ours), opener(theirs));

    // **Written beside `ours` and renamed over it**, never written in place: `ours` is one of the
    // three sides and is still being read while the result is written. The same directory, so the
    // rename is atomic and a failed merge leaves `ours` as `git` handed it over.
    let ours_path = std::path::Path::new(ours);
    let mut staging_name = ours_path.as_os_str().to_owned();
    staging_name.push(".merging");
    let staging = std::path::PathBuf::from(staging_name);

    let outcome = std::fs::File::create(&staging)
        .map_err(crate::merge::MergeError::Unwritable)
        .and_then(|file| {
            let mut out = std::io::BufWriter::new(file);
            let outcome =
                crate::merge::merge_streams(&open_base, &open_ours, &open_theirs, &mut out)?;
            out.flush().map_err(crate::merge::MergeError::Unwritable)?;
            Ok(outcome)
        });
    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(error) => {
            let _ = std::fs::remove_file(&staging);
            let _ = writeln!(err, "scatterhoard: {error}");
            return COULD_NOT_RUN;
        }
    };

    if let Err(error) = std::fs::rename(&staging, ours_path) {
        let _ = std::fs::remove_file(&staging);
        let _ = writeln!(
            err,
            "scatterhoard: merged, but could not write {}: {error}",
            ours_path.display()
        );
        return COULD_NOT_RUN;
    }

    if let Some(side) = outcome.out_of_order {
        // Not a failure, and worth one line: this is the path that holds whole sets, and a person
        // whose merge was slow on a large set should learn both why and what ends it.
        let _ = writeln!(
            err,
            "scatterhoard: the {side} side is not in file order, so the merge read every side whole; \
             exporting the set again writes it in file order"
        );
    }

    if outcome.conflicts.is_empty() {
        return 0;
    }

    // Named, because the alternative is a person opening a file of several hundred records to find
    // out which one `git` is unhappy about. The receiving side's version is what sits there now.
    let _ = writeln!(
        err,
        "scatterhoard: {} record(s) diverged on both sides and were left as ours: {}",
        outcome.conflicts.len(),
        outcome
            .conflicts
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    CONFLICTED
}

/// Scan files for prohibited data, line by line, and return the exit status.
///
/// **Raw lines, not a parsed set.** The file a hook meets may not parse -- a merge left with conflict
/// markers is the likeliest -- and a secret in a broken file is still a secret. Where a line does
/// parse as a record, its memory id is named, since that is what `hoard_correct` takes.
///
/// **A finding names its line and category, never what matched**, for the reason
/// [`prohibited::Finding`] carries no copy: hook output lands in terminals and CI logs.
///
/// **Every file is scanned even after one cannot be read**, so a missing file does not hide a finding
/// in the next. An unreadable file outranks a finding in the exit status, because a hook told only
/// *found something* would not learn that part of what it asked about went unscanned.
pub fn scan(
    files: &[OsString],
    stdin: &mut dyn Read,
    out: &mut dyn Write,
    err: &mut dyn Write,
) -> u8 {
    let mut found = false;
    let mut unreadable = false;
    for file in files {
        let (name, text) = if file == "-" {
            let mut text = String::new();
            if let Err(error) = stdin.read_to_string(&mut text) {
                let _ = writeln!(err, "scatterhoard: could not read standard input: {error}");
                unreadable = true;
                continue;
            }
            ("-".to_owned(), text)
        } else {
            let path = std::path::Path::new(file);
            match std::fs::read(path) {
                Ok(bytes) => (
                    path.display().to_string(),
                    String::from_utf8_lossy(&bytes).into_owned(),
                ),
                Err(error) => {
                    let _ = writeln!(
                        err,
                        "scatterhoard: could not read {}: {error}",
                        path.display()
                    );
                    unreadable = true;
                    continue;
                }
            }
        };

        for (index, line) in text.lines().enumerate() {
            for finding in prohibited::scan(line) {
                found = true;
                let _ = writeln!(
                    out,
                    "{name}:{}: {}{}",
                    index + 1,
                    category_name(finding.category),
                    memory_named_by(line)
                );
            }
        }
    }

    if found {
        let _ = writeln!(
            err,
            "scatterhoard: prohibited data found. The lines above say where without reproducing it. \
             Forget each memory named with hoard_correct, then export the set again, rather than \
             editing the file: a hand-edit leaves the secret in the store to be exported next time."
        );
    }
    if unreadable {
        COULD_NOT_RUN
    } else if found {
        FOUND
    } else {
        0
    }
}

/// The category as the tools spell it, from its serde name rather than a second spelling here.
fn category_name(category: prohibited::Category) -> String {
    serde_json::to_value(category)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| format!("{category:?}"))
}

/// ` in memory <id>` where the line is a record carrying an id, and nothing otherwise.
fn memory_named_by(line: &str) -> String {
    serde_json::from_str::<serde_json::Value>(line)
        .ok()
        .and_then(|record| record["id"].as_str().map(|id| format!(" in memory {id}")))
        .unwrap_or_default()
}
