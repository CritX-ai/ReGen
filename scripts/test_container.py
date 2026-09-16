#!/usr/bin/env python3
"""Defend local cleanup ownership and retained-image publication authorization."""

import copy
import hashlib
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch
import urllib.error

import container
import release
from test_release import CandidateHistory


class LocalOwnership(unittest.TestCase):
    def test_failed_smoke_removes_own_tag_without_removing_shared_image(self):
        with tempfile.TemporaryDirectory(prefix="regen-container-test-") as temporary:
            directory = Path(temporary)
            image_id = "a" * 64
            images = {"localhost/my-existing-image:keep": image_id}

            def fail_smoke(directory, arch, expected, image, runtime):
                images[image] = image_id
                raise subprocess.CalledProcessError(29, [runtime, "run"])

            def inspect_tags(*command):
                reference = next(value.removeprefix("reference=") for value in command if value.startswith("reference="))
                return images.get(reference, "")

            def remove_image(*command):
                selected = command[-1]
                if selected in images and "--force" not in command:
                    del images[selected]
                else:
                    for tag in list(images):
                        if images[tag] == selected or (selected in images and images[tag] == images[selected]):
                            del images[tag]

            with patch.object(container, "local_runtime", return_value="podman"), \
                    patch.object(container, "require_native_runtime"), \
                    patch.object(container, "build_and_smoke", side_effect=fail_smoke), \
                    patch.object(container, "capture", side_effect=inspect_tags), \
                    patch.object(container, "run", side_effect=remove_image):
                with self.assertRaises(subprocess.CalledProcessError) as failure:
                    container.local(directory, "x86_64-unknown-linux-gnu")
            self.assertEqual(failure.exception.returncode, 29)
            self.assertEqual(images, {"localhost/my-existing-image:keep": image_id})
            self.assertEqual(list(directory.iterdir()), [])

    def test_unusable_preferred_runtime_is_not_replaced_with_another_engine(self):
        def runtime(*command):
            if command[0] == "podman":
                raise subprocess.CalledProcessError(125, command)
            if command[1] == "info":
                return json.dumps({"OSType": "linux", "Architecture": "x86_64"})
            return ""

        with patch.dict(os.environ, {}, clear=True), \
                patch.object(container.shutil, "which", side_effect=lambda name: f"/usr/bin/{name}"), \
                patch.object(container.platform, "system", return_value="Linux"), \
                patch.object(container.platform, "machine", return_value="x86_64"), \
                patch.object(container.os, "getuid", return_value=1000), \
                patch.object(container, "build_and_smoke", return_value="sha256:" + "a" * 64), \
                patch.object(container, "capture", side_effect=runtime):
            with self.assertRaises(subprocess.CalledProcessError) as failure:
                container.local(Path("unused"), "x86_64-unknown-linux-gnu")
        self.assertEqual(failure.exception.returncode, 125)


