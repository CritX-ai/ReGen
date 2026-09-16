#!/usr/bin/env python3
"""Install/run ReGen's local Linux CI gate: python3 scripts/pre_commit.py --install.

The installed hook executes the staged checker and verifies an exact index snapshot,
not unstaged fixes. It never commits, pushes, uploads or claims hosted CI success.
Requires the pinned Rust toolchain with rustfmt, Clippy and llvm-tools-preview,
cargo-llvm-cov 0.9.1, strace, and rootless Podman (preferred) or Docker.
Cargo dependencies must already be fetched; the container runtime must be usable.
"""

import argparse
import os
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import sys
import tempfile
import tomllib

HOOK = '''#!/bin/sh
# ReGen staged-source CI gate; installed by scripts/pre_commit.py.
set -eu
PATH="$(git rev-parse --path-format=absolute --git-path regen-hook-tools/bin):$PATH"
export PATH
exec python3 -c 'import subprocess, sys; tree = subprocess.check_output(["git", "write-tree"], text=True).strip(); source = subprocess.check_output(["git", "show", tree + ":scripts/pre_commit.py"]); sys.argv = ["scripts/pre_commit.py", "--tree", tree]; exec(compile(source, "scripts/pre_commit.py", "exec"), {"__name__": "__main__", "__file__": "scripts/pre_commit.py"})'
'''


def git(root, *args):
    return subprocess.check_output(["git", *args], cwd=root)


def export_tree(root, tree, destination):
    """Read blobs directly: checkout filters and export-ignore cannot change the test input."""
    records = git(root, "ls-tree", "-r", "-z", "--full-tree", tree).split(b"\0")
    for record in filter(None, records):
        metadata, raw_name = record.split(b"\t", 1)
        mode, kind, blob = metadata.decode("ascii").split()
        name = raw_name.decode("utf-8")
        path = PurePosixPath(name)
        if mode not in {"100644", "100755"} or kind != "blob":
            raise RuntimeError(f"staged symlink or submodule is not a self-contained input: {name}")
        if path.is_absolute() or path.as_posix() != name or any(part in {"..", ".git"} for part in path.parts) or "\\" in name:
            raise RuntimeError(f"unsafe staged path: {name}")
        output = destination / name
        output.parent.mkdir(parents=True, exist_ok=True)
        output.write_bytes(git(root, "cat-file", "blob", blob))
        output.chmod(int(mode, 8) & 0o777)


def require_same_index(root, tree):
    if git(root, "write-tree").decode().strip() != tree:
        raise RuntimeError("staged files changed during verification; stage the intended changes and retry")


def prerequisites():
    if sys.platform != "linux":
        raise RuntimeError("this local gate requires Linux; the six-platform matrix still runs on GitHub")
    for command in ("cargo", "rustc", "cargo-llvm-cov", "strace"):
        if not shutil.which(command):
            raise RuntimeError(f"missing {command}; install the CI prerequisites before committing")
    version = subprocess.check_output(["cargo-llvm-cov", "llvm-cov", "--version"], text=True).strip()
    if version != "cargo-llvm-cov 0.9.1":
        raise RuntimeError("pre-commit requires cargo-llvm-cov 0.9.1, matching build.yml")


