#!/usr/bin/env python3
"""Offline release-safety regressions using real Git history and blocked remote writes."""

import copy
import hashlib
import io
import json
import os
from pathlib import Path
import tempfile
import unittest
import tarfile
from unittest.mock import patch
import urllib.error
import zipfile

import release
import check_cargo
from test_check_cargo import SourceBundle, write_tar


class ActivationHistory:
    """Real isolated Git objects: the release guard never receives canned diff output."""

    def __init__(self, root):
        self.root = root
        self.run = release.subprocess.run
        self.environment = {key: value for key, value in os.environ.items() if not key.startswith("GIT_")}
        self.environment.update({
            "GIT_CONFIG_NOSYSTEM": "1", "GIT_CONFIG_GLOBAL": os.devnull,
            "GIT_AUTHOR_NAME": "Release regression", "GIT_AUTHOR_EMAIL": "test@example.invalid",
            "GIT_COMMITTER_NAME": "Release regression", "GIT_COMMITTER_EMAIL": "test@example.invalid",
            "GIT_AUTHOR_DATE": "2026-09-15T00:00:00Z", "GIT_COMMITTER_DATE": "2026-09-15T00:00:00Z",
            "GIT_INDEX_FILE": str(root / "fixture-index"),
        })
        self.git("init", "--quiet", "--template=", "--object-format=sha1")
        self.source = {
            "README.md": "# Reviewed package\n",
            "src/main.rs": 'fn main() { println!("reviewed"); }\n',
            ".github/release.yml": "name: reviewed release\non: workflow_dispatch\n",
            ".github/workflows/build.yml": "name: verification\non: push\n",
        }
        self.candidate = self.commit(self.source)
        self.activated = dict(self.source)
        self.activated[".github/workflows/release.yml"] = self.activated.pop(".github/release.yml")
        self.activation = self.commit(self.activated, [self.candidate])
        self.git("update-ref", "HEAD", self.candidate)

    def git(self, *arguments, input=None):
        return self.run(["git", *arguments], cwd=self.root, env=self.environment, input=input,
                        text=True, stdout=release.subprocess.PIPE, stderr=release.subprocess.PIPE, check=True).stdout.strip()

    def commit(self, files, parents=(), modes=None):
        self.git("read-tree", "--empty")
        for name, data in sorted(files.items()):
            blob = self.git("hash-object", "-w", "--stdin", input=data)
            self.git("update-index", "--add", "--cacheinfo", f"{(modes or {}).get(name, '100644')},{blob},{name}")
        tree = self.git("write-tree")
        return self.git("commit-tree", tree, *[arg for parent in parents for arg in ("-p", parent)], input="Fixture revision\n")

    def read_only(self, command, *, cwd, text):
        if command[0] != "git" or command[1] not in {"rev-parse", "rev-list", "diff-tree", "ls-tree"} or cwd != self.root:
            raise AssertionError(f"unexpected process: {command}")
        return self.run(command, cwd=cwd, env=self.environment, text=text,
                        stdout=release.subprocess.PIPE, check=True).stdout


