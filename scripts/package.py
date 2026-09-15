#!/usr/bin/env python3
"""Package a built binary with locked dependency inventories and original notices."""

import argparse
import gzip
import hashlib
import io
import json
from pathlib import Path
import re
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TOOLCHAIN = "1.98.1"
TARGETS = (
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
    "x86_64-pc-windows-msvc",
    "aarch64-pc-windows-msvc",
)
NOTICE = re.compile(r"^(licen[cs]e|copying|notice|copyright)(?:$|[-._])", re.IGNORECASE)
LICENSE = re.compile(r"^(licen[cs]e|copying)(?:$|[-._])", re.IGNORECASE)


def version():
    return tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]


def json_file(path, value):
    path.write_text(json.dumps(value, indent=2, sort_keys=True, ensure_ascii=False) + "\n", encoding="utf-8", newline="\n")


def notices(root, explicit=None, require_license=True):
    root = root.resolve(strict=True)
    selected = set()
    for path in root.rglob("*"):
        relative = path.relative_to(root)
        if any(part in {".git", "target"} for part in relative.parts):
            continue
        if NOTICE.match(path.name) or any(part.lower() in {"licenses", "licences"} for part in relative.parts[:-1]):
            if path.is_file():
                selected.add(path)
    if explicit:
        path = (root / explicit).resolve(strict=True)
        path.relative_to(root)
        selected.add(path)
    result = []
    has_license = False
    for path in sorted(selected):
        if path.is_symlink() or not path.is_file() or not path.read_bytes().strip():
            raise RuntimeError(f"unusable license file: {path.name}")
        path.resolve(strict=True).relative_to(root)
        relative = path.relative_to(root).as_posix()
        result.append((relative, path.read_bytes()))
        if (LICENSE.match(path.name) and path.suffix.lower() not in {".rs", ".py", ".js"}) or (explicit and path == (root / explicit).resolve()):
            has_license = True
    if require_license and not has_license:
        raise RuntimeError(f"no bundled license text for {root.name}; resolve upstream notice availability before packaging")
    return result


def copy_notices(destination, files):
    names = []
    for name, data in files:
        path = destination / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
        names.append(name)
    return names


def supplemental_notices(package, entry):
    """Verify reviewed notices against the exact published crate VCS revision."""
    crate = Path(package["manifest_path"]).parent
    vcs = json.loads((crate / ".cargo_vcs_info.json").read_text(encoding="utf-8"))
    if vcs["git"]["sha1"] != entry["vcs_commit"] or not entry["files"]:
        raise RuntimeError(f"supplemental license revision mismatch: {package['name']}")
    if entry["license_basis"] == "upstream-declaration-with-standard-terms":
        kinds = {notice.get("kind") for notice in entry["files"]}
        if package.get("license") != "MIT" or not {"license-declaration", "standard-license-terms"} <= kinds:
            raise RuntimeError("declared-MIT notice supplement lacks its declaration or standard terms")
    result = []
    for notice in entry["files"]:
        relative = Path(notice["path"])
        relative.relative_to("licenses/supplemental")
        path = ROOT / relative
        path.resolve(strict=True).relative_to(ROOT.resolve())
        if path.is_symlink() or not path.is_file():
            raise RuntimeError("supplemental notice must be a regular repository file")
        data = path.read_bytes()
        if hashlib.sha256(data).hexdigest() != notice["sha256"] or not data.strip():
            raise RuntimeError(f"supplemental notice digest mismatch: {relative}")
        result.append((f"upstream/{relative.relative_to('licenses/supplemental').as_posix()}", data))
    return result


