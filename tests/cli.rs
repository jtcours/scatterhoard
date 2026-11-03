// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! Contract tests for the binary's command line, run against the built binary itself.
//!
//! `prohibited-scan-subcommand` in `TRACKING.md`. The binary took no arguments at all until this
//! existed, and every attached client still launches it bare, by absolute path. **So the first test
//! here pins that a bare invocation serves MCP**, and it was written before any argument handling: a
//! wrong default does not produce an error anyone reads, it produces clients that fail to start, and
//! that is diagnosed as a broken install long before anyone suspects argument parsing.
//!
//! `scan` reaches the prohibited-data scan from outside a running server, so a pre-commit hook can
//! catch a hand-edited memory file that no tool ever saw. It scans raw lines rather than a parsed
//! set, because the file a hook meets may not parse -- a merge left with conflict markers is the
//! likeliest -- and a secret in a broken file is still a secret.
//!
//! `export` reaches the same export a session runs, so a set can be backed up on a schedule, and
//! `merge-file` is the `git` merge driver. Both are pinned here by exit status as much as by effect:
//! a scheduled backup and a `git` merge are each watched by a caller that reads nothing else.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Every credential here is fabricated, and the same ones `tests/prohibited.rs` uses: real vendor
/// prefixes, invented bodies.
const FAKE_GITHUB: &str = "ghp_016c75a7f9b24e0d8c3aa15be92d4f7c1a8e";
const FAKE_AWS: &str = "AKIAIOSFODNN7EXAMPLE";

struct TempDir(PathBuf);

