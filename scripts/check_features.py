#!/usr/bin/env python3
"""Check every optional capability combination except the ordinary default build."""

from itertools import combinations
from pathlib import Path
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]
FEATURES = ("minify-html", "minify-css", "minify-js", "hooks")


def main():
    toolchain = tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"]
    cargo = ["cargo", f"+{toolchain}"]
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text())
    defaults = frozenset(manifest["features"]["default"])
    # Native jobs and the staged gate already exercise the standard distribution.
    for count in range(len(FEATURES) + 1):
        for enabled in combinations(FEATURES, count):
            if frozenset(enabled) == defaults:
                continue
            flags = ["--frozen", "--all-targets", "--no-default-features"]
            if enabled:
                flags.extend(["--features", ",".join(enabled)])
            print(f"Checking capabilities: {', '.join(enabled) or 'none'}", flush=True)
            for command in ([*cargo, "clippy", *flags, "--", "-D", "warnings"],
                            [*cargo, "test", *flags]):
                print("+ " + " ".join(command), flush=True)
                subprocess.run(command, cwd=ROOT, check=True)


if __name__ == "__main__":
    main()