class ReleaseSafety(unittest.TestCase):
    def setUp(self):
        self.history = ActivationHistory(Path(self.enterContext(tempfile.TemporaryDirectory(prefix="regen-activation-test-"))))
        self.activation = self.history.activation
        self.candidate = {"version": "1.0.0", "tag": "v1.0.0", "commit": self.history.candidate,
                          "run_id": "10", "run_attempt": "1", "crate_sha256": "b" * 64}
        self.environment = {
            "GITHUB_ACTIONS": "true", "GITHUB_SERVER_URL": "https://github.com",
            "GITHUB_REF": "refs/heads/main", "GITHUB_SHA": self.activation,
            "GITHUB_WORKFLOW_REF": "CritX-ai/ReGen/.github/workflows/release.yml@refs/heads/main",
            "GITHUB_WORKFLOW_SHA": self.activation,
            "GH_REPO": "CritX-ai/ReGen", "GITHUB_REPOSITORY": "CritX-ai/ReGen",
            "GITHUB_EVENT_NAME": "workflow_dispatch", "GITHUB_RUN_ID": "11",
            "REGEN_RELEASE_VERSION": "1.0.0",
        }
        self.protection = {
            # A single maintainer authorizes publication through workflow_dispatch.
            "protection_rules": [],
            "deployment_branch_policy": {"protected_branches": False, "custom_branch_policies": True},
        }
        self.remote = {
            "repos/CritX-ai/ReGen": {"permissions": {"push": True}},
            "repos/CritX-ai/ReGen/actions/runs/10": {"id": 10, "run_attempt": 1, "status": "completed", "conclusion": "success",
                "head_sha": self.candidate["commit"], "head_branch": "main", "path": ".github/workflows/build.yml",
                "head_repository": {"full_name": "CritX-ai/ReGen"}, "event": "push"},
            "repos/CritX-ai/ReGen/git/ref/heads/main": {"object": {"type": "commit", "sha": self.activation}},
            "repos/CritX-ai/ReGen/environments/release": self.protection,
            "repos/CritX-ai/ReGen/environments/release/deployment-branch-policies?per_page=100": {
                "total_count": 1, "branch_policies": [{"name": "main", "type": "branch"}]},
        }
        self.enterContext(patch.dict(os.environ, self.environment, clear=True))
        self.enterContext(patch.object(release, "ROOT", self.history.root))
        self.process = self.enterContext(patch.object(release.subprocess, "run", side_effect=AssertionError("unexpected process/write")))
        self.enterContext(patch.object(release.subprocess, "check_output", side_effect=self.history.read_only))
        self.enterContext(patch.object(release.urllib.request, "urlopen", side_effect=AssertionError("unexpected network")))

    def api(self, endpoint, payload=None, paginate=False):
        if payload is not None:
            self.fail("unexpected remote write")
        return copy.deepcopy(self.remote[endpoint])

    def test_version_opt_in_is_required_before_remote_access(self):
        for value in ("", "1.0.1"):
            with self.subTest(value=value), patch.dict(os.environ, {"REGEN_RELEASE_VERSION": value}), patch.object(release, "github", side_effect=self.api):
                with self.assertRaises(RuntimeError):
                    release.authorize(self.candidate, "draft", "1.0.0", "10")

    def test_completed_candidate_is_accepted_but_failed_or_different_sha_is_not(self):
        run = self.remote["repos/CritX-ai/ReGen/actions/runs/10"]
        with patch.object(release, "github", side_effect=self.api):
            self.assertEqual(release.authorize(self.candidate, "publish-crate", "1.0.0", "10"), ("CritX-ai/ReGen", self.activation))
            run["conclusion"] = "failure"
            with self.assertRaises(RuntimeError):
                release.authorize(self.candidate, "publish-crate", "1.0.0", "10")
            run["conclusion"] = "success"
            run["head_sha"] = "c" * 40
            with self.assertRaises(RuntimeError):
                release.authorize(self.candidate, "publish-crate", "1.0.0", "10")

    def test_reruns_cannot_replace_the_original_reviewed_bundle(self):
        run = self.remote["repos/CritX-ai/ReGen/actions/runs/10"]
        # Reruns retain GitHub's run ID: matching source and fresh checksums are
        # not enough to authorize regenerated binaries under the old approval.
        for recorded, current in (("1", 2), ("2", 1), ("2", 2)):
            with self.subTest(recorded=recorded, current=current), \
                    patch.dict(self.candidate, {"run_attempt": recorded}), \
                    patch.dict(run, {"run_attempt": current}), \
                    patch.object(release, "github", side_effect=self.api):
                with self.assertRaises(RuntimeError):
                    release.remote_operation(Path("unused"), self.candidate, "authorize", "1.0.0", "10")
        self.process.assert_not_called()

    def test_auto_created_environment_and_hidden_drafts_fail_closed(self):
        with patch.object(release, "github", side_effect=self.api):
            with patch.dict(self.protection, {"deployment_branch_policy": None}), self.assertRaises(RuntimeError):
                release.authorize(self.candidate, "draft", "1.0.0", "10")
            self.remote["repos/CritX-ai/ReGen"]["permissions"]["push"] = False
            with self.assertRaises(RuntimeError):
                release.authorize(self.candidate, "draft", "1.0.0", "10")

    def test_only_exact_activation_preserves_original_candidate_identity(self):
        before = self.history.git("show", f"{self.candidate['commit']}:README.md")
        with patch.object(release, "github", side_effect=self.api):
            release.remote_operation(Path("unused"), self.candidate, "authorize", "1.0.0", "10")
        self.assertEqual(self.history.git("rev-parse", "HEAD"), self.candidate["commit"])
        self.assertEqual(os.environ["GITHUB_SHA"], self.activation)
        self.assertNotEqual(self.candidate["commit"], self.activation)
        self.assertEqual(self.history.git("show", f"{self.activation}:README.md"), before)
        self.process.assert_not_called()

    def test_source_readme_or_activation_edits_cannot_hide_behind_successful_run(self):
        for path, replacement in (
            ("src/main.rs", 'fn main() { println!("unreviewed"); }\n'),
            ("README.md", "# Badge update after testing\n"),
            (".github/workflows/release.yml", "name: unreviewed release\non: push\n"),
            (".github/workflows/extra.yml", "name: extra workflow\non: push\n"),
        ):
            files = {**self.history.activated, path: replacement}
            activation = self.history.commit(files, [self.candidate["commit"]])
            with self.subTest(path=path), patch.dict(os.environ, {"GITHUB_SHA": activation, "GITHUB_WORKFLOW_SHA": activation}), patch.object(release, "github", side_effect=AssertionError("drift must fail before remote access")):
                with self.assertRaises(RuntimeError):
                    release.remote_operation(Path("unused"), self.candidate, "draft", "1.0.0", "10")
        self.process.assert_not_called()

    def test_copy_wrong_destination_mode_and_non_direct_parent_are_not_activation(self):
        copied = {**self.history.source, ".github/workflows/release.yml": self.history.source[".github/release.yml"]}
        wrong_path = dict(self.history.source)
        wrong_path[".github/workflows/publish.yml"] = wrong_path.pop(".github/release.yml")
        intermediate = self.history.commit({**self.history.source, "unreviewed.txt": "extra\n"}, [self.candidate["commit"]])
        revisions = [
            self.history.commit(copied, [self.candidate["commit"]]),
            self.history.commit(wrong_path, [self.candidate["commit"]]),
            self.history.commit(self.history.activated, [self.candidate["commit"]], {".github/workflows/release.yml": "100755"}),
            self.history.commit(self.history.activated, [intermediate]),
            self.history.commit(self.history.activated, [self.candidate["commit"], intermediate]),
            self.candidate["commit"],
        ]
        for activation in revisions:
            with self.subTest(activation=activation), self.assertRaises(RuntimeError):
                release.require_activation(self.candidate["commit"], activation)

    def test_explicit_run_checkout_and_original_workflow_cannot_be_substituted(self):
        with patch.object(release, "github", side_effect=self.api):
            for requested in (None, "11", "010", "local"):
                with self.subTest(requested=requested), self.assertRaises(RuntimeError):
                    release.authorize(self.candidate, "draft", "1.0.0", requested)
            for changed in ({"run_id": "12"}, {"commit": self.activation}):
                with self.subTest(candidate=changed), self.assertRaises(RuntimeError):
                    release.authorize({**self.candidate, **changed}, "draft", "1.0.0", "10")
            run = self.remote["repos/CritX-ai/ReGen/actions/runs/10"]
            for changed in (
                {"id": 12}, {"status": "in_progress"}, {"head_branch": "other"},
                {"path": ".github/workflows/release.yml"}, {"event": "pull_request"},
                {"head_repository": {"full_name": "other/ReGen"}},
            ):
                with self.subTest(run=changed), patch.dict(run, changed), self.assertRaises(RuntimeError):
                    release.authorize(self.candidate, "draft", "1.0.0", "10")
        self.process.assert_not_called()

    def test_wrong_workflow_unprotected_environment_and_advanced_main_fail_closed(self):
        with patch.object(release, "github", side_effect=self.api):
            for changed in (
                {"GITHUB_WORKFLOW_REF": "CritX-ai/ReGen/.github/workflows/build.yml@refs/heads/main"},
                {"GITHUB_WORKFLOW_SHA": self.candidate["commit"]}, {"GITHUB_REF": "refs/tags/v1.0.0"},
                {"GITHUB_RUN_ID": "10"},
            ):
                with self.subTest(environment=changed), patch.dict(os.environ, changed), self.assertRaises(RuntimeError):
                    release.authorize(self.candidate, "draft", "1.0.0", "10")
            branches = self.remote["repos/CritX-ai/ReGen/environments/release/deployment-branch-policies?per_page=100"]
            with patch.dict(branches, {"total_count": 2}), self.assertRaises(RuntimeError):
                release.authorize(self.candidate, "draft", "1.0.0", "10")
            with patch.dict(self.protection, {"deployment_branch_policy": None}), self.assertRaises(RuntimeError):
                release.authorize(self.candidate, "draft", "1.0.0", "10")
            self.remote["repos/CritX-ai/ReGen/git/ref/heads/main"]["object"]["sha"] = self.candidate["commit"]
            with self.assertRaises(RuntimeError):
                release.authorize(self.candidate, "draft", "1.0.0", "10")
        self.process.assert_not_called()

    def test_main_advancing_after_authorization_blocks_each_remote_write(self):
        with tempfile.TemporaryDirectory(prefix="regen-release-race-test-") as temporary:
            directory = Path(temporary)
            notes = b"Reviewed notes\n"
            (directory / "release-notes.txt").write_bytes(notes)
            ref = {"object": {"type": "commit", "sha": self.candidate["commit"]}}
            draft = {"id": 1, "draft": True, "prerelease": False, "body": notes.decode()}
            self.remote["repos/CritX-ai/ReGen/releases/1/assets?per_page=100"] = [[{
                "name": "release-notes.txt", "state": "uploaded", "size": len(notes),
                "digest": "sha256:" + hashlib.sha256(notes).hexdigest(),
            }]]
            for operation in ("draft", "publish-crate", "publish-github"):
                self.remote["repos/CritX-ai/ReGen/git/ref/heads/main"]["object"]["sha"] = self.activation

                def registry(_):
                    self.remote["repos/CritX-ai/ReGen/git/ref/heads/main"]["object"]["sha"] = "c" * 40
                    return {"cksum": self.candidate["crate_sha256"], "yanked": False} if operation == "publish-github" else None

                state = (None, None) if operation == "draft" else (draft, ref)
                with self.subTest(operation=operation), patch.object(release, "github", side_effect=self.api), patch.object(release, "release_state", return_value=state), patch.object(release, "registry_version", side_effect=registry):
                    with self.assertRaises(RuntimeError):
                        release.remote_operation(directory, self.candidate, operation, "1.0.0", "10")
            self.process.assert_not_called()

    def test_registry_errors_are_not_absence(self):
        url = "https://index.crates.io/re/ge/regen-ssg"
        for status in (401, 403, 429, 500):
            with self.subTest(status=status), patch.object(release.urllib.request, "urlopen", side_effect=urllib.error.HTTPError(url, status, "failure", {}, None)):
                with self.assertRaises(RuntimeError):
                    release.public_index("re/ge/regen-ssg", missing=True)
        with patch.object(release.urllib.request, "urlopen", side_effect=[
            urllib.error.HTTPError(url, 404, "missing", {}, None),
            urllib.error.HTTPError(url, 404, "missing", {}, None),
        ]):
            self.assertIsNone(release.public_index("re/ge/regen-ssg", missing=True))
            with self.assertRaises(RuntimeError):
                release.public_index("re/ge/regen-ssg")

    def test_github_api_failure_is_not_an_empty_release_listing(self):
        self.process.side_effect = release.subprocess.CalledProcessError(1, "gh")
        with self.assertRaises(release.subprocess.CalledProcessError):
            release.release_state("CritX-ai/ReGen", "v1.0.0")

    def test_pushes_cannot_authorize_any_remote_operation(self):
        for operation in ("draft", "publish-crate", "publish-github"):
            with self.subTest(operation=operation), patch.dict(os.environ, {"GITHUB_EVENT_NAME": "push"}), patch.object(release, "github", side_effect=AssertionError("unexpected remote access")):
                with self.assertRaises(RuntimeError):
                    release.remote_operation(Path("unused"), self.candidate, operation, "1.0.0", "10")
        self.process.assert_not_called()

    def test_tag_only_partial_release_cannot_be_overwritten(self):
        ref = {"object": {"type": "commit", "sha": self.candidate["commit"]}}
        with patch.object(release, "release_state", return_value=(None, ref)), patch.object(release, "registry_version", return_value=None), patch.object(release, "github", side_effect=self.api):
            with self.assertRaises(RuntimeError):
                release.remote_operation(Path("unused"), self.candidate, "draft", "1.0.0", "10")
        self.process.assert_not_called()

    def test_tag_drift_and_main_drift_block_publication(self):
        ref = {"object": {"type": "commit", "sha": "c" * 40}}
        with self.assertRaises(RuntimeError):
            release.require_tag(ref, self.candidate)
        with patch.object(release, "github", return_value=ref):
            with self.assertRaises(RuntimeError):
                release.require_main("CritX-ai/ReGen", self.activation)

    def test_changed_or_incomplete_draft_assets_block_publication(self):
        with tempfile.TemporaryDirectory(prefix="regen-release-test-") as temporary:
            directory = Path(temporary)
            (directory / "release-notes.txt").write_text("Reviewed notes\n", encoding="utf-8")
            draft = {"id": 1, "draft": True, "prerelease": False, "body": "Reviewed notes\n"}
            asset = {"name": "release-notes.txt", "state": "uploaded", "size": (directory / "release-notes.txt").stat().st_size,
                     "digest": "sha256:" + hashlib.sha256(b"Reviewed notes\n").hexdigest()}
            with patch.object(release, "github", return_value=[[asset]]):
                release.verify_draft("CritX-ai/ReGen", draft, directory)
                asset["digest"] = "sha256:" + "0" * 64
                with self.assertRaises(RuntimeError):
                    release.verify_draft("CritX-ai/ReGen", draft, directory)
            with patch.object(release, "github", return_value=[[]]):
                with self.assertRaises(RuntimeError):
                    release.verify_draft("CritX-ai/ReGen", draft, directory)

    def test_existing_crate_or_missing_registry_confirmation_blocks_publish(self):
        ref = {"object": {"type": "commit", "sha": self.candidate["commit"]}}
        with tempfile.TemporaryDirectory(prefix="regen-release-test-") as temporary:
            directory = Path(temporary)
            notes = b"Reviewed notes\n"
            (directory / "release-notes.txt").write_bytes(notes)
            draft = {"id": 1, "draft": True, "prerelease": False, "body": notes.decode()}
            self.remote["repos/CritX-ai/ReGen/releases/1/assets?per_page=100"] = [[{
                "name": "release-notes.txt", "state": "uploaded", "size": len(notes),
                "digest": "sha256:" + hashlib.sha256(notes).hexdigest(),
            }]]
            states = [
                ("publish-crate", {"cksum": "b" * 64, "yanked": False}),
                ("publish-github", None),
                ("publish-github", {"cksum": "c" * 64, "yanked": False}),
                ("publish-github", {"cksum": "b" * 64, "yanked": True}),
            ]
            for operation, crate in states:
                with self.subTest(operation=operation, crate=crate), patch.object(release, "release_state", return_value=(draft, ref)), patch.object(release, "registry_version", return_value=crate), patch.object(release, "github", side_effect=self.api):
                    with self.assertRaises(RuntimeError):
                        release.remote_operation(directory, self.candidate, operation, "1.0.0", "10")
            self.process.assert_not_called()

    def test_public_index_closes_responses_and_rejects_redirected_absence(self):
        url = "https://index.crates.io/re/ge/regen-ssg"
        response = io.BytesIO(b'{"vers":"1.0.0"}\n')
        response.status, response.url = 200, url
        with patch.object(release.urllib.request, "urlopen", return_value=response):
            self.assertEqual(release.public_index("re/ge/regen-ssg"), '{"vers":"1.0.0"}\n')
        self.assertTrue(response.closed)
        body = io.BytesIO(b"not found")
        error = urllib.error.HTTPError("https://other.invalid/index", 404, "missing", {}, body)
        with patch.object(release.urllib.request, "urlopen", side_effect=error):
            with self.assertRaises(RuntimeError):
                release.public_index("re/ge/regen-ssg", missing=True)
        self.assertTrue(body.closed)

    def test_registry_uses_exact_unambiguous_version_and_public_configuration(self):
        row = {"name": "regen-ssg", "vers": "1.0.0", "cksum": "b" * 64, "yanked": False}
        config = {"api": "https://crates.io"}
        with patch.object(release, "public_index", side_effect=[json.dumps(config), json.dumps(row)]):
            self.assertEqual(release.registry_version("1.0.0"), row)
        with patch.object(release, "public_index", side_effect=[json.dumps(config), json.dumps(row)]):
            self.assertIsNone(release.registry_version("1.0.1"))
        with patch.object(release, "public_index", side_effect=[json.dumps(config), "\n".join([json.dumps(row)] * 2)]):
            with self.assertRaises(RuntimeError):
                release.registry_version("1.0.0")
        with patch.object(release, "public_index", return_value=json.dumps({**config, "auth-required": True})):
            with self.assertRaises(RuntimeError):
                release.registry_version("1.0.0")

    def test_cargo_verification_cannot_expose_secrets_or_upload_changed_bytes(self):
        reviewed = b"reviewed crate bytes"
        self.candidate["crate_sha256"] = hashlib.sha256(reviewed).hexdigest()
        secrets = {"GH_TOKEN": "github-secret", "GITHUB_TOKEN": "actions-secret",
                   "CARGO_REGISTRY_TOKEN": "upload-secret", "CARGO_REGISTRIES_PRIVATE_TOKEN": "private-secret"}
        for dry_run, packaged in ((True, reviewed), (True, b"drifted crate"), (False, reviewed), (False, b"drifted crate")):
            with self.subTest(dry_run=dry_run, drift=packaged != reviewed), patch.dict(os.environ, secrets):
                uploads, builds, build_secrets = [], [], []

                def cargo(command, *, env, **kwargs):
                    self.assertEqual(command[0], "cargo")
                    self.assertNotIn("GH_TOKEN", env)
                    self.assertNotIn("GITHUB_TOKEN", env)
                    target = Path(command[command.index("--target-dir") + 1])
                    crate = target / "package" / "regen-ssg-1.0.0.crate"
                    published = crate.parent / "tmp-crate" / crate.name
                    if "--no-verify" not in command:
                        builds.append(packaged)
                        build_secrets.extend(value for value in env.values() if value in secrets.values())
                    if command[2] == "package" or "--dry-run" in command:
                        self.assertNotIn("CARGO_REGISTRY_TOKEN", env)
                        self.assertNotIn("CARGO_REGISTRIES_PRIVATE_TOKEN", env)
                        if "--dry-run" in command:
                            crate = published
                        crate.parent.mkdir(parents=True, exist_ok=True)
                        crate.write_bytes(packaged)
                    else:
                        self.assertEqual(command[2], "publish")
                        self.assertEqual(env["CARGO_REGISTRY_TOKEN"], secrets["CARGO_REGISTRY_TOKEN"])
                        published.parent.mkdir(parents=True)
                        published.write_bytes(packaged)
                        uploads.append(published.read_bytes())
                    return release.subprocess.CompletedProcess(command, 0)

                self.process.side_effect = cargo
                if packaged != reviewed:
                    with self.assertRaises(RuntimeError):
                        release.cargo_publish(self.candidate, dry_run=dry_run)
                else:
                    release.cargo_publish(self.candidate, dry_run=dry_run)
                self.assertEqual(uploads, [reviewed] if not dry_run and packaged == reviewed else [])
                self.assertEqual(builds, [packaged] if dry_run else [])
                self.assertEqual(build_secrets, [])
                self.assertEqual({key: os.environ[key] for key in secrets}, secrets)

    def test_uncertain_cargo_upload_is_not_retried(self):
        reviewed = b"reviewed crate"
        self.candidate["crate_sha256"] = hashlib.sha256(reviewed).hexdigest()
        uploads = []

        def cargo(command, **kwargs):
            target = Path(command[command.index("--target-dir") + 1])
            crate = target / "package" / "regen-ssg-1.0.0.crate"
            if command[2] == "package":
                crate.parent.mkdir(parents=True)
                crate.write_bytes(reviewed)
                return release.subprocess.CompletedProcess(command, 0)
            self.assertEqual(command[2], "publish")
            uploads.append(crate.read_bytes())
            raise release.subprocess.CalledProcessError(1, command)

        self.process.side_effect = cargo
        with patch.dict(os.environ, {"CARGO_REGISTRY_TOKEN": "upload-secret"}):
            with self.assertRaises(RuntimeError) as failure:
                release.cargo_publish(self.candidate, dry_run=False)
        self.assertIsInstance(failure.exception.__cause__, release.subprocess.CalledProcessError)
        self.assertEqual(uploads, [reviewed])

    def test_draft_crate_and_github_transition_preserves_the_reviewed_candidate(self):
        with tempfile.TemporaryDirectory(prefix="regen-release-test-") as temporary:
            directory = Path(temporary)
            reviewed = {"release-notes.txt": b"Reviewed notes\n", "regen-ssg-1.0.0.crate": b"reviewed crate"}
            for name, data in reviewed.items():
                (directory / name).write_bytes(data)
            self.candidate["crate_sha256"] = hashlib.sha256(reviewed["regen-ssg-1.0.0.crate"]).hexdigest()
            ref = {"ref": "refs/tags/v1.0.0", "object": {"type": "commit", "sha": self.candidate["commit"]}}
            state = {"ref": None, "release": None, "assets": {}, "crate": None, "uploads": 0}
            writes = []

            def api(endpoint, payload=None, paginate=False):
                prefix = "repos/CritX-ai/ReGen/"
                if endpoint == prefix + "git/refs" and payload is not None:
                    self.assertIsNone(state["ref"])
                    state["ref"] = {"ref": payload["ref"], "object": {"type": "commit", "sha": payload["sha"]}}
                    writes.append("tag")
                    return copy.deepcopy(state["ref"])
                if endpoint == prefix + "releases?per_page=100":
                    return [[copy.deepcopy(state["release"])]] if state["release"] else [[]]
                if endpoint == prefix + "git/matching-refs/tags/v1.0.0":
                    return [copy.deepcopy(state["ref"])] if state["ref"] else []
                if endpoint == prefix + "releases/1/assets?per_page=100":
                    return [[{"name": name, "state": "uploaded", "size": len(data),
                              "digest": "sha256:" + hashlib.sha256(data).hexdigest()}
                             for name, data in state["assets"].items()]]
                return self.api(endpoint, payload, paginate)

            def process(command, **kwargs):
                if command[:3] == ["gh", "release", "create"]:
                    self.assertIn("--draft", command)
                    self.assertIsNone(state["release"])
                    end = command.index("--repo")
                    state["assets"] = {Path(path).name: Path(path).read_bytes() for path in command[4:end]}
                    state["release"] = {"id": 1, "tag_name": command[3], "draft": True, "prerelease": False,
                                        "body": Path(command[command.index("--notes-file") + 1]).read_text(encoding="utf-8")}
                    writes.append("draft")
                elif command[0] == "cargo":
                    target = Path(command[command.index("--target-dir") + 1])
                    crate = target / "package" / "regen-ssg-1.0.0.crate"
                    if command[2] == "package":
                        crate.parent.mkdir(parents=True)
                        crate.write_bytes(reviewed[crate.name])
                    elif command[2] == "publish":
                        crate = crate.parent / "tmp-crate" / crate.name
                        crate.parent.mkdir(parents=True)
                        crate.write_bytes(reviewed[crate.name])
                        self.assertIsNone(state["crate"])
                        state["crate"] = {"cksum": hashlib.sha256(crate.read_bytes()).hexdigest(), "yanked": False}
                        state["uploads"] += 1
                        writes.append("crate")
                    else:
                        self.fail("unexpected Cargo action")
                elif command[:2] == ["gh", "api"]:
                    self.assertEqual(command[command.index("--method") + 1], "PATCH")
                    self.assertIn("repos/CritX-ai/ReGen/releases/1", command)
                    self.assertIsNotNone(state["crate"])
                    state["release"].update(json.loads(kwargs["input"]))
                    writes.append("github")
                    return release.subprocess.CompletedProcess(command, 0, json.dumps(state["release"]))
                else:
                    self.fail("unexpected process")
                return release.subprocess.CompletedProcess(command, 0)

            self.process.side_effect = process
            with patch.object(release, "github", side_effect=api), patch.object(release, "registry_version", side_effect=lambda _: copy.deepcopy(state["crate"])), patch.dict(os.environ, {"CARGO_REGISTRY_TOKEN": "upload-secret"}):
                release.remote_operation(directory, self.candidate, "draft", "1.0.0", "10")
                self.assertEqual(state["assets"], reviewed)
                self.assertEqual(state["ref"], ref)
                self.assertTrue(state["release"]["draft"])
                self.assertIsNone(state["crate"])
                release.remote_operation(directory, self.candidate, "publish-crate", "1.0.0", "10")
                self.assertEqual(state["crate"], {"cksum": self.candidate["crate_sha256"], "yanked": False})
                self.assertTrue(state["release"]["draft"])
                with self.assertRaises(RuntimeError):
                    release.remote_operation(directory, self.candidate, "publish-crate", "1.0.0", "10")
                release.remote_operation(directory, self.candidate, "publish-github", "1.0.0", "10")
                self.assertFalse(state["release"]["draft"])
                with self.assertRaises(RuntimeError):
                    release.remote_operation(directory, self.candidate, "publish-github", "1.0.0", "10")
                with patch.dict(os.environ, {"GITHUB_EVENT_NAME": "push"}), self.assertRaises(RuntimeError):
                    release.remote_operation(directory, self.candidate, "draft", None, "10")
            self.assertEqual(writes, ["tag", "draft", "crate", "github"])
            self.assertEqual(state["uploads"], 1)
            self.assertEqual(state["assets"], reviewed)
            self.assertEqual(state["release"], {"id": 1, "tag_name": "v1.0.0", "draft": False,
                                              "prerelease": False, "body": reviewed["release-notes.txt"].decode()})
            self.assertEqual({path.name: path.read_bytes() for path in directory.iterdir()}, reviewed)


