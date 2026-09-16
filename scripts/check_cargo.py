#!/usr/bin/env python3
"""Inspect packaged source, install/smoke it, and optionally prepare a publish candidate."""

import argparse
import hashlib
import json
from pathlib import Path, PurePosixPath
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
TOOLCHAIN = "1.98.1"
GENERATED = {"Cargo.toml.orig", ".cargo_vcs_info.json"}
FORBIDDEN = {
    ".cargo", ".git", ".github", ".regen-stage", ".regen-previous",
    "__pycache__", "dist", "review", "node_modules", "private", "release-artifacts",
    "target", "website-data", "website_data",
}


def public_path(name):
    path = PurePosixPath(name)
    if not name or path.is_absolute() or path.as_posix() != name or "\\" in name:
        raise RuntimeError(f"non-portable source inventory path: {name}")
    if any(part in {".", ".."} or part.casefold() in FORBIDDEN or part.casefold().startswith(".env") for part in path.parts):
        raise RuntimeError(f"private, generated, or cache path in source inventory: {name}")
    if any(character in name for character in "*?[]!"):
        raise RuntimeError(f"source inventory must contain literal files, not patterns: {name}")
    return path


def source_inventory(manifest):
    included = manifest["package"]["include"]
    if not included or len(included) != len(set(included)):
        raise RuntimeError("source inventory is empty or contains duplicates")
    sources = {}
    for pattern in included:
        if not pattern.startswith("/"):
            raise RuntimeError(f"source inventory entry must be root-anchored: {pattern}")
        name = pattern[1:]
        path = ROOT / public_path(name)
        if not path.is_file() or path.resolve(strict=True) != path:
            raise RuntimeError(f"source inventory entry is missing, not a file, or traverses a symlink: {name}")
        sources[name] = path.read_bytes()
    required = {"Cargo.toml", "Cargo.lock", "LICENSE", "README.md", "src/main.rs", "src/lib.rs", "examples/minimal/regen.toml"}
    if not required <= sources.keys():
        raise RuntimeError(f"source inventory omits required package inputs: {sorted(required - sources.keys())}")
    return sources


def check_names(names, sources):
    expected = set(sources) | {"Cargo.toml.orig"}
    missing = expected - names
    unexpected = names - expected - GENERATED
    if missing or unexpected:
        raise RuntimeError(f"Cargo source inventory differs: missing={sorted(missing)}, unexpected={sorted(unexpected)}")
    for name in names:
        public_path(name)


def unpack(crate, destination, identity, sources):
    with tarfile.open(crate, "r:gz") as archive:
        members = {}
        for member in archive.getmembers():
            path = PurePosixPath(member.name)
            if not member.isfile() or not member.name.startswith(f"{identity}/"):
                raise RuntimeError(f"unexpected Cargo archive entry: {member.name}")
            name = member.name[len(identity) + 1:]
            if path.as_posix() != member.name or name in members:
                raise RuntimeError(f"non-portable or duplicate Cargo archive entry: {member.name}")
            public_path(name)
            members[name] = member
        check_names(set(members), sources)
        for name, member in members.items():
            with archive.extractfile(member) as stream:
                data = stream.read()
            if name == "Cargo.toml.orig":
                if data != sources["Cargo.toml"]:
                    raise RuntimeError("packaged original manifest differs from reviewed source")
            elif name == "Cargo.lock":
                if tomllib.loads(data.decode("utf-8")) != tomllib.loads(sources[name].decode("utf-8")):
                    raise RuntimeError("packaging changed the locked dependency graph")
            elif name in sources and name != "Cargo.toml" and data != sources[name]:
                raise RuntimeError(f"packaged input differs from reviewed source: {name}")
            output = destination / name
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_bytes(data)
    return sorted(members)


