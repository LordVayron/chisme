#!/usr/bin/env python3
"""Repository guard, run by CI on every push and pull request.

Fails when the public repository would gain something that must not be here:
  - files: personal paths, personal email addresses, website/deploy references
  - commits in a pull request: AI co-author trailers
  - pull request title: not in Conventional Commits format

Usage:
  guard.py files
  guard.py commits <base-sha> <head-sha>
  guard.py title "<pull request title>"
"""
import re
import subprocess
import sys

SELF = "scripts/guard.py"

# Allowed email domains (bots and GitHub's private addresses).
EMAIL_OK = re.compile(r"@(users\.noreply\.github\.com|github\.com)$", re.I)
EMAIL = re.compile(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}")

FILE_RULES = [
    ("personal path", re.compile(r"[A-Za-z]:[\\/]+Users[\\/]+(?!Public\b|<|%|\{|\$)[A-Za-z0-9._-]+", re.I)),
    ("personal path", re.compile(r"/(home|Users)/(?!runner\b|<|\$)[a-z][a-z0-9._-]+/", re.I)),
    ("OneDrive path", re.compile(r"OneDrive", re.I)),
    ("website or deploy reference", re.compile(r"vercel|netlify|chisme-ten|landing page", re.I)),
]

TRAILERS = re.compile(r"co-authored-by:.*(claude|anthropic|openai|copilot)|generated with \[?claude", re.I)

TYPES = "feat|fix|docs|refactor|perf|test|ci|chore|release|build|revert|style"
TITLE = re.compile(rf"^({TYPES})(\([a-z0-9._/-]+\))?!?: \S.{{0,62}}$")


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout


def check_files():
    problems = []
    for path in git("ls-files").splitlines():
        if path == SELF:
            continue
        try:
            text = open(path, encoding="utf-8").read()
        except (UnicodeDecodeError, OSError):
            continue  # binary
        for n, line in enumerate(text.splitlines(), 1):
            for label, rx in FILE_RULES:
                if rx.search(line):
                    problems.append(f"{path}:{n}: {label}")
            for email in EMAIL.findall(line):
                if not EMAIL_OK.search(email) and not email.endswith("@example.com"):
                    problems.append(f"{path}:{n}: email address")
    return problems


def check_commits(base, head):
    problems = []
    for sha in git("rev-list", f"{base}..{head}").split():
        msg = git("log", "-1", "--format=%B", sha)
        if TRAILERS.search(msg):
            problems.append(f"commit {sha[:7]}: AI co-author or attribution line")
        for addr in git("log", "-1", "--format=%ae%n%ce", sha).split():
            if not EMAIL_OK.search(addr):
                problems.append(f"commit {sha[:7]}: author or committer email is not a GitHub noreply address")
    return problems


def check_title(title):
    if TITLE.match(title):
        return []
    return [f'PR title "{title}" is not Conventional Commits: "type: summary" '
            f"(type is one of {TYPES.replace('|', ', ')}; under 64 characters)"]


def main():
    cmd = sys.argv[1] if len(sys.argv) > 1 else "files"
    if cmd == "files":
        problems = check_files()
    elif cmd == "commits":
        problems = check_commits(sys.argv[2], sys.argv[3])
    elif cmd == "title":
        problems = check_title(sys.argv[2])
    else:
        sys.exit(__doc__)
    for p in problems:
        print(f"::error::{p}")
    if problems:
        sys.exit(1)
    print(f"guard {cmd}: ok")


if __name__ == "__main__":
    main()