class CandidateBytes(unittest.TestCase):
    def setUp(self):
        self.directory = Path(self.enterContext(tempfile.TemporaryDirectory(prefix="regen-candidate-test-")))
        self.bundle = SourceBundle(self.directory)
        self.enterContext(patch.object(check_cargo, "ROOT", self.bundle.root))
        self.enterContext(patch.dict(os.environ, {"GITHUB_RUN_ID": "10"}, clear=True))
        self.enterContext(patch.object(release.subprocess, "run", side_effect=AssertionError("unexpected process")))
        self.enterContext(patch.object(release.subprocess, "check_output", side_effect=AssertionError("unexpected process")))
        self.enterContext(patch.object(release.urllib.request, "urlopen", side_effect=AssertionError("unexpected network")))
        self.inventories = []
        for target in release.TARGETS:
            base = f"regen-1.0.0-{target}"
            inventory = {"format": 1, "package": "regen-ssg", "version": "1.0.0", "target": target,
                         "packages": [{"name": "dependency", "version": "1.0.0", "notices": ["dependency/LICENSE"]}],
                         "rust": {"version": "1.98.1", "notices": ["rust/LICENSE"]}}
            self.inventories.append(inventory)
            notices = {"licenses/inventory.json": json.dumps(inventory).encode(),
                       "licenses/dependency/LICENSE": b"Dependency license\n", "licenses/rust/LICENSE": b"Rust license\n"}
            package = {**notices, "LICENSE": b"Project license\n", "THIRD-PARTY-NOTICES.txt": b"Notices\n",
                       "regen.exe" if "windows" in target else "regen": b"archive fixture, not a native executable"}
            if "windows" in target:
                with zipfile.ZipFile(self.bundle.output / f"{base}.zip", "w") as archive:
                    for name, data in package.items():
                        archive.writestr(f"{base}/{name}", data)
            else:
                write_tar(self.bundle.output / f"{base}.tar.gz", [(f"{base}/{name}", data) for name, data in package.items()])
            write_tar(self.bundle.output / f"{base}-licenses.tar.gz", [(f"{base}-licenses/{name}", data) for name, data in notices.items()])
            (self.bundle.output / f"{base}-dependencies.json").write_text(json.dumps(inventory), encoding="utf-8")

    def snapshot(self):
        return {path.name: path.read_bytes() for path in self.bundle.output.iterdir()}

    def test_preparation_retains_exact_inputs_and_verification_is_read_only(self):
        before = self.snapshot()
        candidate = release.prepare(self.bundle.output, "1.0.0", self.bundle.commit)
        self.assertEqual(candidate["crate_sha256"], hashlib.sha256(before[self.bundle.crate.name]).hexdigest())
        after = self.snapshot()
        self.assertEqual({name: after[name] for name in before}, before)
        self.assertEqual(set(after) - set(before), {"DEPENDENCIES.json", "CANDIDATE.json", "SHA256SUMS", "release-notes.txt"})
        self.assertEqual(json.loads(after["DEPENDENCIES.json"])["targets"], self.inventories)
        recorded = dict(line.split("  ", 1)[::-1] for line in after["SHA256SUMS"].decode().splitlines())
        self.assertEqual(recorded, {name: hashlib.sha256(data).hexdigest() for name, data in after.items() if name != "SHA256SUMS"})
        self.assertEqual(release.prepare(self.bundle.output, "1.0.0", self.bundle.commit, verify=True), candidate)
        self.assertEqual(self.snapshot(), after)

    def test_candidate_identity_and_notes_drift_are_not_repaired_by_verification(self):
        release.prepare(self.bundle.output, "1.0.0", self.bundle.commit)
        candidate_path = self.bundle.output / "CANDIDATE.json"
        original = candidate_path.read_bytes()
        checksum_path = self.bundle.output / "SHA256SUMS"
        original_checksums = checksum_path.read_bytes()
        candidate = json.loads(original)
        candidate["commit"] = "b" * 40
        candidate_path.write_text(json.dumps(candidate), encoding="utf-8")
        changed = self.snapshot()
        checksum_path.write_text("".join(
            f"{hashlib.sha256(data).hexdigest()}  {name}\n"
            for name, data in sorted(changed.items()) if name != "SHA256SUMS"
        ), encoding="utf-8")
        before = self.snapshot()
        with self.assertRaises(RuntimeError):
            release.prepare(self.bundle.output, "1.0.0", self.bundle.commit, verify=True)
        self.assertEqual(self.snapshot(), before)
        candidate_path.write_bytes(original)
        checksum_path.write_bytes(original_checksums)
        (self.bundle.output / "release-notes.txt").write_bytes(b"unreviewed notes")
        before = self.snapshot()
        with self.assertRaises(RuntimeError):
            release.prepare(self.bundle.output, "1.0.0", self.bundle.commit, verify=True)
        self.assertEqual(self.snapshot(), before)

    def test_archive_notice_drift_blocks_candidate_before_generated_assets(self):
        target = release.TARGETS[0]
        base = f"regen-1.0.0-{target}-licenses"
        path = self.bundle.output / f"{base}.tar.gz"
        entries = release.archive_files(path)
        entries[f"{base}/licenses/dependency/LICENSE"] = b"Changed license\n"
        write_tar(path, list(entries.items()))
        before = self.snapshot()
        with self.assertRaises(RuntimeError):
            release.prepare(self.bundle.output, "1.0.0", self.bundle.commit)
        self.assertEqual(self.snapshot(), before)

    def test_archive_duplicates_and_nonfiles_are_not_silently_collapsed(self):
        path = self.directory / "duplicate.tar.gz"
        write_tar(path, [("LICENSE", b"original"), ("LICENSE", b"replacement")])
        with self.assertRaises(RuntimeError):
            release.archive_files(path)
        member = tarfile.TarInfo("linked")
        member.type = tarfile.SYMTYPE
        member.linkname = "LICENSE"
        write_tar(path, [(member, b"")])
        with self.assertRaises(RuntimeError):
            release.archive_files(path)
        path = self.directory / "duplicate.zip"
        with zipfile.ZipFile(path, "w") as archive:
            archive.writestr("LICENSE", b"original")
            with self.assertWarns(UserWarning):
                archive.writestr("LICENSE", b"replacement")
        with self.assertRaises(RuntimeError):
            release.archive_files(path)


if __name__ == "__main__":
    unittest.main()