class PublicationAuthorization(unittest.TestCase):
    def setUp(self):
        root = Path(self.enterContext(tempfile.TemporaryDirectory(prefix="regen-container-publish-test-")))
        (root / "history").mkdir()
        self.history = CandidateHistory(root / "history")
        self.directory, self.candidate_directory = root / "images", root / "candidate"
        self.directory.mkdir()
        self.candidate_directory.mkdir()
        self.candidate = {"commit": self.history.candidate, "version": "1.1.0", "tag": "v1.1.0",
                          "run_id": "41", "run_attempt": "1", "crate_sha256": "d" * 64}
        self.expected = {key: self.candidate[key] for key in ("commit", "version", "run_id")}
        self.expected["source"] = "https://github.com/CritX-ai/ReGen"
        self.ids = {"amd64": "sha256:" + "a" * 64, "arm64": "sha256:" + "b" * 64}
        self.retained = dict(self.ids)
        self.labels = {arch: {f"org.opencontainers.image.{key}": value for key, value in (
            ("source", self.expected["source"]), ("version", self.expected["version"]), ("revision", self.expected["commit"]))}
            for arch in container.TARGETS}
        self.images, self.registry_images, self.indexes = {}, {}, {}
        self.writes, self.loads, self.write_loads, self.summaries = [], [], [], []
        self.load_hook = self.push_hook = lambda arch: None
        notes = b"Reviewed candidate notes\n"
        (self.candidate_directory / "release-notes.txt").write_bytes(notes)
        self.published = {"id": 1, "tag_name": "v1.1.0", "draft": False, "prerelease": False,
                          "published_at": "2026-09-16T00:00:00Z", "body": notes.decode()}
        self.ref = {"ref": "refs/tags/v1.1.0", "object": {"type": "commit", "sha": self.candidate["commit"]}}
        self.crate = {"cksum": self.candidate["crate_sha256"], "yanked": False}
        self.remote = {
            "repos/CritX-ai/ReGen": {"full_name": "CritX-ai/ReGen"},
            "repos/CritX-ai/ReGen/actions/runs/41": {
                "id": 41, "run_attempt": 1, "status": "completed", "conclusion": "success",
                "head_sha": self.candidate["commit"], "head_branch": "main", "event": "push",
                "head_repository": {"full_name": "CritX-ai/ReGen"}, "path": ".github/workflows/build.yml"},
            "repos/CritX-ai/ReGen/environments/release": {
                "deployment_branch_policy": {"protected_branches": False, "custom_branch_policies": True}},
            "repos/CritX-ai/ReGen/environments/release/deployment-branch-policies?per_page=100": {
                "total_count": 1, "branch_policies": [{"name": "main", "type": "branch"}]},
            "repos/CritX-ai/ReGen/git/ref/heads/main": {"object": {"type": "commit", "sha": self.candidate["commit"]}},
            "repos/CritX-ai/ReGen/releases?per_page=100": [[self.published]],
            "repos/CritX-ai/ReGen/git/matching-refs/tags/v1.1.0": [self.ref],
            "repos/CritX-ai/ReGen/releases/1/assets?per_page=100": [[{
                "name": "release-notes.txt", "state": "uploaded", "size": len(notes),
                "digest": "sha256:" + hashlib.sha256(notes).hexdigest()}]],
        }
        self.enterContext(patch.dict(os.environ, {
            "GITHUB_ACTIONS": "true", "GITHUB_SERVER_URL": "https://github.com",
            "GITHUB_EVENT_NAME": "workflow_dispatch", "GITHUB_REF": "refs/heads/main",
            "GITHUB_REPOSITORY": "CritX-ai/ReGen", "GH_REPO": "CritX-ai/ReGen",
            "GITHUB_SHA": self.candidate["commit"], "GITHUB_WORKFLOW_SHA": self.candidate["commit"],
            "GITHUB_WORKFLOW_REF": "CritX-ai/ReGen/.github/workflows/release.yml@refs/heads/main",
            "GITHUB_RUN_ID": "99",
        }, clear=True))
        self.enterContext(patch.object(release, "ROOT", self.history.root))
        self.enterContext(patch.object(release.subprocess, "check_output", side_effect=self.history.read_only))
        self.enterContext(patch.object(release.subprocess, "run", side_effect=AssertionError("unexpected process/write")))
        self.enterContext(patch.object(release, "github", side_effect=self.api))
        self.enterContext(patch.object(container, "version", return_value=self.candidate["version"]))
        self.enterContext(patch.object(container, "prepare", side_effect=lambda *args, **kwargs: copy.deepcopy(self.candidate)))
        self.enterContext(patch.object(container, "registry_version", side_effect=lambda _: copy.deepcopy(self.crate)))
        self.enterContext(patch.object(container, "registry_token", return_value="read-token"))
        self.enterContext(patch.object(container, "registry_response", side_effect=self.registry))
        self.enterContext(patch.object(container, "run", side_effect=self.runtime))
        self.enterContext(patch.object(container, "capture", side_effect=self.capture))
        self.enterContext(patch.object(container, "summary", side_effect=self.summaries.append))
        for arch in container.TARGETS:
            self.receipt(arch)
            (self.directory / f"{arch}.tar").write_bytes(b"retained image archive")

    def api(self, endpoint, payload=None, paginate=False):
        if payload is not None:
            self.fail("unexpected GitHub write")
        return copy.deepcopy(self.remote[endpoint])

    def receipt(self, platform_arch, **changes):
        value = {**self.expected, "arch": platform_arch, "image_id": self.ids[platform_arch], **changes}
        (self.directory / f"{platform_arch}.json").write_text(json.dumps(value))

    def publish(self):
        container.publish(self.directory, self.candidate_directory, "41")

    def registry(self, url, authorization, missing=False):
        tag = url.rsplit("/", 1)[1]
        return self.registry_images.get(tag)

    def store(self, tag, manifest):
        body = json.dumps(manifest).encode()
        digest = "sha256:" + hashlib.sha256(body).hexdigest()
        self.registry_images[tag] = ({"Content-Type": manifest["mediaType"], "Docker-Content-Digest": digest}, body)
        return digest

    def runtime(self, *command):
        if command[:3] == ("docker", "image", "load"):
            arch = Path(command[-1]).stem
            self.images[container.reference(self.expected["commit"], arch)] = (arch, self.retained[arch])
            self.loads.append(arch)
            self.load_hook(arch)
        elif command[:3] == ("docker", "image", "tag"):
            image_id, image = command[-2:]
            arch = next(arch for arch, value in self.ids.items() if value == image_id)
            self.images[image] = (arch, image_id)
        elif command[:3] == ("docker", "image", "push"):
            image = command[-1]
            arch, image_id = self.images[image]
            self.writes.append(image)
            self.write_loads.append(tuple(self.loads))
            self.store(image.rsplit(":", 1)[1], {"schemaVersion": 2, "mediaType": container.MANIFEST_TYPES[0],
                                                "config": {"digest": image_id}, "layers": []})
            self.push_hook(arch)
        elif command[:3] == ("docker", "manifest", "create"):
            self.indexes[command[3]] = command[4:]
        else:
            self.fail(f"unexpected container action: {command}")

    def capture(self, *command):
        if command[:3] == ("docker", "image", "inspect"):
            arch, image_id = self.images[command[-1]]
            return json.dumps([{"Id": image_id, "Os": "linux", "Architecture": arch,
                                "Config": {"User": "65532:65532", "Labels": self.labels[arch]}}])
        if command[:3] == ("docker", "manifest", "push"):
            image = command[-1]
            self.writes.append(image)
            self.write_loads.append(tuple(self.loads))
            manifests = []
            for reference in self.indexes[image]:
                digest = reference.split("@", 1)[1]
                manifest = next(json.loads(body) for headers, body in self.registry_images.values()
                                if headers["Docker-Content-Digest"] == digest)
                arch = next(arch for arch, image_id in self.ids.items() if image_id == manifest["config"]["digest"])
                manifests.append({"digest": digest, "platform": {"os": "linux", "architecture": arch}})
            return self.store(image.rsplit(":", 1)[1], {
                "schemaVersion": 2, "mediaType": container.INDEX_TYPES[0], "manifests": manifests})
        self.fail(f"unexpected container inspection: {command}")

    def test_released_version_promotes_original_images_and_exact_multiplatform_digest(self):
        self.publish()
        self.assertEqual(self.writes, [f"{container.IMAGE}:v1.1.0-amd64",
                                      f"{container.IMAGE}:v1.1.0-arm64", f"{container.IMAGE}:v1.1.0"])
        self.assertEqual(self.write_loads, [("amd64", "arm64")] * 3)
        index_headers, index_bytes = self.registry_images["v1.1.0"]
        index = json.loads(index_bytes)
        for item in index["manifests"]:
            arch = item["platform"]["architecture"]
            headers, data = self.registry_images[f"v1.1.0-{arch}"]
            self.assertEqual(item["digest"], headers["Docker-Content-Digest"])
            self.assertEqual(json.loads(data)["config"]["digest"], self.ids[arch])
        self.assertIn(f"{container.IMAGE}@{index_headers['Docker-Content-Digest']}", self.summaries[0])
        with self.assertRaises(RuntimeError):
            self.publish()
        self.assertEqual(len(self.writes), 3)

    def test_main_push_and_pull_request_cannot_publish_matching_receipts(self):
        for event in ("push", "pull_request"):
            with self.subTest(event=event), patch.dict(os.environ, {"GITHUB_EVENT_NAME": event}):
                with self.assertRaises(RuntimeError):
                    self.publish()
        self.assertEqual(self.loads, [])
        self.assertEqual(self.writes, [])

    def test_fork_cannot_load_or_publish_to_the_upstream_namespace(self):
        with patch.dict(os.environ, {"GITHUB_REPOSITORY": "someone/ReGen", "GH_REPO": "someone/ReGen"}):
            with self.assertRaises(RuntimeError):
                self.publish()
        self.assertEqual(self.loads, [])
        self.assertEqual(self.writes, [])

    def test_dispatch_workflow_environment_and_original_run_gates_still_apply(self):
        for changed in (
            {"GITHUB_WORKFLOW_REF": "CritX-ai/ReGen/.github/workflows/build.yml@refs/heads/main"},
            {"GITHUB_REF": "refs/tags/v1.1.0"}, {"GITHUB_SHA": "f" * 40},
            {"GITHUB_WORKFLOW_SHA": "f" * 40}, {"GITHUB_RUN_ID": "41"},
        ):
            with self.subTest(environment=changed), patch.dict(os.environ, changed), self.assertRaises(RuntimeError):
                self.publish()
        run = self.remote["repos/CritX-ai/ReGen/actions/runs/41"]
        for changed in ({"run_attempt": 2}, {"conclusion": "failure"}, {"head_sha": "f" * 40},
                        {"path": ".github/workflows/release.yml"}):
            with self.subTest(run=changed), patch.dict(run, changed), self.assertRaises(RuntimeError):
                self.publish()
        with patch.dict(self.candidate, {"run_attempt": "2"}), self.assertRaises(RuntimeError):
            self.publish()
        environment = self.remote["repos/CritX-ai/ReGen/environments/release"]
        with patch.dict(environment, {"deployment_branch_policy": None}), self.assertRaises(RuntimeError):
            self.publish()
        self.assertEqual(self.loads, [])
        self.assertEqual(self.writes, [])

    def test_unpublished_prerelease_or_altered_release_is_rejected(self):
        for changed in ({"draft": True}, {"prerelease": True}, {"published_at": None},
                        {"body": "changed notes"}, {"tag_name": "v1.0.0"}):
            with self.subTest(release=changed), patch.dict(self.published, changed), self.assertRaises(RuntimeError):
                self.publish()
        asset = self.remote["repos/CritX-ai/ReGen/releases/1/assets?per_page=100"][0][0]
        with patch.dict(asset, {"digest": "sha256:" + "f" * 64}), self.assertRaises(RuntimeError):
            self.publish()
        self.assertEqual(self.writes, [])

    def test_matching_unyanked_crate_and_exact_commit_tag_are_required(self):
        for crate in (None, {"cksum": "f" * 64, "yanked": False},
                      {"cksum": self.candidate["crate_sha256"], "yanked": True}):
            with self.subTest(crate=crate), patch.object(self, "crate", crate), self.assertRaises(RuntimeError):
                self.publish()
        for obj in ({"type": "tag", "sha": self.candidate["commit"]}, {"type": "commit", "sha": "f" * 40}):
            with self.subTest(tag=obj), patch.dict(self.ref, {"object": obj}), self.assertRaises(RuntimeError):
                self.publish()
        self.assertEqual(self.writes, [])

    def test_foreign_or_altered_second_platform_receipt_blocks_all_writes(self):
        for changed in ({"run_id": "99"}, {"commit": "f" * 40}, {"version": "1.0.0"},
                        {"source": "https://github.com/other/ReGen"}, {"arch": "amd64"},
                        {"image_id": "sha256:" + "c" * 64}):
            with self.subTest(receipt=changed):
                self.receipt("arm64", **changed)
                with self.assertRaises(RuntimeError):
                    self.publish()
        self.assertEqual(self.writes, [])

    def test_changed_second_image_or_labels_block_all_registry_writes(self):
        with patch.dict(self.retained, {"arm64": "sha256:" + "c" * 64}), self.assertRaises(RuntimeError):
            self.publish()
        with patch.dict(self.labels["arm64"], {"org.opencontainers.image.revision": "f" * 40}), self.assertRaises(RuntimeError):
            self.publish()
        self.assertEqual(self.writes, [])

    def test_existing_version_or_partial_platform_never_gets_overwritten(self):
        for tag in ("v1.1.0", "v1.1.0-amd64", "v1.1.0-arm64"):
            with self.subTest(tag=tag):
                self.registry_images.clear()
                self.store(tag, {"schemaVersion": 2, "mediaType": container.MANIFEST_TYPES[0], "config": {"digest": self.ids["amd64"]}})
                before = copy.deepcopy(self.registry_images)
                with self.assertRaises(RuntimeError):
                    self.publish()
                self.assertEqual(self.registry_images, before)
        self.assertEqual(self.writes, [])

    def test_unknown_registry_state_does_not_authorize_publication(self):
        self.registry_images["v1.1.0"] = ({"Content-Type": container.MANIFEST_TYPES[0]}, b'{"schemaVersion":2}')
        with self.assertRaises(RuntimeError):
            self.publish()
        self.assertEqual(self.writes, [])

    def test_tag_and_collision_are_rechecked_after_loading(self):
        def drift(arch):
            if arch == "arm64":
                self.ref["object"]["sha"] = "f" * 40

        self.load_hook = drift
        with self.assertRaises(RuntimeError):
            self.publish()
        self.ref["object"]["sha"] = self.candidate["commit"]

        def collision(arch):
            if arch == "arm64":
                self.store("v1.1.0-arm64", {"schemaVersion": 2, "mediaType": container.MANIFEST_TYPES[0],
                                          "config": {"digest": self.ids["arm64"]}})

        self.load_hook = collision
        with self.assertRaises(RuntimeError):
            self.publish()
        self.assertEqual(self.writes, [])

    def test_main_advancing_after_first_push_blocks_remaining_publication(self):
        def advance(arch):
            self.remote["repos/CritX-ai/ReGen/git/ref/heads/main"]["object"]["sha"] = "f" * 40

        self.push_hook = advance
        with self.assertRaises(RuntimeError):
            self.publish()
        self.assertEqual(self.writes, [f"{container.IMAGE}:v1.1.0-amd64"])
        self.assertNotIn("v1.1.0", self.registry_images)

    def test_wrong_pushed_image_cannot_become_a_release_index(self):
        def replace(arch):
            self.store(f"v1.1.0-{arch}", {"schemaVersion": 2, "mediaType": container.MANIFEST_TYPES[0],
                                        "config": {"digest": "sha256:" + "f" * 64}})

        self.push_hook = replace
        with self.assertRaises(RuntimeError):
            self.publish()
        self.assertEqual(self.writes, [f"{container.IMAGE}:v1.1.0-amd64"])
        self.assertNotIn("v1.1.0", self.registry_images)

    def test_earlier_platform_drift_blocks_index_publication(self):
        def replace(arch):
            if arch == "arm64":
                self.store("v1.1.0-amd64", {"schemaVersion": 2, "mediaType": container.MANIFEST_TYPES[0],
                                          "config": {"digest": "sha256:" + "f" * 64}})

        self.push_hook = replace
        with self.assertRaises(RuntimeError):
            self.publish()
        self.assertEqual(self.writes, [f"{container.IMAGE}:v1.1.0-amd64", f"{container.IMAGE}:v1.1.0-arm64"])
        self.assertNotIn("v1.1.0", self.registry_images)

    def test_wrong_index_platforms_cannot_report_success(self):
        def capture(*command):
            result = self.capture(*command)
            if command[:3] == ("docker", "manifest", "push"):
                headers, body = self.registry_images["v1.1.0"]
                index = json.loads(body)
                index["manifests"][1]["platform"]["architecture"] = "amd64"
                return self.store("v1.1.0", index)
            return result

        with patch.object(container, "capture", side_effect=capture), self.assertRaises(RuntimeError):
            self.publish()
        self.assertEqual(self.summaries, [])
        self.assertEqual(self.writes, [f"{container.IMAGE}:v1.1.0-amd64",
                                      f"{container.IMAGE}:v1.1.0-arm64", f"{container.IMAGE}:v1.1.0"])

    def test_uncertain_push_is_not_retried_or_promoted(self):
        def fail(arch):
            raise subprocess.CalledProcessError(1, "docker push")

        self.push_hook = fail
        with self.assertRaises(subprocess.CalledProcessError):
            self.publish()
        self.assertEqual(self.writes, [f"{container.IMAGE}:v1.1.0-amd64"])
        self.assertNotIn("v1.1.0", self.registry_images)


