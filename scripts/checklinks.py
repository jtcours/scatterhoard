#!/usr/bin/env python3
"""Check the cross-references between this project's Markdown files.

The project's tracking surfaces -- TRACKING.md, EARLY-VALIDATION.md and the
design documents -- are held together almost entirely by links between them, and a stale link is
worse than a missing one: it still resolves, so it reads as a working reference while pointing at
something that has moved. Two failure modes have actually happened here rather than being imagined:

  * A heading was renamed and every anchor into it kept rendering as a link that goes nowhere.
  * Line-number references (`store.rs:392`) went stale within the same session that wrote them,
    broken by an edit to the file *above* the cited line. They fail silently and stay plausible.

The second is why STANDARDS.md now says cross-references must not use line numbers, and why this
script warns about them. Reference by heading anchor, by a quoted bold lead-in, or by symbol name.

Usage:
    python3 scripts/checklinks.py              # every git-tracked .md file
    python3 scripts/checklinks.py TRACKING.md  # just these
    python3 scripts/checklinks.py --strict     # line-number references become errors too
    python3 scripts/checklinks.py --quiet      # only the summary line

Exit status is non-zero if any *error* was found. Line-number references are warnings by default,
because TRACKING.md carries several dozen of them from before the standard existed and a checker
that
fails on its first run is a checker nobody runs again. Once those are migrated, wire in --strict.
"""

import argparse
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

# [text](target) but not ![image](target); target must not be a bare fragment we treat separately.
LINK = re.compile(r"(?<!\!)\[[^\]]*\]\(\s*([^)\s]+)")
FENCE = re.compile(r"^\s*(```|~~~)")
LINE_SUFFIX = re.compile(r":\d+$")
EXTERNAL = ("http://", "https://", "mailto:")


def slug(heading: str) -> str:
    """Approximate GitHub's heading-to-anchor rule: strip markup, drop punctuation, hyphenate.

    Each remaining space becomes one hyphen -- runs are *not* collapsed. This matters more than it
    looks: a heading like `name -- Title` loses the dash as punctuation and keeps both spaces, so
    the real anchor carries a double hyphen. Collapsing them here would reject correct links, and
    every entry heading in TRACKING.md has this shape.
    """
    text = heading.lstrip("#").strip().lower()
    text = re.sub(r"`([^`]*)`", r"\1", text)          # inline code keeps its contents
    text = re.sub(r"\*+|_+", "", text)                # bold / italic markers
    text = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", text)  # links keep their text
    text = re.sub(r"[^\w\s-]", "", text)              # remaining punctuation
    return re.sub(r"\s", "-", text.strip())


def headings(path: pathlib.Path) -> set:
    """Every anchor `path` defines, including GitHub's -1, -2 suffixes for repeated headings."""
    found, seen = set(), {}
    in_fence = False
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        if in_fence or not line.startswith("#"):
            continue
        base = slug(line)
        if not base:
            continue
        count = seen.get(base, 0)
        seen[base] = count + 1
        found.add(base if count == 0 else f"{base}-{count}")
    return found


def links(path: pathlib.Path):
    """Yield (lineno, target) for each link outside a fenced code block."""
    in_fence = False
    for number, line in enumerate(
        path.read_text(encoding="utf-8", errors="replace").splitlines(), start=1
    ):
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        for target in LINK.findall(line):
            yield number, target


def tracked_markdown():
    """Every .md file git would consider part of the project, tracked or not yet added.

    `--others --exclude-standard` is what includes a brand-new file that has not been `git add`ed
    while still honouring .gitignore, so `checkpoints/` stays out. Without it a newly created
    document is invisible to this script, which is the moment its links are least likely to be
    right -- TRACKING.md was skipped exactly that way on the run that introduced it.
    """
    result = subprocess.run(
        ["git", "-C", str(ROOT), "ls-files", "--cached", "--others", "--exclude-standard", "*.md"],
        capture_output=True, text=True, check=True,
    )
    return sorted({ROOT / name for name in result.stdout.split()})


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("paths", nargs="*", help="files to check (default: git-tracked *.md)")
    parser.add_argument("--strict", action="store_true",
                        help="treat line-number references as errors")
    parser.add_argument("--quiet", action="store_true", help="print only the summary")
    args = parser.parse_args()

    files = [pathlib.Path(p).resolve() for p in args.paths] if args.paths else tracked_markdown()

    anchor_cache: dict = {}
    errors, warnings = 0, 0

    for path in files:
        shown = path.relative_to(ROOT) if path.is_relative_to(ROOT) else path
        if not path.exists():
            print(f"error  {shown}: file does not exist")
            errors += 1
            continue

        for lineno, raw in links(path):
            if raw.startswith(EXTERNAL):
                continue
            target, _, anchor = raw.partition("#")

            # A bare `#anchor` points into this same file.
            if not target:
                if path not in anchor_cache:
                    anchor_cache[path] = headings(path)
                if anchor and anchor not in anchor_cache[path]:
                    if not args.quiet:
                        print(f"error  {shown}:{lineno}: no such heading in this file -- `{raw}`")
                    errors += 1
                continue

            if LINE_SUFFIX.search(target):
                label = "error " if args.strict else "warn  "
                if not args.quiet:
                    print(f"{label} {shown}:{lineno}: line-number reference `{raw}` "
                          f"-- use a heading anchor or symbol name (STANDARDS.md)")
                errors += args.strict
                warnings += not args.strict
                target = LINE_SUFFIX.sub("", target)

            resolved = (path.parent / target).resolve()
            if not resolved.exists():
                if not args.quiet:
                    print(f"error  {shown}:{lineno}: target does not exist -- `{raw}`")
                errors += 1
                continue

            if not anchor or resolved.suffix != ".md":
                continue
            if resolved not in anchor_cache:
                anchor_cache[resolved] = headings(resolved)
            if anchor not in anchor_cache[resolved]:
                if not args.quiet:
                    print(f"error  {shown}:{lineno}: no such heading -- `{raw}`")
                errors += 1

    summary = f"{len(files)} file(s): {errors} error(s)"
    if warnings:
        summary += f", {warnings} line-number warning(s) -- re-run with --strict to fail on them"
    print(summary)
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