impl TempDir {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn tempdir() -> TempDir {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("after the epoch")
        .as_nanos();
    let unique = format!("{nanos}-{:?}", std::thread::current().id())
        .replace(['(', ')', ' '], "")
        .replace("ThreadId", "t");
    let path = std::env::temp_dir().join(format!("scatterhoard-cli-{unique}"));
    std::fs::create_dir_all(&path).expect("creates a scratch directory");
    TempDir(path)
}

/// The binary, run the way a client or a hook runs it: no inherited Scatterhoard environment, a
/// home that belongs to this test, and a model path that loads nothing.
fn binary(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_scatterhoard"));
    command
        .env_remove("SCATTERHOARD_COMMON")
        .env_remove("SCATTERHOARD_AUTHOR")
        .env("SCATTERHOARD_HOME", home)
        .env("SCATTERHOARD_MODEL", home.join("no-model-here"));
    command
}

/// Run with these arguments, feeding `stdin` and closing it.
fn run(home: &Path, args: &[&str], stdin: &str) -> Output {
    let mut child = binary(home)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the binary starts");
    child
        .stdin
        .take()
        .expect("a stdin pipe")
        .write_all(stdin.as_bytes())
        .expect("writes stdin");
    child.wait_with_output().expect("the binary finishes")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A memory file's header and one record per content, in the shape an export writes.
fn a_memory_file(dir: &Path, name: &str, contents: &[&str]) -> PathBuf {
    let mut text =
        String::from("{\"format_version\":\"0.4.0\",\"scope\":{\"shared\":\"example\"}}\n");
    for (n, content) in contents.iter().enumerate() {
        text.push_str(&format!(
            "{{\"id\":\"01M2H0000000000000000000{n:02}\",\"content\":\"{content}\"}}\n"
        ));
    }
    let path = dir.join(name);
    std::fs::write(&path, text).expect("writes the fixture");
    path
}

// ---------------------------------------------------------------------------
// The bare invocation
// ---------------------------------------------------------------------------

/// **Pinned before any argument handling existed.** Every attached client launches the binary with
/// no arguments, so a bare invocation has to go on serving MCP whatever subcommands are added.
/// A tester's bug report needs the build's version, and the command line is where a person asks.
#[test]
fn version_prints_the_crate_version() {
    let home = tempdir();
    for flag in ["--version", "version", "-V"] {
        let output = run(home.path(), &[flag], "");
        assert!(output.status.success(), "{flag}: {}", stderr(&output));
        assert_eq!(
            stdout(&output).trim(),
            format!("scatterhoard {}", env!("CARGO_PKG_VERSION")),
            "{flag}"
        );
    }
}

#[test]
fn a_bare_invocation_serves_mcp() {
    let home = tempdir();
    let output = run(
        home.path(),
        &[],
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
    );

    assert!(output.status.success(), "stderr: {}", stderr(&output));
    let response = stdout(&output);
    assert!(
        response.contains("serverInfo") && response.contains("scatterhoard"),
        "a bare invocation should answer initialize over stdio, and said: {response}"
    );
}

/// An argument nothing recognises is refused, not ignored. Ignoring it would start a server that
/// waits on stdin, which from a shell or a hook looks like a hang.
#[test]
fn an_unknown_subcommand_is_refused_rather_than_served() {
    let home = tempdir();
    let output = run(home.path(), &["serve-please"], "");

    assert_eq!(output.status.code(), Some(2), "stdout: {}", stdout(&output));
    assert!(
        stderr(&output).contains("usage"),
        "the refusal should say how the binary is used: {}",
        stderr(&output)
    );
    assert!(
        stdout(&output).is_empty(),
        "nothing goes to stdout, which is the protocol channel when serving"
    );
}

// ---------------------------------------------------------------------------
// scan
// ---------------------------------------------------------------------------

#[test]
fn scanning_a_clean_file_exits_zero() {
    let home = tempdir();
    let file = a_memory_file(home.path(), "clean.json", &["a memory about API keys"]);

    let output = run(home.path(), &["scan", file.to_str().unwrap()], "");

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
}

/// **The case the subcommand exists for**: a credential in a file no tool wrote. It exits non-zero so
/// a hook stops the commit, and it names the line and the category -- **never the credential**,
/// since hook output lands in terminals and CI logs, which is spreading the thing the scan contains.
#[test]
fn scanning_a_file_holding_a_key_names_the_line_and_never_the_key() {
    let home = tempdir();
    let file = a_memory_file(
        home.path(),
        "leaky.json",
        &[
            "nothing here",
            &format!("the deploy step needs {FAKE_GITHUB} exported first"),
        ],
    );

    let output = run(home.path(), &["scan", file.to_str().unwrap()], "");

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let report = stdout(&output);
    assert!(
        report.contains(":3:"),
        "the second record is line 3, after the header: {report}"
    );
    assert!(report.contains("api_key"), "{report}");
    assert!(
        !report.contains(FAKE_GITHUB) && !stderr(&output).contains(FAKE_GITHUB),
        "the credential must not be reproduced anywhere in the output"
    );
}

/// The file a hook meets may not parse, and a merge left with conflict markers is the likeliest.
#[test]
fn a_file_that_is_not_a_valid_memory_file_is_still_scanned() {
    let home = tempdir();
    let path = home.path().join("conflicted.json");
    std::fs::write(
        &path,
        format!("<<<<<<< HEAD\nkeyed with {FAKE_AWS}\n=======\n>>>>>>> theirs\n"),
    )
    .expect("writes");

    let output = run(home.path(), &["scan", path.to_str().unwrap()], "");

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stdout(&output).contains(":2:"), "{}", stdout(&output));
}

/// A hook scans what is being committed, which is the staged blob rather than the working file:
/// `git show :path | scatterhoard scan -`.
#[test]
fn a_dash_scans_standard_input() {
    let home = tempdir();
    let output = run(
        home.path(),
        &["scan", "-"],
        &format!("a header\nkeyed with {FAKE_AWS}\n"),
    );

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    assert!(stdout(&output).contains(":2:"), "{}", stdout(&output));
}

/// Several files in one call, as a hook passing everything staged would. Any finding fails the run,
/// and each is prefixed with the file it came from.
#[test]
fn scanning_several_files_reports_each_and_fails_on_any() {
    let home = tempdir();
    let clean = a_memory_file(home.path(), "clean.json", &["nothing here"]);
    let leaky = a_memory_file(home.path(), "leaky.json", &[&format!("uses {FAKE_AWS}")]);

    let output = run(
        home.path(),
        &["scan", clean.to_str().unwrap(), leaky.to_str().unwrap()],
        "",
    );

    assert_eq!(output.status.code(), Some(1), "{}", stderr(&output));
    let report = stdout(&output);
    assert!(
        report.contains(&format!("{}:2:", leaky.display())),
        "{report}"
    );
    assert!(
        !report.contains(&format!("{}:", clean.display())),
        "a clean file contributes no finding lines: {report}"
    );
}

/// A file that cannot be read is a different failure from a finding, so a hook can tell *the scan
/// found something* from *the scan could not run*.
#[test]
fn a_file_that_cannot_be_read_exits_two() {
    let home = tempdir();
    let missing = home.path().join("not-there.json");

    let output = run(home.path(), &["scan", missing.to_str().unwrap()], "");

    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("not-there.json"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn scan_with_nothing_to_scan_exits_two_with_usage() {
    let home = tempdir();
    let output = run(home.path(), &["scan"], "");

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("usage"), "{}", stderr(&output));
}

// ---------------------------------------------------------------------------
// export
// ---------------------------------------------------------------------------
//
// `export-subcommand`: export could only be reached from inside a session, so a set could not be
// backed up on a schedule. A scheduled job watches an exit status and reads nothing, so every
// refusal here has to be non-zero as well as legible.

/// Store a memory in the local set by serving one session, the way a client would.
fn memorize_local(home: &Path, content: &str) -> Output {
    let requests = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{}}}}\n\
         {{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\
         \"hoard_memorize\",\"arguments\":{{\"scope\":\"local\",\"content\":\"{content}\",\
         \"why\":\"a fixture\",\"confidence\":0.8,\"origin\":\"llm\"}}}}}}\n"
    );
    run(home, &[], &requests)
}