class RegistryPreflight(unittest.TestCase):
    def setUp(self):
        self.url = "https://ghcr.io/v2/critx-ai/regen-ssg/manifests/v1.1.0"
        self.opener = self.enterContext(patch.object(container.urllib.request, "build_opener")).return_value

    def test_only_explicit_manifest_or_repository_absence_is_accepted(self):
        for code in ("MANIFEST_UNKNOWN", "NAME_UNKNOWN"):
            body = io.BytesIO(json.dumps({"errors": [{"code": code}]}).encode())
            self.opener.open.side_effect = urllib.error.HTTPError(self.url, 404, "missing", {}, body)
            with self.subTest(code=code):
                self.assertIsNone(container.registry_manifest("v1.1.0", "read-token"))
                self.assertTrue(body.closed)

    def test_auth_network_malformed_and_redirected_errors_are_not_absence(self):
        errors = [
            urllib.error.HTTPError(self.url, status, "failure", {}, io.BytesIO(b'{"errors":[{"code":"MANIFEST_UNKNOWN"}]}'))
            for status in (401, 403, 429, 500)
        ]
        errors.extend([
            urllib.error.HTTPError(self.url, 404, "missing", {}, io.BytesIO(body))
            for body in (b"not found", b"{}", b'{"errors":[]}', b'{"errors":[{"code":"UNAUTHORIZED"}]}',
                         b'{"errors":[{"code":"MANIFEST_UNKNOWN"},{"code":"DENIED"}]}')
        ])
        errors.extend([
            urllib.error.HTTPError("https://other.invalid/manifest", 404, "missing", {},
                                   io.BytesIO(b'{"errors":[{"code":"MANIFEST_UNKNOWN"}]}')),
            urllib.error.URLError("offline"), TimeoutError("timed out"),
        ])
        for error in errors:
            with self.subTest(error=error):
                self.opener.open.side_effect = error
                with self.assertRaises(RuntimeError):
                    container.registry_manifest("v1.1.0", "read-token")

    def test_success_requires_verifiable_manifest_bytes(self):
        manifest = {"schemaVersion": 2, "mediaType": container.MANIFEST_TYPES[0], "config": {"digest": "sha256:" + "a" * 64}}
        data = json.dumps(manifest).encode()
        digest = "sha256:" + hashlib.sha256(data).hexdigest()
        for body, claimed_digest, media_type, accepted in (
            (data, digest, manifest["mediaType"], True),
            (data, "sha256:" + "f" * 64, manifest["mediaType"], False),
            (data, digest, "text/html", False),
            (b"not json", "sha256:" + hashlib.sha256(b"not json").hexdigest(), manifest["mediaType"], False),
        ):
            response = io.BytesIO(body)
            response.status, response.url = 200, self.url
            response.headers = {"Docker-Content-Digest": claimed_digest, "Content-Type": media_type}
            self.opener.open.return_value = response
            with self.subTest(accepted=accepted):
                if accepted:
                    self.assertEqual(container.registry_manifest("v1.1.0", "read-token"), (digest, manifest))
                else:
                    with self.assertRaises(RuntimeError):
                        container.registry_manifest("v1.1.0", "read-token")
            self.assertTrue(response.closed)

    def test_registry_token_requires_authenticated_success(self):
        with patch.dict(os.environ, {"GITHUB_ACTOR": "maintainer", "GH_TOKEN": "protected-token"}, clear=True):
            for body in (b"{}", b'{"token":null}', b'{"token":""}', b"not json"):
                response = io.BytesIO(body)
                response.status = 200
                response.url = "https://ghcr.io/token?service=ghcr.io&scope=repository:critx-ai/regen-ssg:pull"
                response.headers = {}
                self.opener.open.return_value = response
                with self.subTest(body=body), self.assertRaises(RuntimeError):
                    container.registry_token()


if __name__ == "__main__":
    unittest.main()
