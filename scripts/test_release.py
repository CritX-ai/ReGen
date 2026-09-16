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


class CandidateHistory:
    """Real isolated Git commits for the tested checkout and its older source."""

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
        self.previous = self.commit({"src/main.rs": "fn main() {}\n"})
        self.source = {
            "README.md": "# Reviewed package\n",
            "src/main.rs": 'fn main() { println!("reviewed"); }\n',
            ".github/workflows/release.yml": "name: reviewed release\non: workflow_dispatch\n",
            ".github/workflows/build.yml": "name: verification\non: push\n",
        }
        self.candidate = self.commit(self.source, [self.previous])
        self.git("update-ref", "HEAD", self.candidate)

    def git(self, *arguments, input=None):
        return self.run(["git", *arguments], cwd=self.root, env=self.environment, input=input,
                        text=True, stdout=release.subprocess.PIPE, stderr=release.subprocess.PIPE, check=True).stdout.strip()

    def commit(self, files, parents=()):
        self.git("read-tree", "--empty")
        for name, data in sorted(files.items()):
            blob = self.git("hash-object", "-w", "--stdin", input=data)
            self.git("update-index", "--add", "--cacheinfo", f"100644,{blob},{name}")
        tree = self.git("write-tree")
        return self.git("commit-tree", tree, *[arg for parent in parents for arg in ("-p", parent)], input="Fixture revision\n")

    def read_only(self, command, *, cwd, text):
        if command != ["git", "rev-parse", "HEAD"] or cwd != self.root:
            raise AssertionError(f"unexpected process: {command}")
        return self.run(command, cwd=cwd, env=self.environment, text=text,
                        stdout=release.subprocess.PIPE, check=True).stdout