/// The case the subcommand exists for: a backup that no session had to be open for.
#[test]
fn exporting_a_set_writes_its_memory_file() {
    let home = tempdir();
    let stored = memorize_local(home.path(), "a memory to back up");
    assert!(stored.status.success(), "stderr: {}", stderr(&stored));
    let destination = home.path().join("backup");
    std::fs::create_dir_all(&destination).expect("creates the destination");

    let output = run(
        home.path(),
        &[
            "export",
            "local",
            "--file",
            destination.join("local.json").to_str().unwrap(),
        ],
        "",
    );

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let written = destination.join("local.json");
    assert!(
        written.exists(),
        "nothing was written to {}",
        written.display()
    );
    let text = std::fs::read_to_string(&written).expect("reads the export");
    assert!(
        text.contains("a memory to back up"),
        "the export should carry the set's memories: {text}"
    );
}

/// An exact file, for a backup or a synced folder, where the well-known path would not be wanted.
#[test]
fn exporting_to_an_exact_file_writes_there_and_nowhere_else() {
    let home = tempdir();
    let stored = memorize_local(home.path(), "a memory for the file form");
    assert!(stored.status.success(), "stderr: {}", stderr(&stored));
    let file = home.path().join("backup.json");

    let output = run(
        home.path(),
        &["export", "local", "--file", file.to_str().unwrap()],
        "",
    );

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(file.exists(), "nothing was written to {}", file.display());
    assert!(
        !home.path().join("scatterhoard_mem").exists(),
        "the file form must not also write the well-known path"
    );
}

/// **The refusal a scheduled backup has to be able to notice.** A destination that does not exist is
/// the likeliest way a cron entry rots -- a removed volume, a renamed directory -- and the job sees
/// only the exit status.
#[test]
fn exporting_to_a_directory_that_does_not_exist_exits_non_zero() {
    let home = tempdir();
    let missing = home.path().join("no-such-directory");

    let output = run(
        home.path(),
        &["export", "local", "--directory", missing.to_str().unwrap()],
        "",
    );

    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("no-such-directory"),
        "the refusal should name the destination: {}",
        stderr(&output)
    );
}

/// A set this machine does not hold is a refusal rather than an empty file, for the same reason:
/// a backup of nothing that reports success is worse than no backup.
#[test]
fn exporting_a_set_this_machine_does_not_hold_exits_non_zero() {
    let home = tempdir();
    let destination = home.path().join("backup");
    std::fs::create_dir_all(&destination).expect("creates the destination");

    let output = run(
        home.path(),
        &[
            "export",
            "shared:not-here",
            "--directory",
            destination.to_str().unwrap(),
        ],
        "",
    );

    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("not-here"),
        "the refusal should name the set: {}",
        stderr(&output)
    );
}