def verify_source_bundle(directory, commit):
    """Verify the retained crate against both its receipt and this exact checkout."""
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    sources = source_inventory(manifest)
    package = manifest["package"]
    identity = f"{package['name']}-{package['version']}"
    crate = directory / f"{identity}.crate"
    receipt = json.loads((directory / "SOURCE.json").read_text(encoding="utf-8"))
    expected = {
        "format": 1, "package": package["name"], "version": package["version"],
        "commit": commit, "crate_sha256": hashlib.sha256(crate.read_bytes()).hexdigest(),
        "sources": {name: hashlib.sha256(data).hexdigest() for name, data in sources.items()},
    }
    if receipt != expected:
        raise RuntimeError("retained source receipt differs from the tested commit, crate, or source inventory")
    with tempfile.TemporaryDirectory(prefix="regen-source-") as temporary:
        extracted = Path(temporary)
        unpack(crate, extracted, identity, sources)
        vcs = json.loads((extracted / ".cargo_vcs_info.json").read_text(encoding="utf-8"))
        if vcs["git"]["sha1"] != commit or vcs["git"].get("dirty", False):
            raise RuntimeError("retained crate is not from the clean tested commit")
        normalized = tomllib.loads((extracted / "Cargo.toml").read_text(encoding="utf-8"))
        if any(normalized["package"][key] != package[key] for key in ("name", "version")):
            raise RuntimeError("retained crate changed the package identity")
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", help="Require this native Rust target, as in scripts/smoke.py")
    parser.add_argument("--allow-dirty", action="store_true", help="Explicitly allow packaging reviewed, uncommitted local changes")
    parser.add_argument("--publish-dry-run", action="store_true", help="Run Cargo publication checks without uploading or using a registry token")
    parser.add_argument("--output", type=Path, help="Retain the verified crate and SOURCE.json in a new directory; requires a clean checkout and --publish-dry-run")
    args = parser.parse_args()
    if args.output and (args.allow_dirty or not args.publish_dry_run):
        parser.error("--output requires a clean checkout and --publish-dry-run")
    if args.output and args.output.exists():
        parser.error("--output must name a new directory; retained candidates are never overwritten")
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    package = manifest["package"]
    if package["name"] != "regen-ssg":
        raise RuntimeError("unexpected Cargo package identity")
    sources = source_inventory(manifest)
    identity = f"{package['name']}-{package['version']}"
    with tempfile.TemporaryDirectory(prefix="regen-cargo-") as temporary:
        work = Path(temporary)
        cargo = ["cargo", f"+{TOOLCHAIN}"]
        command = [*cargo, "package", "--frozen", "--target-dir", str(work / "package-target")]
        if args.allow_dirty:
            command.append("--allow-dirty")
        listed = subprocess.check_output([*command, "--list"], cwd=ROOT, text=True).splitlines()
        if len(listed) != len(set(listed)):
            raise RuntimeError("Cargo listed duplicate package files")
        check_names(set(listed), sources)
        # Installation below is the packaged-source build; do not compile it twice.
        subprocess.run([*command, "--no-verify"], cwd=ROOT, check=True)
        crate = work / "package-target" / "package" / f"{identity}.crate"
        extracted = work / identity
        names = unpack(crate, extracted, identity, sources)
        if set(names) != set(listed):
            raise RuntimeError("generated Cargo archive differs from cargo package --list")
        normalized = tomllib.loads((extracted / "Cargo.toml").read_text(encoding="utf-8"))
        if any(normalized["package"][key] != package[key] for key in ("name", "version")):
            raise RuntimeError("packaged manifest changed the package identity")
        print("Verified Cargo source inventory:\n" + "\n".join(names), flush=True)
        print(f"Crate SHA256: {hashlib.sha256(crate.read_bytes()).hexdigest()}", flush=True)
        install = [*cargo, "install", "--path", str(extracted), "--bin", "regen", "--frozen", "--root", str(work / "install"), "--target-dir", str(work / "build")]
        if args.target:
            install.extend(["--target", args.target])
        subprocess.run(install, cwd=extracted, check=True)
        binary = work / "install" / "bin" / ("regen.exe" if sys.platform == "win32" else "regen")
        reported = subprocess.check_output([str(binary), "--version"], text=True).strip()
        if reported != f"regen {package['version']}":
            raise RuntimeError(f"installed executable version differs from package: {reported}")
        smoke = [sys.executable, str(extracted / "scripts" / "smoke.py"), "--binary", str(binary), "--site", str(extracted / "examples" / "minimal")]
        if args.target:
            smoke.extend(["--target", args.target])
        subprocess.run(smoke, cwd=extracted, check=True)
        print(f"Packaged-source installation and executable smoke passed: {identity}")
        if args.publish_dry_run:
            publish = [*cargo, "publish", "--registry", "crates-io", "--locked", "--dry-run", "--target-dir", str(work / "publish-target")]
            if args.allow_dirty:
                publish.append("--allow-dirty")
            subprocess.run(publish, cwd=ROOT, check=True)
            published = work / "publish-target" / "package" / "tmp-crate" / f"{identity}.crate"
            if published.read_bytes() != crate.read_bytes():
                raise RuntimeError("Cargo publish dry-run repackaged different bytes from the installed crate")
            print("Cargo publish --dry-run passed; nothing uploaded")
        if args.output:
            commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
            receipt = {
                "format": 1, "package": package["name"], "version": package["version"],
                "commit": commit, "crate_sha256": hashlib.sha256(crate.read_bytes()).hexdigest(),
                "sources": {name: hashlib.sha256(data).hexdigest() for name, data in sources.items()},
            }
            args.output.mkdir(parents=True)
            shutil.copyfile(crate, args.output / crate.name)
            (args.output / "SOURCE.json").write_text(json.dumps(receipt, indent=2, sort_keys=True) + "\n", encoding="utf-8", newline="\n")
            verify_source_bundle(args.output, commit)
            print(f"Retained clean source candidate for {commit}: {args.output}")


if __name__ == "__main__":
    main()
