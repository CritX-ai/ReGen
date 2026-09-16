#!/usr/bin/env python3
"""Offline checks of real source archives; no Cargo or native-platform simulation."""

import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch
import urllib.request

import check_cargo


def write_tar(path, entries):
    with tarfile.open(path, "w:gz") as archive:
        for name, data in entries:
            member = name if isinstance(name, tarfile.TarInfo) else tarfile.TarInfo(name)
            member.size = len(data)
            archive.addfile(member, io.BytesIO(data))


class SourceBundle:
    """Small reviewed checkout and retained crate, not a compilable/native fixture."""

    commit = "a" * 40
    identity = "regen-ssg-1.0.0"

    def __init__(self, directory):
        directory = directory.resolve()
        self.root = directory / "checkout"
        self.output = directory / "candidate"
        self.output.mkdir()
        self.sources = {
            "Cargo.lock": b'version = 4\n[[package]]\nname = "regen-ssg"\nversion = "1.0.0"\n',
            "LICENSE": b"Fixture license\n", "README.md": b"Fixture readme\n",
            "src/main.rs": b"fn main() {}\n", "src/lib.rs": b"pub fn fixture() {}\n",
            "examples/minimal/regen.toml": b'title = "Fixture"\n',
            "examples/minimal/content/Über uns.md": b"# Fixture page\n",
        }
        included = ["/Cargo.toml", *[f"/{name}" for name in self.sources]]
        self.sources["Cargo.toml"] = (
            '[package]\nname = "regen-ssg"\nversion = "1.0.0"\ninclude = ' + json.dumps(included) + "\n"
        ).encode()
        for name, data in self.sources.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(data)
        self.entries = dict(self.sources)
        self.entries["Cargo.toml.orig"] = self.sources["Cargo.toml"]
        self.entries["Cargo.toml"] = b'[package]\nname = "regen-ssg"\nversion = "1.0.0"\n'
        # Cargo may normalize formatting without changing the dependency graph.
        self.entries["Cargo.lock"] = b"# normalized by Cargo\n" + self.sources["Cargo.lock"]
        self.entries[".cargo_vcs_info.json"] = json.dumps({"git": {"sha1": self.commit}, "path_in_vcs": ""}).encode()
        self.crate = self.output / f"{self.identity}.crate"
        self.retain()

    def retain(self):
        write_tar(self.crate, [(f"{self.identity}/{name}", data) for name, data in self.entries.items()])
        self.receipt = {
            "format": 1, "package": "regen-ssg", "version": "1.0.0", "commit": self.commit,
            "crate_sha256": hashlib.sha256(self.crate.read_bytes()).hexdigest(),
            "sources": {name: hashlib.sha256(data).hexdigest() for name, data in self.sources.items()},
        }
        (self.output / "SOURCE.json").write_text(json.dumps(self.receipt), encoding="utf-8")