#[test]
fn export_without_a_destination_exits_two_with_usage() {
    let home = tempdir();
    let output = run(home.path(), &["export", "local"], "");

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("usage"), "{}", stderr(&output));
}

#[test]
fn export_without_a_scope_exits_two_with_usage() {
    let home = tempdir();
    let output = run(home.path(), &["export"], "");

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("usage"), "{}", stderr(&output));
}

// ---------------------------------------------------------------------------
// merge-file
// ---------------------------------------------------------------------------
//
// `git-merge-driver`. The merge itself is covered by `tests/merge.rs`; what is pinned here is the
// driver contract -- three paths in `git`'s order, the result written over OURS, and an exit status
// `git` can read.

/// A record line with the fields a memory file requires, spelled by hand so the fixture does not
/// depend on the writer it is checking.
fn a_record(id: u8, content: &str, confidence: f64, keywords: &str, modified: &str) -> String {
    format!(
        "{{\"id\":\"01M2H0000000000000000000{id:02}\",\"content\":\"{content}\",\
         \"confidence\":{confidence},\"origin\":\"llm\",{keywords}\
         \"created\":{{\"by\":\"01M2H000000000000000000099\",\"at\":\"2026-08-01T09:00:00Z\"}},\
         \"modified\":{{\"by\":\"01M2H000000000000000000099\",\"at\":\"{modified}\"}}}}"
    )
}

fn a_side(dir: &Path, name: &str, records: &[String]) -> PathBuf {
    let mut text =
        String::from("{\"format_version\":\"0.4.0\",\"scope\":{\"shared\":\"example\"}}\n");
    for record in records {
        text.push_str(record);
        text.push('\n');
    }
    let path = dir.join(name);
    std::fs::write(&path, text).expect("writes a side");
    path
}

/// The case the driver exists for, end to end: compatible edits to one record, which plain `git`
/// would have reported as a line conflict.
#[test]
fn merging_compatible_edits_exits_zero_and_writes_over_ours() {
    let home = tempdir();
    let base = a_side(
        home.path(),
        "base.json",
        &[a_record(1, "the claim", 0.8, "", "2026-08-01T09:00:00Z")],
    );
    let ours = a_side(
        home.path(),
        "ours.json",
        &[a_record(1, "the claim", 0.95, "", "2026-09-01T10:00:00Z")],
    );
    let theirs = a_side(
        home.path(),
        "theirs.json",
        &[a_record(
            1,
            "the claim",
            0.8,
            "\"keywords\":[\"added\"],",
            "2026-09-02T10:00:00Z",
        )],
    );

    let output = run(
        home.path(),
        &[
            "merge-file",
            base.to_str().unwrap(),
            ours.to_str().unwrap(),
            theirs.to_str().unwrap(),
        ],
        "",
    );

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let merged = std::fs::read_to_string(&ours).expect("ours was written");
    assert!(merged.contains("0.95"), "our confidence was lost: {merged}");
    assert!(merged.contains("added"), "their keyword was lost: {merged}");
}