class ReleaseSafety(unittest.TestCase):
    def setUp(self):
        self.history = CandidateHistory(Path(self.enterContext(tempfile.TemporaryDirectory(prefix="regen-release-history-test-"))))
        self.candidate = {"version": "1.0.0", "tag": "v1.0.0", "commit": self.history.candidate,
                          "run_id": "10", "run_attempt": "1", "crate_sha256": "b" * 64}
        self.environment = {
            "GITHUB_ACTIONS": "true", "GITHUB_SERVER_URL": "https://github.com",
            "GITHUB_REF": "refs/heads/main", "GITHUB_SHA": self.candidate["commit"],
            "GITHUB_WORKFLOW_REF": "CritX-ai/ReGen/.github/workflows/release.yml@refs/heads/main",
            "GITHUB_WORKFLOW_SHA": self.candidate["commit"],
            "GH_REPO": "CritX-ai/ReGen", "GITHUB_REPOSITORY": "CritX-ai/ReGen",
            "GITHUB_EVENT_NAME": "workflow_dispatch", "GITHUB_RUN_ID": "11",
        }
        self.protection = {
            # A single maintainer authorizes publication through workflow_dispatch.
            "protection_rules": [],
            "deployment_branch_policy": {"protected_branches": False, "custom_branch_policies": True},
        }
        self.remote = {
            # Actions installation-token responses omit per-user permissions.
            "repos/CritX-ai/ReGen": {"full_name": "CritX-ai/ReGen"},
            "repos/CritX-ai/ReGen/actions/runs/10": {"id": 10, "run_attempt": 1, "status": "completed", "conclusion": "success",
                "head_sha": self.candidate["commit"], "head_branch": "main", "path": ".github/workflows/build.yml",
                "head_repository": {"full_name": "CritX-ai/ReGen"}, "event": "push"},
            "repos/CritX-ai/ReGen/git/ref/heads/main": {"object": {"type": "commit", "sha": self.candidate["commit"]}},
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

    def test_local_or_unknown_operations_cannot_authorize_remote_access(self):
        for operation in ("prepare", "verify", "check-publish", "publish"):
            with self.subTest(operation=operation), patch.object(release, "github", side_effect=AssertionError("unexpected remote access")):
                with self.assertRaises(RuntimeError):
                    release.authorize(self.candidate, operation, "10")
        self.process.assert_not_called()

    def test_container_authorization_cannot_dispatch_a_github_publication(self):
        with patch.object(release, "github", side_effect=self.api):
            self.assertEqual(release.authorize(self.candidate, "publish-container", "10"), "CritX-ai/ReGen")
            with self.assertRaises(RuntimeError):
                release.remote_operation(Path("unused"), self.candidate, "publish-container", "10")
        self.process.assert_not_called()

    def test_completed_candidate_is_accepted_but_failed_or_different_sha_is_not(self):
        run = self.remote["repos/CritX-ai/ReGen/actions/runs/10"]
        with patch.object(release, "github", side_effect=self.api):
            self.assertEqual(release.authorize(self.candidate, "publish-crate", "10"), "CritX-ai/ReGen")
            run["conclusion"] = "failure"
            with self.assertRaises(RuntimeError):
                release.authorize(self.candidate, "publish-crate", "10")
            run["conclusion"] = "success"
            run["head_sha"] = "c" * 40
            with self.assertRaises(RuntimeError):
                release.authorize(self.candidate, "publish-crate", "10")

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
                    release.remote_operation(Path("unused"), self.candidate, "authorize", "10")
        self.process.assert_not_called()

    def test_auto_created_or_wrong_repository_environment_fails_closed(self):
        with patch.object(release, "github", side_effect=self.api):
            with patch.dict(self.protection, {"deployment_branch_policy": None}), self.assertRaises(RuntimeError):
                release.authorize(self.candidate, "draft", "10")
            self.remote["repos/CritX-ai/ReGen"]["full_name"] = "other/ReGen"
            with self.assertRaises(RuntimeError):
                release.authorize(self.candidate, "draft", "10")

    def test_candidate_dispatch_workflow_and_checkout_must_share_one_commit(self):
        previous = self.history.previous
        with patch.object(release, "github", side_effect=AssertionError("commit drift must fail before remote access")):
            for changed in (
                {"GITHUB_SHA": previous},
                {"GITHUB_WORKFLOW_SHA": previous},
                {"GITHUB_SHA": previous, "GITHUB_WORKFLOW_SHA": previous},
            ):
                with self.subTest(environment=changed), patch.dict(os.environ, changed), self.assertRaises(RuntimeError):
                    release.remote_operation(Path("unused"), self.candidate, "draft", "10")
            with self.subTest(candidate=previous), self.assertRaises(RuntimeError):
                release.remote_operation(Path("unused"), {**self.candidate, "commit": previous}, "draft", "10")
            self.history.git("update-ref", "HEAD", previous)
            with self.subTest(checkout=previous), self.assertRaises(RuntimeError):
                release.remote_operation(Path("unused"), self.candidate, "draft", "10")
        self.process.assert_not_called()

    def test_explicit_run_and_build_workflow_cannot_be_substituted(self):
        with patch.object(release, "github", side_effect=self.api):
            for requested in (None, "11", "010", "local"):
                with self.subTest(requested=requested), self.assertRaises(RuntimeError):
                    release.authorize(self.candidate, "draft", requested)
            with self.assertRaises(RuntimeError):
                release.authorize({**self.candidate, "run_id": "12"}, "draft", "10")
            run = self.remote["repos/CritX-ai/ReGen/actions/runs/10"]
            for changed in (
                {"id": 12}, {"status": "in_progress"}, {"head_branch": "other"},
                {"path": ".github/workflows/release.yml"}, {"event": "pull_request"},
                {"head_repository": {"full_name": "other/ReGen"}},
            ):
                with self.subTest(run=changed), patch.dict(run, changed), self.assertRaises(RuntimeError):
                    release.authorize(self.candidate, "draft", "10")
        self.process.assert_not_called()

    def test_wrong_workflow_unprotected_environment_and_advanced_main_fail_closed(self):
        with patch.object(release, "github", side_effect=self.api):
            for changed in (
                {"GITHUB_WORKFLOW_REF": "CritX-ai/ReGen/.github/workflows/build.yml@refs/heads/main"},
                {"GITHUB_REF": "refs/tags/v1.0.0"},
                {"GITHUB_RUN_ID": "10"},
            ):
                with self.subTest(environment=changed), patch.dict(os.environ, changed), self.assertRaises(RuntimeError):
                    release.authorize(self.candidate, "draft", "10")
            branches = self.remote["repos/CritX-ai/ReGen/environments/release/deployment-branch-policies?per_page=100"]
            with patch.dict(branches, {"total_count": 2}), self.assertRaises(RuntimeError):
                release.authorize(self.candidate, "draft", "10")
            with patch.dict(self.protection, {"deployment_branch_policy": None}), self.assertRaises(RuntimeError):
                release.authorize(self.candidate, "draft", "10")
            self.remote["repos/CritX-ai/ReGen/git/ref/heads/main"]["object"]["sha"] = "c" * 40
            with self.assertRaises(RuntimeError):
                release.authorize(self.candidate, "draft", "10")
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
                self.remote["repos/CritX-ai/ReGen/git/ref/heads/main"]["object"]["sha"] = self.candidate["commit"]

                def registry(_):
                    self.remote["repos/CritX-ai/ReGen/git/ref/heads/main"]["object"]["sha"] = "c" * 40
                    return {"cksum": self.candidate["crate_sha256"], "yanked": False} if operation == "publish-github" else None

                state = (None, None) if operation == "draft" else (draft, ref)
                with self.subTest(operation=operation), patch.object(release, "github", side_effect=self.api), patch.object(release, "release_state", return_value=state), patch.object(release, "registry_version", side_effect=registry):
                    with self.assertRaises(RuntimeError):
                        release.remote_operation(directory, self.candidate, operation, "10")
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
                    release.remote_operation(Path("unused"), self.candidate, operation, "10")
        self.process.assert_not_called()

    def test_tag_only_partial_release_cannot_be_overwritten(self):
        ref = {"object": {"type": "commit", "sha": self.candidate["commit"]}}
        with patch.object(release, "release_state", return_value=(None, ref)), patch.object(release, "registry_version", return_value=None), patch.object(release, "github", side_effect=self.api):
            with self.assertRaises(RuntimeError):
                release.remote_operation(Path("unused"), self.candidate, "draft", "10")
        self.process.assert_not_called()

    def test_tag_drift_and_main_drift_block_publication(self):
        ref = {"object": {"type": "commit", "sha": "c" * 40}}
        with self.assertRaises(RuntimeError):
            release.require_tag(ref, self.candidate)
        with patch.object(release, "github", return_value=ref):
            with self.assertRaises(RuntimeError):
                release.require_main("CritX-ai/ReGen", self.candidate["commit"])

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
                        release.remote_operation(directory, self.candidate, operation, "10")
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
                release.remote_operation(directory, self.candidate, "draft", "10")
                self.assertEqual(state["assets"], reviewed)
                self.assertEqual(state["ref"], ref)
                self.assertTrue(state["release"]["draft"])
                self.assertIsNone(state["crate"])
                release.remote_operation(directory, self.candidate, "publish-crate", "10")
                self.assertEqual(state["crate"], {"cksum": self.candidate["crate_sha256"], "yanked": False})
                self.assertTrue(state["release"]["draft"])
                with self.assertRaises(RuntimeError):
                    release.remote_operation(directory, self.candidate, "publish-crate", "10")
                release.remote_operation(directory, self.candidate, "publish-github", "10")
                self.assertFalse(state["release"]["draft"])
                with self.assertRaises(RuntimeError):
                    release.remote_operation(directory, self.candidate, "publish-github", "10")
                with patch.dict(os.environ, {"GITHUB_EVENT_NAME": "push"}), self.assertRaises(RuntimeError):
                    release.remote_operation(directory, self.candidate, "draft", "10")
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
        for name in ("dist/index.html", ".regen-stage/state", ".regen-previous/state", "private/secret", ".maintainer/notes"):
            path = self.bundle.root / "examples/minimal" / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(b"not a reviewed source")
        before = self.snapshot()
        candidate = release.prepare(self.bundle.output, "1.0.0", self.bundle.commit)
        self.assertEqual(candidate["crate_sha256"], hashlib.sha256(before[self.bundle.crate.name]).hexdigest())
        after = self.snapshot()
        self.assertEqual({name: after[name] for name in before}, before)
        self.assertEqual(set(after) - set(before), {
            "DEPENDENCIES.json", "CANDIDATE.json", "SHA256SUMS", "release-notes.txt",
            "regen-example-1.0.0.tar.gz", "regen-example-1.0.0.zip",
        })
        self.assertEqual(json.loads(after["DEPENDENCIES.json"])["targets"], self.inventories)
        recorded = dict(line.split("  ", 1)[::-1] for line in after["SHA256SUMS"].decode().splitlines())
        self.assertEqual(recorded, {name: hashlib.sha256(data).hexdigest() for name, data in after.items() if name != "SHA256SUMS"})
        metadata = {path.name: (path.stat().st_mtime_ns, path.stat().st_ino) for path in self.bundle.output.iterdir()}
        self.assertEqual(release.prepare(self.bundle.output, "1.0.0", self.bundle.commit, verify=True), candidate)
        self.assertEqual(self.snapshot(), after)
        self.assertEqual(
            {path.name: (path.stat().st_mtime_ns, path.stat().st_ino) for path in self.bundle.output.iterdir()},
            metadata,
        )
        expected = {
            "regen-example-1.0.0/LICENSE": self.bundle.sources["LICENSE"],
            "regen-example-1.0.0/regen.toml": self.bundle.sources["examples/minimal/regen.toml"],
            "regen-example-1.0.0/content/Über uns.md": self.bundle.sources["examples/minimal/content/Über uns.md"],
        }
        for name in ("regen-example-1.0.0.tar.gz", "regen-example-1.0.0.zip"):
            self.assertEqual(release.archive_files(self.bundle.output / name), expected)

    def rehash(self):
        files = self.snapshot()
        (self.bundle.output / "SHA256SUMS").write_text("".join(
            f"{hashlib.sha256(data).hexdigest()}  {name}\n"
            for name, data in sorted(files.items()) if name != "SHA256SUMS"
        ), encoding="utf-8")

    def replace_archive(self, path, entries):
        if path.suffix == ".zip":
            with zipfile.ZipFile(path, "w") as archive:
                for name, data in entries:
                    archive.writestr(name, data)
        else:
            write_tar(path, entries)

    def test_example_inventory_and_bytes_remain_source_bound_after_rehashing(self):
        release.prepare(self.bundle.output, "1.0.0", self.bundle.commit)
        base = "regen-example-1.0.0"
        for name in (f"{base}.tar.gz", f"{base}.zip"):
            path = self.bundle.output / name
            original = path.read_bytes()
            entries = release.archive_files(path)
            variants = {
                "missing": [(name, data) for name, data in entries.items() if name != f"{base}/regen.toml"],
                "extra": [*entries.items(), (f"{base}/dist/index.html", b"unreviewed output")],
                "changed": [(name, b"unreviewed page" if name.endswith(".md") else data) for name, data in entries.items()],
                "license": [(name, b"unreviewed license" if name.endswith("/LICENSE") else data) for name, data in entries.items()],
                "root": [(name.replace(base, "other-root", 1), data) for name, data in entries.items()],
                "escape": [*entries.items(), (f"{base}/../escaped", b"outside the site")],
            }
            for damage, changed in variants.items():
                with self.subTest(archive=path.name, damage=damage):
                    self.replace_archive(path, changed)
                    self.rehash()
                    before = self.snapshot()
                    with self.assertRaises(RuntimeError):
                        release.prepare(self.bundle.output, "1.0.0", self.bundle.commit, verify=True)
                    self.assertEqual(self.snapshot(), before)
            path.write_bytes(original)
            self.rehash()

    def test_example_crate_substitution_fails_even_with_rehashed_candidate(self):
        release.prepare(self.bundle.output, "1.0.0", self.bundle.commit)
        self.bundle.entries["examples/minimal/regen.toml"] = b'title = "Unreviewed"\n'
        self.bundle.retain()
        for name in ("regen-example-1.0.0.tar.gz", "regen-example-1.0.0.zip"):
            path = self.bundle.output / name
            entries = release.archive_files(path)
            entries["regen-example-1.0.0/regen.toml"] = self.bundle.entries["examples/minimal/regen.toml"]
            self.replace_archive(path, entries.items())
        candidate_path = self.bundle.output / "CANDIDATE.json"
        candidate = json.loads(candidate_path.read_bytes())
        candidate["crate_sha256"] = self.bundle.receipt["crate_sha256"]
        candidate_path.write_text(json.dumps(candidate), encoding="utf-8")
        self.rehash()
        before = self.snapshot()
        with self.assertRaises(RuntimeError):
            release.prepare(self.bundle.output, "1.0.0", self.bundle.commit, verify=True)
        self.assertEqual(self.snapshot(), before)

    def test_missing_example_asset_is_not_recreated_by_verification(self):
        release.prepare(self.bundle.output, "1.0.0", self.bundle.commit)
        (self.bundle.output / "regen-example-1.0.0.zip").unlink()
        self.rehash()
        before = self.snapshot()
        with self.assertRaises(RuntimeError):
            release.prepare(self.bundle.output, "1.0.0", self.bundle.commit, verify=True)
        self.assertEqual(self.snapshot(), before)

    def test_preexisting_example_asset_is_never_overwritten_or_partially_replaced(self):
        path = self.bundle.output / "regen-example-1.0.0.zip"
        path.write_bytes(b"previously retained archive")
        before = self.snapshot()
        with self.assertRaises(RuntimeError):
            release.prepare(self.bundle.output, "1.0.0", self.bundle.commit)
        self.assertEqual(self.snapshot(), before)
        with self.assertRaises(RuntimeError):
            release.package_examples(self.bundle.output, "1.0.0", self.bundle.sources)
        self.assertEqual(self.snapshot(), before)

    def test_example_archives_are_deterministic_across_source_order_and_metadata(self):
        release.prepare(self.bundle.output, "1.0.0", self.bundle.commit)
        destination = self.directory / "standalone"
        destination.mkdir()
        for name in self.bundle.sources:
            path = self.bundle.root / name
            path.chmod(0o700)
            os.utime(path, (123456789, 123456789))
        sources = check_cargo.source_inventory({
            "package": {"include": [f"/{name}" for name in reversed(self.bundle.sources)]},
        })
        release.package_examples(destination, "1.0.0", sources)
        self.assertEqual(
            {path.name: path.read_bytes() for path in destination.iterdir()},
            {name: (self.bundle.output / name).read_bytes() for name in ("regen-example-1.0.0.tar.gz", "regen-example-1.0.0.zip")},
        )

    def test_cli_binds_candidate_version_and_tag_to_cargo_even_with_fresh_checksums(self):
        release.prepare(self.bundle.output, "1.0.0", self.bundle.commit)
        original = self.snapshot()
        arguments = ["release.py", "--directory", str(self.bundle.output),
                     "--commit", self.bundle.commit, "--operation", "verify"]
        with patch("sys.argv", arguments), patch("package.ROOT", self.bundle.root), \
                patch.object(release.subprocess, "check_output", return_value=self.bundle.commit + "\n"):
            release.main()
            self.assertEqual(self.snapshot(), original)
            candidate_path = self.bundle.output / "CANDIDATE.json"
            candidate = json.loads(original["CANDIDATE.json"])
            candidate.update(version="1.0.1", tag="v1.0.1")
            candidate_path.write_text(json.dumps(candidate), encoding="utf-8")
            changed = self.snapshot()
            (self.bundle.output / "SHA256SUMS").write_text("".join(
                f"{hashlib.sha256(data).hexdigest()}  {name}\n"
                for name, data in sorted(changed.items()) if name != "SHA256SUMS"
            ), encoding="utf-8")
            before = self.snapshot()
            with self.assertRaises(RuntimeError):
                release.main()
            self.assertEqual(self.snapshot(), before)

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
        # Retargeting the candidate and its checksums cannot authorize an older
        # source receipt and crate, even when the requested commit now matches.
        with self.assertRaises(RuntimeError):
            release.prepare(self.bundle.output, "1.0.0", candidate["commit"], verify=True)
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
        for name, mode in (("directory/", 0o040755), ("linked", 0o120644)):
            with self.subTest(member=name):
                member = zipfile.ZipInfo(name)
                member.create_system = 3
                member.external_attr = mode << 16
                with zipfile.ZipFile(path, "w") as archive:
                    archive.writestr(member, b"LICENSE")
                with self.assertRaises(RuntimeError):
                    release.archive_files(path)


if __name__ == "__main__":
    unittest.main()
