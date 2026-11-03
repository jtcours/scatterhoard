// ⚠️ IMPORTANT: ⚠️
// This source code is part of Scatterhoard. In every case, the Scatterhoard source code must
// conform to the Scatterhoard Vision. If you do not have the Scatterhoard Vision in context, load
// it before making any changes to this file. After you are done modifying the source code, before
// you commit your changes, check the Scatterhoard design documents and update them if necessary to
// keep them current.

//! The text the library serves, checked where it is written.
//!
//! `the-silenced-recall-note-carries-source-indentation`: five messages served to callers carried
//! runs of source indentation -- *a level calibrated                  from what the set has been
//! able to answer* -- so every one spent a caller's context on whitespace. They were found one at a
//! time, by reading a response, and a test per message would have missed the other four. So this
//! reads the library's source instead: **no string literal on one line may contain two spaces in a
//! row**, which is the shape a line break without its trailing `\` leaves behind once joined.
//!
//! The library only. `src/bin/` holds the evaluation harness, whose aligned columns are deliberate
//! and are read by a person at a terminal rather than by a model in a context window.

use std::path::{Path, PathBuf};

/// Every `.rs` file under `dir`, recursively, except the binaries.
fn library_sources(dir: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("reads the source directory") {
        let path = entry.expect("an entry").path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name == "bin") {
                continue;
            }
            library_sources(&path, found);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            found.push(path);
        }
    }
}

/// Every non-raw string literal in `source`, as the compiler would read it, with the line it opens
/// on.
///
/// **Read across lines, not line by line.** A first version read only literals that opened and
/// closed on one line, which all five known defects happened to do -- and a mutation breaking a
/// literal in its middle, where the line still ends in `\`, walked straight past it. So this follows
/// a literal from its opening quote to its closing one: a `\` before a newline drops the newline and
/// the next line's leading whitespace, as the compiler does, and an escaped `\n` becomes a newline.
/// Comments, character literals, lifetimes and raw strings are skipped.
fn literals_in(source: &str) -> Vec<(usize, String)> {
    let chars: Vec<char> = source.chars().collect();
    let mut found = Vec::new();
    let mut line = 1;
    let mut i = 0;
    let is_ident = |c: char| c.is_alphanumeric() || c == '_';
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            '\n' => {
                line += 1;
                i += 1;
            }
            '/' if next == Some('/') => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '/' if next == Some('*') => {
                i += 2;
                while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                    if chars[i] == '\n' {
                        line += 1;
                    }
                    i += 1;
                }
                i += 2;
            }
            // A raw string, `r"..."` or `r#"..."#`, has no escapes to process and is skipped.
            'r' if (i == 0 || !is_ident(chars[i - 1]) || chars[i - 1] == 'b')
                && matches!(next, Some('"' | '#')) =>
            {
                let mut j = i + 1;
                let mut hashes = 0;
                while chars.get(j) == Some(&'#') {
                    hashes += 1;
                    j += 1;
                }
                if chars.get(j) != Some(&'"') {
                    i += 1;
                    continue;
                }
                j += 1;
                loop {
                    match chars.get(j) {
                        None => break,
                        Some('\n') => line += 1,
                        Some('"') if (1..=hashes).all(|k| chars.get(j + k) == Some(&'#')) => {
                            j += 1 + hashes;
                            break;
                        }
                        Some(_) => {}
                    }
                    j += 1;
                }
                i = j;
            }
            // A character literal, including an escaped one; a lifetime has no closing quote.
            '\'' => {
                if next == Some('\\') {
                    i += 2;
                    while i < chars.len() && chars[i] != '\'' {
                        i += 1;
                    }
                    i += 1;
                } else if chars.get(i + 2) == Some(&'\'') {
                    i += 3;
                } else {
                    i += 1;
                }
            }
            '"' => {
                let opened = line;
                let mut text = String::new();
                let mut spans_lines = false;
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    match chars[i] {
                        '\\' => {
                            match chars.get(i + 1) {
                                Some('\n') => {
                                    line += 1;
                                    i += 2;
                                    while i < chars.len() && chars[i].is_whitespace() {
                                        if chars[i] == '\n' {
                                            line += 1;
                                        }
                                        i += 1;
                                    }
                                    continue;
                                }
                                Some('n') => text.push('\n'),
                                Some(other) => {
                                    text.push('\\');
                                    text.push(*other);
                                }
                                None => {}
                            }
                            i += 2;
                        }
                        '\n' => {
                            line += 1;
                            text.push('\n');
                            spans_lines = true;
                            i += 1;
                        }
                        other => {
                            text.push(other);
                            i += 1;
                        }
                    }
                }
                i += 1;
                // **A literal holding a real newline is exempt as a whole.** Prose served to a
                // caller continues its lines with `\`; a literal that breaks lines for real is SQL,
                // whose column alignment and indentation are deliberate and never served.
                if !spans_lines {
                    found.push((opened, text));
                }
            }
            _ => i += 1,
        }
    }
    found
}