def verify(source, work, cache):
    # Isolate Cargo and nested Git fixtures from the committing repository/index.
    # Publication checks have no need for the caller's registry or GitHub tokens.
    environment = {key: value for key, value in os.environ.items()
                   if not key.startswith("GIT_") and key not in {"GH_TOKEN", "GITHUB_TOKEN", "GHCR_TOKEN", "DOCKER_AUTH_CONFIG"}
                   and not (key.startswith("CARGO_") and key.endswith("_TOKEN"))}
    environment["CARGO_TARGET_DIR"] = str(cache)
    toolchain = tomllib.loads((source / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    cargo = ["cargo", f"+{toolchain}"]

    def run(*command):
        print("+ " + " ".join(map(str, command)), flush=True)
        subprocess.run(list(map(str, command)), cwd=source, env=environment, check=True)

    details = subprocess.check_output(["rustc", f"+{toolchain}", "-vV"], env=environment, text=True)
    target = next(line.removeprefix("host: ") for line in details.splitlines() if line.startswith("host: "))
    run(*cargo, "fmt", "--all", "--", "--check")
    run(*cargo, "clippy", "--frozen", "--all-targets", "--", "-D", "warnings")
    run(*cargo, "test", "--frozen", "--all-targets")
    run(sys.executable, "scripts/check_features.py")
    run(sys.executable, "-m", "unittest", "discover", "-s", "scripts", "-p", "test_*.py")
    report = work / "rust.json"
    run(*cargo, "llvm-cov", "clean", "--workspace", "--frozen")
    run(*cargo, "llvm-cov", "--release", "--frozen", "--all-targets", "--all-features", "--no-report", "--", "--include-ignored")
    run(*cargo, "llvm-cov", "--release", "--frozen", "--all-targets", "--no-default-features",
        "--no-report", "--", "--include-ignored")
    run(*cargo, "llvm-cov", "report", "--release", "--frozen", "--json", "--output-path", report,
        "--ignore-filename-regex", "/tests/|/examples/")
    run(*cargo, "llvm-cov", "report", "--release", "--fail-uncovered-lines", "0", "--ignore-filename-regex", "/tests/|/examples/")
    run(sys.executable, "scripts/check_rust_coverage.py", "--report", report,
        "--output", work / "rust-source-regions.json", "--require-full")
    # Build and smoke the default-feature artifacts only after every coverage gate.
    run(*cargo, "build", "--release", "--frozen", "--bin", "regen", "--example", "build_docs")
    binary = cache / "release" / "regen"
    run(sys.executable, "scripts/smoke.py", "--binary", binary, "--target", target)
    docs = [work / "docs-primary", work / "docs-comparison"]
    for output in docs:
        run(cache / "release/examples/build_docs", "--base-url", "https://regen.critx.ai/", "--output", output)
    run(sys.executable, "scripts/check_docs.py", "--site", docs[0], "--base-url", "https://regen.critx.ai/", "--compare", docs[1])
    run(sys.executable, "scripts/package.py", "--target", target, "--binary", binary, "--output", work / "native")
    run(sys.executable, "scripts/container.py", "check", "--target", target, "--cache", cache / "container")
    run(sys.executable, "scripts/check_cargo.py", "--target", target, "--allow-dirty", "--publish-dry-run")


def install(root):
    configured = subprocess.run(["git", "config", "--get", "core.hooksPath"], cwd=root, stdout=subprocess.PIPE, check=False)
    if configured.returncode != 1:
        raise RuntimeError("core.hooksPath is configured or unreadable; refusing to replace a shared hook setup")
    path = Path(git(root, "rev-parse", "--path-format=absolute", "--git-path", "hooks/pre-commit").decode().strip())
    if path.is_symlink() or (path.exists() and path.read_text() != HOOK):
        raise RuntimeError(f"existing pre-commit hook preserved: {path}")
    path.parent.mkdir(parents=True, exist_ok=True)
    if not path.exists():
        with path.open("x", encoding="utf-8", newline="\n") as output:
            output.write(HOOK)
    path.chmod(0o755)
    print(f"Installed staged-source CI gate: {path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--install", action="store_true")
    parser.add_argument("--tree", help="Frozen staged tree supplied by the installed hook")
    args = parser.parse_args()
    root = Path(git(Path.cwd(), "rev-parse", "--show-toplevel").decode().strip())
    prerequisites()
    if args.install:
        install(root)
        return
    tree = args.tree or git(root, "write-tree").decode().strip()
    require_same_index(root, tree)
    cache = Path(git(root, "rev-parse", "--path-format=absolute", "--git-path", "regen-pre-commit-target").decode().strip())
    with tempfile.TemporaryDirectory(prefix="regen-pre-commit-") as temporary:
        work = Path(temporary)
        source = work / "source"
        source.mkdir()
        export_tree(root, tree, source)
        verify(source, work, cache)
    require_same_index(root, tree)
    print(f"Local native CI checks passed for staged tree {tree}. Hosted CI runs only after push.")


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, subprocess.CalledProcessError) as error:
        print(f"Pre-commit blocked: {error}", file=sys.stderr)
        sys.exit(1)