/// **A side out of file order still merges, and the driver says which and what ends it.** Records
/// `01` and `02` hash in the reverse of their id order, so a side listing them by id — as every file
/// written before 2026-09-21 does — is out of file order and is read whole rather than walked in
/// step. Nothing else is left in the directory: the result is written beside `ours` and renamed.
#[test]
fn merging_a_side_out_of_file_order_names_it_and_leaves_nothing_behind() {
    let home = tempdir();
    let one = a_record(1, "first", 0.8, "", "2026-08-01T09:00:00Z");
    let two = a_record(2, "second", 0.8, "", "2026-08-01T09:00:00Z");
    let base = a_side(home.path(), "base.json", &[one.clone(), two.clone()]);
    let ours = a_side(home.path(), "ours.json", &[two.clone(), one.clone()]);
    let theirs = a_side(home.path(), "theirs.json", &[two, one]);

    let output = run(
        home.path(),
        &[
            "merge-file",
            base.to_str().unwrap(),
            ours.to_str().unwrap(),
            theirs.to_str().unwrap(),
        ],
        "",
    );

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    let said = stderr(&output);
    assert!(
        said.contains("base") && said.contains("file order") && said.contains("export"),
        "the driver should name the side and say exporting again ends it: {said}"
    );
    let mut left: Vec<String> = std::fs::read_dir(home.path())
        .expect("reads the directory")
        .map(|entry| {
            entry
                .expect("an entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .filter(|name| name.contains("json"))
        .collect();
    left.sort();
    assert_eq!(left, ["base.json", "ours.json", "theirs.json"]);
}

/// A genuine divergence exits non-zero so `git` marks the merge failed, names the record, and still
/// leaves a file that parses.
#[test]
fn merging_a_divergence_exits_non_zero_and_names_the_record() {
    let home = tempdir();
    let base = a_side(
        home.path(),
        "base.json",
        &[a_record(1, "the claim", 0.8, "", "2026-08-01T09:00:00Z")],
    );
    let ours = a_side(
        home.path(),
        "ours.json",
        &[a_record(
            1,
            "our correction",
            0.8,
            "",
            "2026-09-01T10:00:00Z",
        )],
    );
    let theirs = a_side(
        home.path(),
        "theirs.json",
        &[a_record(
            1,
            "their correction",
            0.8,
            "",
            "2026-09-02T10:00:00Z",
        )],
    );

    let output = run(
        home.path(),
        &[
            "merge-file",
            base.to_str().unwrap(),
            ours.to_str().unwrap(),
            theirs.to_str().unwrap(),
        ],
        "",
    );

    assert_eq!(output.status.code(), Some(1), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("01M2H000000000000000000001"),
        "the refusal should name the diverged record: {}",
        stderr(&output)
    );
    let left = std::fs::read_to_string(&ours).expect("ours was written");
    assert!(
        left.contains("our correction"),
        "a conflicted record keeps our side: {left}"
    );
}

/// A path that cannot be read is a different failure from a conflict, so a person reading the
/// terminal can tell a broken driver line from an honest divergence.
#[test]
fn merging_with_an_unreadable_side_exits_two() {
    let home = tempdir();
    let base = a_side(
        home.path(),
        "base.json",
        &[a_record(1, "the claim", 0.8, "", "2026-08-01T09:00:00Z")],
    );
    let missing = home.path().join("not-there.json");

    let output = run(
        home.path(),
        &[
            "merge-file",
            base.to_str().unwrap(),
            missing.to_str().unwrap(),
            base.to_str().unwrap(),
        ],
        "",
    );

    assert_eq!(output.status.code(), Some(2), "{}", stdout(&output));
    assert!(
        stderr(&output).contains("not-there.json"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn merge_file_without_three_paths_is_refused() {
    let home = tempdir();
    let output = run(home.path(), &["merge-file", "one", "two"], "");

    assert_eq!(output.status.code(), Some(2));
    assert!(stderr(&output).contains("usage"), "{}", stderr(&output));
}

/// **Scanning touches no store.** A hook runs on a machine that may have no Scatterhoard home at all,
/// and a scan that created one -- or opened a set the MCP servers hold -- would be a side effect
/// nobody asked a pre-commit check for.
#[test]
fn scanning_creates_no_store() {
    let home = tempdir();
    let unused_home = home.path().join("never-created");
    let file = a_memory_file(home.path(), "clean.json", &["nothing here"]);

    let output = Command::new(env!("CARGO_BIN_EXE_scatterhoard"))
        .env_remove("SCATTERHOARD_COMMON")
        .env("SCATTERHOARD_HOME", &unused_home)
        .env("SCATTERHOARD_MODEL", home.path().join("no-model-here"))
        .args(["scan", file.to_str().unwrap()])
        .stdin(Stdio::null())
        .output()
        .expect("runs");

    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));
    assert!(
        !unused_home.exists(),
        "a scan must not create a Scatterhoard home"
    );
}
