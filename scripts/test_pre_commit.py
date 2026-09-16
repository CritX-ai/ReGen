#!/usr/bin/env python3
"""Defend staged-input isolation, user-hook preservation and the shared coverage gate."""

import copy
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

import check_rust_coverage
import pre_commit


class StagedInputs(unittest.TestCase):
    def setUp(self):
        self.root = Path(self.enterContext(tempfile.TemporaryDirectory(prefix="regen-hook-test-")))
        environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        environment.update({"GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull})
        self.enterContext(patch.dict(os.environ, environment, clear=True))
        self.git("init", "--quiet", "--template=", "--object-format=sha1")

    def git(self, *args):
        return pre_commit.git(self.root, *args).decode().strip()

    def stage(self, name, contents):
        path = self.root / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(contents)
        self.git("add", "--", name)
        return self.git("write-tree")

    def test_unstaged_fix_cannot_hide_broken_staged_source(self):
        tree = self.stage("module.py", "def incomplete(\n")
        (self.root / "module.py").write_text("value = 42\n")
        destination = self.root / "snapshot"
        pre_commit.export_tree(self.root, tree, destination)
        result = subprocess.run([sys.executable, "-m", "py_compile", str(destination / "module.py")], capture_output=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.root / "module.py").read_text(), "value = 42\n")
        pre_commit.require_same_index(self.root, tree)

    def test_unstaged_errors_and_export_attributes_cannot_change_staged_execution(self):
        self.stage(".gitattributes", "dependency.py export-ignore\n")
        self.stage("dependency.py", "def answer():\n    return 42\n")
        tree = self.stage("entry.py", "from dependency import answer\nassert answer() == 42\n")
        (self.root / "dependency.py").write_text("invalid Python here\n")
        (self.root / "private.txt").write_text("not staged\n")
        destination = self.root / "snapshot"
        pre_commit.export_tree(self.root, tree, destination)
        subprocess.run([sys.executable, str(destination / "entry.py")], check=True, capture_output=True)
        self.assertFalse((destination / "private.txt").exists())
        self.assertEqual((self.root / "dependency.py").read_text(), "invalid Python here\n")

    def test_newly_staged_changes_invalidate_completed_checks(self):
        tree = self.stage("input.txt", "reviewed\n")
        pre_commit.require_same_index(self.root, tree)
        self.stage("input.txt", "different\n")
        with self.assertRaises(RuntimeError):
            pre_commit.require_same_index(self.root, tree)

    @unittest.skipUnless(sys.platform == "linux", "local hook targets Linux")
    def test_staged_symlink_cannot_read_unstaged_private_input(self):
        private = self.root / "private.txt"
        private.write_text("not part of the commit\n")
        (self.root / "link.txt").symlink_to("private.txt")
        self.git("add", "link.txt")
        with self.assertRaises(RuntimeError):
            pre_commit.export_tree(self.root, self.git("write-tree"), self.root / "snapshot")
        self.assertEqual(private.read_text(), "not part of the commit\n")

    def test_installer_preserves_existing_hook_and_shared_configuration(self):
        hook = self.root / ".git/hooks/pre-commit"
        hook.parent.mkdir(exist_ok=True)
        hook.write_text("#!/bin/sh\nexit 9\n")
        with self.assertRaises(RuntimeError):
            pre_commit.install(self.root)
        self.assertEqual(hook.read_text(), "#!/bin/sh\nexit 9\n")
        hook.unlink()
        self.git("config", "core.hooksPath", "shared-hooks")
        with self.assertRaises(RuntimeError):
            pre_commit.install(self.root)
        self.assertFalse(hook.exists())


class SourceCoverage(unittest.TestCase):
    def test_one_executed_instantiation_covers_a_region_but_not_a_different_region(self):
        region = [1, 1, 1, 9, 0, 0, 0, 0]
        hit = copy.copy(region)
        hit[4] = 1
        other = [2, 1, 2, 9, 0, 0, 0, 0]
        report = {"data": [{"files": [{"filename": "src/lib.rs"}], "functions": [
            {"filenames": ["src/lib.rs"], "regions": [region, other]},
            {"filenames": ["src/lib.rs"], "regions": [hit]},
        ]}]}
        result = check_rust_coverage.project(report, ["src/lib.rs"])
        self.assertEqual(result["totals"], {"count": 2, "covered": 1})
        self.assertEqual(result["uncovered"], [{"filename": "src/lib.rs", "span": [2, 1, 2, 9]}])
        with self.assertRaises(RuntimeError):
            check_rust_coverage.require_full(result)
        other[4] = 1
        check_rust_coverage.require_full(check_rust_coverage.project(report, ["src/lib.rs"]))

    def test_missing_production_file_or_empty_report_cannot_pass(self):
        report = {"data": [{"files": [], "functions": []}]}
        with self.assertRaises(RuntimeError):
            check_rust_coverage.project(report, ["src/lib.rs"])
        with self.assertRaises(RuntimeError):
            check_rust_coverage.require_full(check_rust_coverage.project(report, []))

    def test_badge_uses_line_counts_without_rounding_incomplete_coverage_to_full(self):
        lines = {"count": 10001, "covered": 10000, "percent": 100}
        report = {"data": [{"totals": {"lines": lines}}]}
        partial = check_rust_coverage.badge(report)
        self.assertEqual(partial["message"], "99.9%")
        lines["covered"] = lines["count"]
        complete = check_rust_coverage.badge(report)
        self.assertEqual(complete["message"], "100%")
        self.assertNotEqual(partial["color"], complete["color"])
        lines.update(count=0, covered=0)
        with self.assertRaises(RuntimeError):
            check_rust_coverage.badge(report)


if __name__ == "__main__":
    unittest.main()