/// Where in `text` a run of spaces sits that is not the indentation after an escaped `\n`, if
/// anywhere. **That indentation is exempt**, because a served message may lay out a list on lines of
/// its own; the defect is a run in the middle of a line of prose.
fn interior_run(text: &str) -> Option<&str> {
    text.split('\n')
        .map(str::trim_start)
        .find(|piece| piece.contains("  "))
}

#[test]
fn no_library_string_literal_carries_a_run_of_spaces() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    library_sources(&root, &mut files);
    assert!(!files.is_empty(), "found no source to check");

    let mut offending = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file).expect("reads a source file");
        for (line, literal) in literals_in(&source) {
            if let Some(piece) = interior_run(&literal) {
                let at = piece.find("  ").expect("found above");
                let start = piece[..at]
                    .char_indices()
                    .rev()
                    .nth(30)
                    .map_or(0, |(i, _)| i);
                offending.push(format!(
                    "{}:{line}: …{}",
                    file.strip_prefix(&root).unwrap_or(&file).display(),
                    &piece[start..at + 2]
                ));
            }
        }
    }
    assert!(
        offending.is_empty(),
        "string literals carrying a run of spaces -- a line break that lost its trailing `\\`, \
         served to callers as whitespace:\n{}",
        offending.join("\n")
    );
}

/// The checker itself, on the shapes it has to tell apart. A lint that finds nothing proves nothing
/// until it has been seen to find something.
#[test]
fn the_literal_reader_finds_what_it_should_and_nothing_else() {
    let found = |source: &str| -> Vec<Option<String>> {
        literals_in(source)
            .into_iter()
            .map(|(_, text)| interior_run(&text).map(str::to_owned))
            .collect()
    };
    // The known defect: a run inside one line.
    assert_eq!(found("f(\"a  b\")"), vec![Some("a  b".to_owned())]);
    // A continuation drops the next line's indentation, so this is clean.
    assert_eq!(found("f(\"a \\\n        b\")"), vec![None]);
    // The mutation that got past the first version: a run on a line that continues.
    assert_eq!(
        found("f(\"a          b \\\n        c\")"),
        vec![Some("a          b c".to_owned())]
    );
    // SQL, which breaks lines for real, is not a literal to check -- column alignment included.
    assert!(found("q(\"SELECT a,\n    key   TEXT\")").is_empty());
    // An escaped `\n` followed by indentation is layout, and exempt.
    assert_eq!(found("f(\"list:\\n  - one\")"), vec![None]);
    // Comments, character literals, lifetimes and raw strings are not literals to check.
    assert!(found("// \"a  b\"\n/* \"a  b\" */ let q = '\"'; fn f<'a>() {}").is_empty());
    assert!(found("r#\"a  b\"#").is_empty());
}