class SourceSafety(unittest.TestCase):
    def setUp(self):
        self.directory = Path(self.enterContext(tempfile.TemporaryDirectory(prefix="regen-source-test-")))
        self.bundle = SourceBundle(self.directory)
        self.enterContext(patch.object(check_cargo, "ROOT", self.bundle.root))
        self.enterContext(patch.object(subprocess, "run", side_effect=AssertionError("unexpected process")))
        self.enterContext(patch.object(subprocess, "check_output", side_effect=AssertionError("unexpected process")))
        self.enterContext(patch.object(urllib.request, "urlopen", side_effect=AssertionError("unexpected network")))

    def test_reviewed_bytes_survive_extraction_and_receipt_verification(self):
        bundle = self.bundle
        retained = {path.name: path.read_bytes() for path in bundle.output.iterdir()}
        extracted = self.directory / "extracted"
        names = check_cargo.unpack(bundle.crate, extracted, bundle.identity, bundle.sources)
        self.assertEqual(set(names), set(bundle.entries))
        self.assertEqual({name: (extracted / name).read_bytes() for name in names}, bundle.entries)
        self.assertEqual(check_cargo.verify_source_bundle(bundle.output, bundle.commit), bundle.receipt)
        self.assertEqual({path.name: path.read_bytes() for path in bundle.output.iterdir()}, retained)

    def test_receipt_does_not_authorize_changed_checkout_or_crate_bytes(self):
        bundle = self.bundle
        with self.assertRaises(RuntimeError):
            check_cargo.verify_source_bundle(bundle.output, "b" * 40)
        source = bundle.root / "src/lib.rs"
        source.write_bytes(b"pub fn changed() {}\n")
        with self.assertRaises(RuntimeError):
            check_cargo.verify_source_bundle(bundle.output, bundle.commit)
        source.write_bytes(bundle.sources["src/lib.rs"])
        bundle.crate.write_bytes(bundle.crate.read_bytes() + b"changed")
        with self.assertRaises(RuntimeError):
            check_cargo.verify_source_bundle(bundle.output, bundle.commit)

    def test_matching_receipt_cannot_hide_changed_archive_content_or_provenance(self):
        bundle = self.bundle
        changes = [
            ("src/lib.rs", b"pub fn unreviewed() {}\n"),
            ("Cargo.toml.orig", bundle.sources["Cargo.toml"] + b"# unreviewed\n"),
            ("Cargo.lock", bundle.sources["Cargo.lock"].replace(b'1.0.0', b'2.0.0')),
            ("Cargo.toml", b'[package]\nname = "other-package"\nversion = "1.0.0"\n'),
            (".cargo_vcs_info.json", json.dumps({"git": {"sha1": bundle.commit, "dirty": True}}).encode()),
            (".cargo_vcs_info.json", json.dumps({"git": {"sha1": "b" * 40, "dirty": False}}).encode()),
        ]
        for name, data in changes:
            with self.subTest(member=name, data=data):
                original = bundle.entries[name]
                bundle.entries[name] = data
                bundle.retain()
                with self.assertRaises(RuntimeError):
                    check_cargo.verify_source_bundle(bundle.output, bundle.commit)
                bundle.entries[name] = original

    def test_unsafe_or_inexact_archive_inventory_is_rejected_before_extraction(self):
        bundle = self.bundle
        regular = [(f"{bundle.identity}/{name}", data) for name, data in bundle.entries.items()]
        link = tarfile.TarInfo(f"{bundle.identity}/linked")
        link.type = tarfile.SYMTYPE
        link.linkname = "../../outside"
        variants = {
            "traversal": regular + [(f"{bundle.identity}/../outside", b"unsafe")],
            "symlink": regular + [(link, b"")],
            "duplicate": regular + [regular[0]],
            "private": regular + [(f"{bundle.identity}/.env.secret", b"secret")],
            "missing": regular[1:],
        }
        for label, entries in variants.items():
            with self.subTest(inventory=label):
                write_tar(bundle.crate, entries)
                destination = self.directory / "extracted"
                with self.assertRaises(RuntimeError):
                    check_cargo.unpack(bundle.crate, destination, bundle.identity, bundle.sources)
                self.assertFalse(destination.exists())
                self.assertFalse((self.directory / "outside").exists())

    def test_source_inventory_rejects_unreviewable_inputs(self):
        bundle = self.bundle
        included = [f"/{name}" for name in bundle.sources]
        variants = [included + ["/src/*.rs"], included + [included[0]],
                    included[1:], [name.lstrip("/") for name in included]]
        for names in variants:
            with self.subTest(inventory=names), self.assertRaises(RuntimeError):
                check_cargo.source_inventory({"package": {"include": names}})

    def test_source_inventory_rejects_symlinked_files(self):
        bundle = self.bundle
        try:
            (bundle.root / "linked.rs").symlink_to(bundle.root / "src/lib.rs")
        except OSError as error:
            if getattr(error, "winerror", None) == 1314:
                self.skipTest("Windows account cannot create symlinks")
            raise
        included = [f"/{name}" for name in bundle.sources] + ["/linked.rs"]
        with self.assertRaises(RuntimeError):
            check_cargo.source_inventory({"package": {"include": included}})


if __name__ == "__main__":
    unittest.main()