def collect_licenses(destination, target):
    # Cargo emits UTF-8 regardless of the host's legacy Windows code page.
    metadata = json.loads(subprocess.check_output([
        "cargo", f"+{TOOLCHAIN}", "metadata", "--frozen", "--format-version", "1", "--filter-platform", target,
    ], cwd=ROOT, encoding="utf-8"))
    packages = []
    workspace = set(metadata["workspace_members"])
    identities = set()
    supplement = json.loads((ROOT / "licenses" / "supplemental.json").read_text(encoding="utf-8"))
    if supplement.get("format") != 1:
        raise RuntimeError("unknown supplemental notice manifest format")
    supplemental = {(entry["name"], entry["version"]): entry for entry in supplement["packages"]}
    if len(supplemental) != len(supplement["packages"]):
        raise RuntimeError("duplicate supplemental dependency identity")
    for package in sorted(metadata["packages"], key=lambda item: (item["name"], item["version"])):
        if package["id"] in workspace:
            continue
        if package["source"] not in {"registry+https://github.com/rust-lang/crates.io-index", "sparse+https://index.crates.io/"}:
            raise RuntimeError(f"non-public-registry dependency needs explicit release review: {package['name']}")
        identity = f"{package['name']}-{package['version']}"
        if identity in identities:
            raise RuntimeError(f"ambiguous dependency identity: {identity}")
        identities.add(identity)
        extra = supplemental.get((package["name"], package["version"]))
        original = notices(Path(package["manifest_path"]).parent, package.get("license_file"), require_license=extra is None)
        if extra is not None:
            original.extend(supplemental_notices(package, extra))
        paths = copy_notices(destination / "crates" / identity, original)
        packages.append({
            "name": package["name"],
            "version": package["version"],
            "license": package.get("license"),
            "source": f"https://crates.io/crates/{package['name']}/{package['version']}",
            "notices": [f"crates/{identity}/{path}" for path in paths],
            "supplemental_sources": extra["files"] if extra else [],
            "license_basis": extra["license_basis"] if extra else "bundled-upstream-text",
            "notice_source_gap": extra.get("unavailable_source") if extra else None,
        })
    if not packages:
        raise RuntimeError("locked dependency inventory is empty")
    sysroot = Path(subprocess.check_output(["rustc", f"+{TOOLCHAIN}", "--print", "sysroot"], encoding="utf-8").strip())
    rust_docs = sysroot / "share" / "doc" / "rust"
    runtime_copyright = rust_docs / "COPYRIGHT-library.html"
    runtime_files = [runtime_copyright, *sorted((rust_docs / "licenses").glob("*.txt"))]
    if len(runtime_files) < 2 or any(path.is_symlink() or not path.is_file() or not path.read_bytes().strip() for path in runtime_files):
        raise RuntimeError("selected Rust toolchain lacks original library copyright/license documents")
    rust_notices = copy_notices(destination / "rust", [
        (path.relative_to(rust_docs).as_posix(), path.read_bytes()) for path in runtime_files
    ])
    inventory = {
        "format": 1,
        "package": "regen-ssg",
        "version": version(),
        "target": target,
        "scope": "Conservative locked Cargo metadata inventory; may include build, development, and inactive packages, not a binary reachability analysis.",
        "packages": packages,
        "rust": {
            "version": TOOLCHAIN,
            "source": f"https://github.com/rust-lang/rust/tree/{TOOLCHAIN}",
            "notices": [f"rust/{path}" for path in rust_notices],
        },
    }
    json_file(destination / "inventory.json", inventory)
    lines = [
        "THIRD-PARTY NOTICES", "",
        "ReGen's WTFPL does not relicense dependencies or the Rust runtime.",
        "Original license, copyright and notice files are included under licenses/.",
        "inventory.json identifies the exact locked package versions and source URLs.",
        "The inventory is conservative, not a list of code proven linked into the executable.",
        "Some upstream distributions declare MIT but omit a full license/copyright file.",
        "Those exact declarations and separate standard MIT terms are retained and explicitly flagged.",
        "This inventory does not establish license clearance; review flagged source gaps before publication.",
        "System libraries supplied by the operating system are not bundled.", "",
    ]
    for package in packages:
        lines.extend([f"{package['name']} {package['version']}: {package['license'] or 'see license files'}", package["source"]])
        lines.extend(f"  licenses/{path}" for path in package["notices"])
        if package["notice_source_gap"]:
            lines.extend(["  UPSTREAM NOTICE SOURCE GAP — maintainer review required", "  " + package["notice_source_gap"]["reason"]])
        lines.append("")
    lines.extend([f"Rust {TOOLCHAIN}", inventory["rust"]["source"]])
    lines.extend(f"  licenses/{path}" for path in inventory["rust"]["notices"])
    return "\n".join(lines) + "\n"


