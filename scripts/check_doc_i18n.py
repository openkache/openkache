#!/usr/bin/env python3
"""Enforce trilingual documentation sync.

For each registered document set (English base + Korean + Chinese), this
checks two things against the *staged* content:

1. Co-change — if any file in a set is staged, all three must be staged.
   Prevents editing one language and forgetting the others.
2. Structural parity — the three files must share the same number of
   ``##`` / ``###`` headings and fenced code blocks, and each must carry a
   language-switcher line. Prevents "added a section in English only".

It cannot verify that a translation is *correct*, only that the three files
stay in lockstep. Exit code 1 on any violation.

Add a new set by appending to DOC_SETS below.
"""
import re
import subprocess
import sys

# (english_base, korean, chinese)
DOC_SETS = [
    ("README.md", "README.ko.md", "README.zh.md"),
]

LANG_SWITCHER = re.compile(r"English.*한국어.*中文|한국어.*中文.*English|中文")


def staged_files():
    out = subprocess.run(
        ["git", "diff", "--cached", "--name-only", "--diff-filter=ACMR"],
        capture_output=True, text=True, check=True,
    ).stdout
    return set(line.strip() for line in out.splitlines() if line.strip())


def staged_blob(path):
    """Return the staged (index) content of a path, or None if absent."""
    result = subprocess.run(
        ["git", "show", f":{path}"], capture_output=True, text=True,
    )
    return result.stdout if result.returncode == 0 else None


def structure(text):
    lines = text.splitlines()
    return {
        "headings": sum(1 for ln in lines if re.match(r"^#{2,3} ", ln)),
        "code_fences": sum(1 for ln in lines if ln.startswith("```")),
        "has_switcher": any(LANG_SWITCHER.search(ln) for ln in lines),
    }


def check_set(base, ko, zh, staged):
    files = (base, ko, zh)
    problems = []

    # 1. Co-change: if any is staged, all must be staged.
    touched = [f for f in files if f in staged]
    if touched and len(touched) != 3:
        missing = [f for f in files if f not in staged]
        problems.append(
            f"co-change: {', '.join(touched)} staged but "
            f"{', '.join(missing)} not. Update all three languages together."
        )
        return problems  # parity check is meaningless if not co-staged

    if not touched:
        return problems  # this set is untouched in this commit

    # 2. Structural parity across the three staged versions.
    structs = {}
    for f in files:
        content = staged_blob(f)
        if content is None:
            problems.append(f"missing file in set: {f}")
            continue
        structs[f] = structure(content)
    if len(structs) != 3:
        return problems

    for key, label in [("headings", "## / ### heading count"),
                       ("code_fences", "code-fence (```) count")]:
        vals = {f: structs[f][key] for f in files}
        if len(set(vals.values())) != 1:
            detail = ", ".join(f"{f}={vals[f]}" for f in files)
            problems.append(f"{label} differs across languages: {detail}")

    for f in files:
        if not structs[f]["has_switcher"]:
            problems.append(f"language-switcher line missing in {f}")

    return problems


def main():
    staged = staged_files()
    all_problems = []
    for base, ko, zh in DOC_SETS:
        all_problems.extend(check_set(base, ko, zh, staged))

    if all_problems:
        sys.stderr.write("\n✗ Trilingual doc sync check failed:\n")
        for p in all_problems:
            sys.stderr.write(f"  - {p}\n")
        sys.stderr.write(
            "\nFix the files above so English/Korean/Chinese stay in sync, "
            "then re-stage and commit.\n"
        )
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
