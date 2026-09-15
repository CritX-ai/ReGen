#!/usr/bin/env python3
"""Exercise a native ReGen binary and compare complete generated output trees."""

import argparse
import hashlib
from html.parser import HTMLParser
import json
from pathlib import Path
import re
import shutil
import struct
import subprocess
import tempfile
import tomllib
import xml.etree.ElementTree as ET


def snapshot(root):
    entries = {}
    tree = hashlib.sha256()
    for path in sorted(root.rglob("*")):
        if path.is_symlink():
            raise RuntimeError(f"generated symlink: {path.relative_to(root)}")
        if path.is_dir():
            continue
        if not path.is_file():
            raise RuntimeError("generated special file")
        name = path.relative_to(root).as_posix()
        data = path.read_bytes()
        encoded = name.encode("utf-8")
        tree.update(struct.pack(">Q", len(encoded)))
        tree.update(encoded)
        tree.update(struct.pack(">Q", len(data)))
        tree.update(data)
        entries[name] = (len(data), hashlib.sha256(data).hexdigest())
    return entries, tree.hexdigest()


class Links(HTMLParser):
    def __init__(self, html):
        super().__init__()
        self.links = []
        self.feed(html)

    def handle_starttag(self, tag, attrs):
        if tag == "link":
            self.links.append(dict(attrs))


def inspect_site(site):
    config = tomllib.loads((site / "regen.toml").read_text(encoding="utf-8"))
    dist = site / "dist"
    origin = config["site"]["base_url"].rstrip("/")
    routes = {
        "index.html": ("/", {"en": "/", "de": "/de/"}),
        "de/index.html": ("/de/", {"en": "/", "de": "/de/"}),
    }
    pages = {path.relative_to(dist).as_posix() for path in dist.rglob("*.html")}
    if pages != set(routes):
        raise RuntimeError("minimal example must contain exactly two localized pages")
    for name, (route, alternates) in routes.items():
        html = (dist / name).read_text(encoding="utf-8")
        if "<html" not in html.lower():
            raise RuntimeError(f"missing rendered page: {name}")
        links = Links(html).links
        canonical = [link.get("href") for link in links if link.get("rel") == "canonical"]
        if canonical != [origin + route]:
            raise RuntimeError(f"incorrect canonical link: {name}")
        actual = {link.get("hreflang"): link.get("href") for link in links if link.get("rel") == "alternate"}
        if any(actual.get(code) != origin + path for code, path in alternates.items()):
            raise RuntimeError(f"incorrect hreflang links: {name}")
        stylesheets = [link.get("href", "") for link in links if link.get("rel") == "stylesheet"]
        if not any(re.fullmatch(r"/assets/[0-9a-f]{64}/site\.css", href) and (dist / href.lstrip("/")).is_file() for href in stylesheets):
            raise RuntimeError(f"missing content-hashed stylesheet: {name}")
    sitemap = ET.parse(dist / "sitemap.xml")
    locations = {node.text for node in sitemap.findall(".//{http://www.sitemaps.org/schemas/sitemap/0.9}loc")}
    if locations != {origin + route for route, _ in routes.values()}:
        raise RuntimeError("sitemap does not describe the generated pages")
    entries, digest = snapshot(dist)
    manifest = json.loads((dist / "regen-manifest.json").read_text(encoding="utf-8"))
    expected = {name: {"bytes": size, "sha256": checksum} for name, (size, checksum) in entries.items() if name != "regen-manifest.json"}
    version = tomllib.loads((Path(__file__).resolve().parents[1] / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    if manifest.get("format") != 1 or manifest.get("generator") != "ReGen" or manifest.get("version") != version or manifest.get("files") != expected:
        raise RuntimeError("manifest metadata or file hashes do not match output bytes")
    marks = list((dist / "assets").glob("*/mark.svg"))
    if len(marks) != 1 or marks[0].read_bytes() != (site / "assets" / "mark.svg").read_bytes():
        raise RuntimeError("SVG passthrough bytes changed")
    for public in sorted((site / "public").rglob("*")):
        if public.is_file():
            if public.read_bytes() != (dist / public.relative_to(site / "public")).read_bytes():
                raise RuntimeError("public file bytes changed")
    if (site / ".regen-stage").exists() or (site / ".regen-previous").exists():
        raise RuntimeError("successful build left a recovery directory")
    return entries, digest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--site", type=Path, default=Path("examples/minimal"))
    parser.add_argument("--target", help="Require the selected Rust toolchain to match the native target")
    args = parser.parse_args()
    binary = args.binary.resolve(strict=True)
    if args.target:
        details = subprocess.check_output(["rustc", "+1.98.1", "-vV"], text=True)
        if f"host: {args.target}" not in details.splitlines():
            raise RuntimeError("Rust host differs from the native matrix target")
    version = subprocess.check_output([str(binary), "--version"], text=True).strip()
    print(version)
    with tempfile.TemporaryDirectory(prefix="regen-smoke-") as temporary:
        root = Path(temporary)
        snapshots = []
        for name in ("first", "second"):
            site = root / name
            shutil.copytree(args.site, site, ignore=shutil.ignore_patterns("dist", ".regen-stage", ".regen-previous"))
            subprocess.run([str(binary), "build", "--site", str(site)], check=True)
            snapshots.append(inspect_site(site))
            if name == "first":
                subprocess.run([str(binary), "build", "--site", str(site)], check=True)
                snapshots.append(inspect_site(site))
        if any(result != snapshots[0] for result in snapshots[1:]):
            raise RuntimeError("generated paths or bytes differ across repeated builds or site roots")
        entries, digest = snapshots[0]
        print(f"Native example, output replacement, and deterministic comparison passed: {len(entries)} files")
        print(f"Output tree SHA256: {digest}")


if __name__ == "__main__":
    main()