def archive_tree(root, output):
    files = sorted(path for path in root.rglob("*") if path.is_file())
    if output.suffix == ".zip":
        with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
            for path in files:
                info = zipfile.ZipInfo(f"{root.name}/{path.relative_to(root).as_posix()}", date_time=(1980, 1, 1, 0, 0, 0))
                info.create_system = 3
                mode = 0o755 if path.name in {"regen", "regen.exe"} else 0o644
                info.external_attr = (0o100000 | mode) << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(info, path.read_bytes())
    else:
        with output.open("wb") as raw:
            with gzip.GzipFile(filename="", mode="wb", fileobj=raw, mtime=0) as compressed:
                with tarfile.open(fileobj=compressed, mode="w", format=tarfile.PAX_FORMAT) as archive:
                    for path in files:
                        data = path.read_bytes()
                        info = tarfile.TarInfo(f"{root.name}/{path.relative_to(root).as_posix()}")
                        info.size = len(data)
                        info.mode = 0o755 if path.name == "regen" else 0o644
                        info.mtime = 0
                        archive.addfile(info, io.BytesIO(data))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", type=Path, default=ROOT / "release-artifacts")
    parser.add_argument("--tag", help="If supplied, must equal v followed by the Cargo package version")
    args = parser.parse_args()
    current = version()
    if args.tag and args.tag != f"v{current}":
        raise RuntimeError("release tag and Cargo package version differ")
    binary = args.binary.resolve(strict=True)
    reported = subprocess.check_output([str(binary), "--version"], text=True).strip()
    if reported != f"regen {current}":
        raise RuntimeError(f"binary version does not match Cargo package: {reported}")
    args.output.mkdir(parents=True, exist_ok=True)
    name = f"regen-{current}-{args.target}"
    extension = ".zip" if "windows" in args.target else ".tar.gz"
    archive_path = args.output / f"{name}{extension}"
    notice_archive = args.output / f"{name}-licenses.tar.gz"
    inventory_path = args.output / f"{name}-dependencies.json"
    if any(path.exists() for path in (archive_path, notice_archive, inventory_path)):
        raise RuntimeError("refusing to overwrite existing release artifacts; use an empty output directory")
    with tempfile.TemporaryDirectory(prefix="regen-package-") as temporary:
        package = Path(temporary) / name
        package.mkdir()
        executable = "regen.exe" if "windows" in args.target else "regen"
        shutil.copyfile(binary, package / executable)
        (package / executable).chmod(0o755)
        for document in ("LICENSE", "README.md", "CHANGELOG.md", "SECURITY.md", "CONTRIBUTING.md", "Cargo.lock"):
            shutil.copyfile(ROOT / document, package / document)
        shutil.copytree(ROOT / "docs", package / "docs")
        shutil.copytree(ROOT / "examples" / "minimal", package / "examples" / "minimal", ignore=shutil.ignore_patterns("dist", ".regen-stage", ".regen-previous"))
        assets = package / "site" / "assets"
        assets.mkdir(parents=True)
        for asset in ("regen-logo.svg", "regen-logo-static.svg", "regen-mark.svg"):
            shutil.copyfile(ROOT / "site" / "assets" / asset, assets / asset)
        notice_text = collect_licenses(package / "licenses", args.target)
        (package / "THIRD-PARTY-NOTICES.txt").write_text(notice_text, encoding="utf-8", newline="\n")
        archive_tree(package, archive_path)
        notice_root = Path(temporary) / f"{name}-licenses"
        notice_root.mkdir()
        shutil.copytree(package / "licenses", notice_root / "licenses")
        shutil.copyfile(package / "THIRD-PARTY-NOTICES.txt", notice_root / "THIRD-PARTY-NOTICES.txt")
        archive_tree(notice_root, notice_archive)
        shutil.copyfile(package / "licenses" / "inventory.json", inventory_path)
    for path in (archive_path, notice_archive, inventory_path):
        print(f"{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}")


if __name__ == "__main__":
    main()
