#!/usr/bin/env python3
"""Smoke native local packages, or wrap and promote verified CI candidate archives."""

import argparse
import base64
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import re
import shlex
import shutil
import subprocess
import sys
import tarfile
import tempfile
import tomllib
import urllib.error
import urllib.request

import uuid
from check_cargo import source_inventory
from package import ROOT, TOOLCHAIN, json_file, version
from release import (authorize, candidate_commit, prepare, registry_version, release_state,
                     require_tag, verify_release_assets)

IMAGE = "ghcr.io/critx-ai/regen-ssg"
BUILDER = "docker.io/library/rust:1.98.1-bookworm@sha256:9a73a5088750b4c95158ab26629c854c3d6fc4b173cb7bc8079ad252d8ed7bfa"
TARGETS = {"amd64": "x86_64-unknown-linux-gnu", "arm64": "aarch64-unknown-linux-gnu"}
HOSTS = {"x86_64": "amd64", "aarch64": "arm64", "arm64": "arm64"}
DIGEST = re.compile(r"sha256:[0-9a-f]{64}")
MANIFEST_TYPES = (
    "application/vnd.docker.distribution.manifest.v2+json",
    "application/vnd.oci.image.manifest.v1+json",
)
INDEX_TYPES = (
    "application/vnd.docker.distribution.manifest.list.v2+json",
    "application/vnd.oci.image.index.v1+json",
)


def capture(*command):
    return subprocess.check_output(command, text=True).strip()


def run(*command):
    subprocess.run(command, check=True)


def identity():
    commit = candidate_commit(os.environ["GITHUB_SHA"])
    return {
        "commit": commit,
        "version": version(),
        "run_id": os.environ["GITHUB_RUN_ID"],
        "source": f"{os.environ.get('GITHUB_SERVER_URL', 'https://github.com')}/{os.environ['GITHUB_REPOSITORY']}",
    }


def reference(commit, arch=None):
    return f"{IMAGE}:sha-{commit}" + (f"-{arch}" if arch else "")


def local_runtime(override=None):
    runtime = override or os.environ.get("REGEN_CONTAINER_RUNTIME")
    if runtime is None:
        runtime = next((name for name in ("podman", "docker") if shutil.which(name)), None)
    if runtime not in {"podman", "docker"} or not shutil.which(runtime):
        raise RuntimeError("install Podman (preferred) or Docker; REGEN_CONTAINER_RUNTIME must be podman or docker")
    return runtime


def require_native_runtime(runtime, arch):
    if platform.system() != "Linux" or HOSTS.get(platform.machine()) != arch:
        raise RuntimeError("container must be built and smoked on its native Linux host")
    if os.getuid() == 0:
        raise RuntimeError("container smoke must run as an unprivileged host user")
    if runtime == "podman":
        host = json.loads(capture(runtime, "info", "--format", "json"))["host"]
        server_os, server_arch = host["os"], host["arch"]
        if host["security"]["rootless"] is not True or host["serviceIsRemote"] is not False:
            raise RuntimeError("local Podman must be rootless and use local storage, not a remote service")
    else:
        host = json.loads(capture(runtime, "info", "--format", "{{json .}}"))
        server_os, server_arch = host["OSType"], host["Architecture"]
    if server_os != "linux" or HOSTS.get(server_arch, server_arch) != arch:
        raise RuntimeError(f"{runtime} server differs from the native Linux host architecture")


def inspect_image(image, arch, expected, runtime="docker"):
    info = json.loads(capture(runtime, "image", "inspect", image))[0]
    # Podman exposes User/Labels at the top level and uses a bare SHA256 image ID;
    # Docker exposes them under Config and prefixes its ID with sha256:.
    config = info if runtime == "podman" else info["Config"]
    labels = config.get("Labels") or {}
    image_id = info["Id"]
    valid_id = re.fullmatch(r"[0-9a-f]{64}", image_id) if runtime == "podman" else DIGEST.fullmatch(image_id)
    if not valid_id:
        raise RuntimeError(f"{runtime} did not return an immutable image ID")
    if info["Os"] != "linux" or info["Architecture"] != arch:
        raise RuntimeError("image platform differs from the native target")
    if config["User"] != "65532:65532":
        raise RuntimeError("image must default to the unprivileged runtime user")
    for name in ("source", "revision", "version"):
        value = expected["commit" if name == "revision" else name]
        if labels.get(f"org.opencontainers.image.{name}") != value:
            raise RuntimeError(f"image {name} label differs from the package identity")
    return info


def summary(text):
    print(text)
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as output:
            output.write(text + "\n")


def runtime_command(runtime):
    command = [runtime, "run", "--rm", "--pull", "never", "--network", "none", "--read-only"]
    if runtime == "podman":
        command += ["--read-only-tmpfs=false"]
    return command


def launch(image, arguments, runtime="docker"):
    # smoke.py chooses writable temporary roots (including its caller's TMPDIR).
    # Bind the exact requested site, not a guessed /tmp or repository directory.
    command = runtime_command(runtime)
    if arguments[:2] == ["build", "--site"] and len(arguments) == 3:
        site = Path(arguments[2]).resolve(strict=True)
        if os.getuid() == 0:
            raise RuntimeError("container smoke must run as an unprivileged host user")
        if runtime == "podman":
            command += ["--userns", "keep-id"]
        command += ["--user", f"{os.getuid()}:{os.getgid()}",
                    "--volume", f"{site}:/site:rw,Z"]
        arguments = ["build", "--site", "/site"]
    elif arguments != ["--version"]:
        raise RuntimeError("container smoke launcher only supports --version and build --site PATH")
    # exec preserves the runtime/application exit status. Version uses Dockerfile USER.
    os.execvp(command[0], [*command, image, *arguments])


def build_and_smoke(directory, arch, expected, image, runtime):
    base = f"regen-{expected['version']}-{TARGETS[arch]}"
    with tempfile.TemporaryDirectory(prefix="regen-container-") as temporary:
        context = Path(temporary)
        unpacked = context / "unpacked"
        with tarfile.open(directory / f"{base}.tar.gz", "r:gz") as archive:
            for member in archive.getmembers():
                path = PurePosixPath(member.name)
                if (not member.isfile() or path.is_absolute() or ".." in path.parts
                        or not path.parts or path.parts[0] != base):
                    raise RuntimeError("package archive has an unsafe or unexpected payload path")
            archive.extractall(unpacked, filter="data")
        payload = context / "release" / arch
        payload.parent.mkdir()
        shutil.move(unpacked / base, payload)
        for required in ("regen", "LICENSE", "THIRD-PARTY-NOTICES.txt", "licenses/inventory.json"):
            if not (payload / required).is_file():
                raise RuntimeError(f"package archive is missing {required}")
        shutil.copyfile(ROOT / "Dockerfile", context / "Dockerfile")
        shutil.copyfile(ROOT / ".dockerignore", context / ".dockerignore")
        run(runtime, "build", "--force-rm", "--platform", f"linux/{arch}",
            "--build-arg", f"TARGETARCH={arch}",
            "--build-arg", f"OCI_SOURCE={expected['source']}",
            "--build-arg", f"OCI_REVISION={expected['commit']}",
            "--build-arg", f"OCI_VERSION={expected['version']}",
            "--tag", image, str(context))
        image_id = inspect_image(image, arch, expected, runtime)["Id"]
        # Check every archived byte in the image, including the executable and all
        # notices, as Dockerfile USER with no network and a read-only runtime root.
        checksums = []
        for path in sorted(payload.rglob("*")):
            if path.is_file():
                with path.open("rb") as source:
                    digest = hashlib.file_digest(source, "sha256").hexdigest()
                name = path.relative_to(payload).as_posix()
                escaped = name.replace("\\", "\\\\").replace("\n", "\\n")
                checksums.append(("\\" if escaped != name else "") + digest + "  " + escaped + "\n")
        subprocess.run([*runtime_command(runtime), "--interactive", "--workdir", "/opt/regen",
                        "--entrypoint", "sha256sum", image_id, "--check", "--strict", "--quiet"],
                       input="".join(checksums), text=True, check=True)
        launcher = context / "regen-container"
        launcher.write_text("#!/bin/sh\nexec " + shlex.join([
            sys.executable, str(Path(__file__).resolve()), "launch",
            "--runtime", runtime, "--image", image_id, "--",
        ]) + ' "$@"\n', encoding="utf-8")
        launcher.chmod(0o755)
        run(sys.executable, str(ROOT / "scripts" / "smoke.py"),
            "--binary", str(launcher), "--site", str(ROOT / "examples" / "minimal"))
        if inspect_image(image, arch, expected, runtime)["Id"] != image_id:
            raise RuntimeError("image tag changed during smoke")
        return image_id


def local(directory, target, runtime=None):
    """Check a native package without candidate authorization or retained CI receipts."""
    runtime = local_runtime(runtime)
    arch = next(arch for arch, native_target in TARGETS.items() if native_target == target)
    require_native_runtime(runtime, arch)
    expected = {"version": version(), "commit": "local-unpublished", "source": "https://github.com/CritX-ai/ReGen"}
    # A private per-invocation tag cannot replace a candidate or a user's named image.
    image = f"localhost/regen-local-check:{uuid.uuid4().hex}"
    try:
        image_id = build_and_smoke(directory.resolve(strict=True), arch, expected, image, runtime)
    finally:
        # Remove only our tag; never force-remove shared IDs or prune parent images.
        if capture(runtime, "image", "ls", "--filter", f"reference={image}", "--format", "{{.ID}}"):
            run(runtime, "image", "rm", "--no-prune", image)
    print(f"Local {runtime} native image {image_id} passed archive-byte and offline smoke checks; image removed. No publication.")


def check_source(target, cache, runtime=None):
    """Build this source against Bookworm, not the workstation's newer libc."""
    runtime = local_runtime(runtime)
    arch = next(arch for arch, native_target in TARGETS.items() if native_target == target)
    require_native_runtime(runtime, arch)
    cache.mkdir(parents=True, exist_ok=True)
    cache = cache.resolve(strict=True)
    (cache / "tmp").mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="regen-container-source-") as temporary:
        work = Path(temporary)
        source = work / "source"
        source.mkdir()
        manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
        for name, contents in source_inventory(manifest).items():
            destination = source / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(contents)
        # Vendor only locked, locally fetched sources. No host Cargo credentials
        # or unrelated working-tree files enter the compiler container.
        vendor = work / "vendor"
        capture("cargo", f"+{TOOLCHAIN}", "vendor", "--locked", "--offline",
                "--versioned-dirs", "--manifest-path", str(source / "Cargo.toml"), str(vendor))
        command = [runtime, "run", "--rm", "--pull", "missing", "--network", "none", "--read-only"]
        if runtime == "podman":
            command += ["--read-only-tmpfs=false", "--userns", "keep-id"]
        command += [
            "--user", f"{os.getuid()}:{os.getgid()}", "--workdir", "/source",
            "--volume", f"{source}:/source:ro,Z", "--volume", f"{vendor}:/vendor:ro,Z",
            "--volume", f"{cache}:/build:rw,Z",
            "--env", "CARGO_HOME=/build/cargo-home", "--env", "CARGO_TARGET_DIR=/build",
            "--env", "TMPDIR=/build/tmp", BUILDER,
            "cargo", f"+{TOOLCHAIN}", "build", "--release", "--frozen", "--bin", "regen",
            "--target", target, "--config", 'source.crates-io.replace-with="vendored-sources"',
            "--config", 'source.vendored-sources.directory="/vendor"',
        ]
        run(*command)
        packages = work / "native"
        run(sys.executable, str(ROOT / "scripts/package.py"), "--target", target,
            "--binary", str(cache / target / "release/regen"), "--output", str(packages))
        local(packages, target, runtime)


def build(directory, arch, output):
    expected = identity()
    candidate = prepare(directory.resolve(strict=True), expected["version"], expected["commit"], verify=True)
    if candidate["run_id"] != expected["run_id"]:
        raise RuntimeError("release bundle belongs to a different workflow run")
    require_native_runtime("docker", arch)
    output.mkdir(parents=True, exist_ok=True)
    archive_output = output / f"{arch}.tar"
    receipt_output = output / f"{arch}.json"
    if archive_output.exists() or receipt_output.exists():
        raise RuntimeError("refusing to overwrite retained container artifacts")
    image = reference(expected["commit"], arch)
    image_id = build_and_smoke(directory, arch, expected, image, "docker")
    # Save the tagged image only after the same immutable image ID passed smoke.
    if inspect_image(image, arch, expected)["Id"] != image_id:
        raise RuntimeError("image tag changed before retention")
    run("docker", "image", "save", "--output", str(archive_output), image)
    json_file(receipt_output, {**expected, "arch": arch, "image_id": image_id})
    summary(f"## Native container: linux/{arch}\n\n"
            f"Candidate `{expected['commit']}` (run `{expected['run_id']}`), package `{expected['version']}`.\n\n"
            f"Smoke passed for image ID `{image_id}`. Retained `{arch}.tar`; no registry write.\n")


def push_index(tag, references):
    run("docker", "manifest", "create", tag, *references)
    digest = capture("docker", "manifest", "push", "--purge", tag)
    if not DIGEST.fullmatch(digest):
        raise RuntimeError(f"Docker did not return a manifest digest for {tag}: {digest}")
    return digest


class RegistryRedirects(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):
        # Never forward registry credentials to a redirect target.
        return None


def registry_response(url, authorization, missing=False):
    request = urllib.request.Request(url, headers={
        "Authorization": authorization, "Accept": ", ".join((*MANIFEST_TYPES, *INDEX_TYPES)),
        "Cache-Control": "no-cache", "User-Agent": "ReGen-release/1.0",
    })
    try:
        with urllib.request.build_opener(RegistryRedirects()).open(request, timeout=30) as response:
            if response.status != 200 or response.url != url:
                raise RuntimeError("unexpected container registry response; publication state is unknown")
            return response.headers, response.read()
    except urllib.error.HTTPError as error:
        with error:
            if missing and error.code == 404 and error.url == url:
                try:
                    body = json.loads(error.read())
                except (ValueError, UnicodeError) as malformed:
                    raise RuntimeError("malformed container registry absence response") from malformed
                errors = body.get("errors") if isinstance(body, dict) else None
                if (isinstance(errors, list) and errors
                        and all(isinstance(item, dict) and item.get("code") in {"MANIFEST_UNKNOWN", "NAME_UNKNOWN"}
                                for item in errors)):
                    return None
            raise RuntimeError(f"container registry returned HTTP {error.code}; publication state is unknown") from error
    except (urllib.error.URLError, TimeoutError) as error:
        raise RuntimeError("container registry lookup failed; publication state is unknown") from error


def registry_token():
    actor, token = os.environ.get("GITHUB_ACTOR"), os.environ.get("GH_TOKEN")
    if not actor or not token:
        raise RuntimeError("authenticated container registry preflight requires GITHUB_ACTOR and GH_TOKEN")
    credentials = base64.b64encode(f"{actor}:{token}".encode()).decode()
    _, body = registry_response(
        "https://ghcr.io/token?service=ghcr.io&scope=repository:critx-ai/regen-ssg:pull",
        f"Basic {credentials}")
    try:
        value = json.loads(body)
    except (ValueError, UnicodeError) as error:
        raise RuntimeError("malformed container registry token response") from error
    if not isinstance(value, dict) or not isinstance(value.get("token"), str) or not value["token"].strip():
        raise RuntimeError("container registry did not issue a read token")
    return value["token"]


def registry_manifest(tag, token):
    response = registry_response(f"https://ghcr.io/v2/critx-ai/regen-ssg/manifests/{tag}", f"Bearer {token}", missing=True)
    if response is None:
        return None
    headers, body = response
    try:
        manifest = json.loads(body)
    except (ValueError, UnicodeError) as error:
        raise RuntimeError("malformed container registry manifest") from error
    digest = headers.get("Docker-Content-Digest", "")
    if (not DIGEST.fullmatch(digest) or digest != f"sha256:{hashlib.sha256(body).hexdigest()}"
            or not isinstance(manifest, dict) or manifest.get("schemaVersion") != 2
            or manifest.get("mediaType") not in (*MANIFEST_TYPES, *INDEX_TYPES)
            or headers.get("Content-Type", "").split(";", 1)[0] != manifest["mediaType"]):
        raise RuntimeError("unverifiable container registry manifest; publication state is unknown")
    return digest, manifest


def require_registry_state(tags, token, promoted):
    for tag in tags:
        manifest = registry_manifest(tag, token)
        if tag in promoted:
            if manifest is None or manifest[0] != promoted[tag]:
                raise RuntimeError("promoted platform changed or disappeared; inspect registry state, do not retry")
        elif manifest is not None:
            raise RuntimeError("container version or platform tag already exists; inspect partial/existing state, never overwrite")


def published_candidate(repo, candidate):
    published, ref = release_state(repo, candidate["tag"])
    require_tag(ref, candidate)
    if (not published or published.get("draft") is not False or published.get("prerelease") is not False
            or published.get("tag_name") != candidate["tag"]
            or not isinstance(published.get("published_at"), str) or not published["published_at"]):
        raise RuntimeError("containers require the candidate's already-published full GitHub release")
    crate = registry_version(candidate["version"])
    if crate is None or crate["cksum"] != candidate["crate_sha256"] or crate["yanked"]:
        raise RuntimeError("containers require the matching, unyanked immutable crate")
    return published


def publish(directory, candidate_directory, requested_run):
    if os.environ.get("GITHUB_REPOSITORY") != "CritX-ai/ReGen":
        raise RuntimeError("container publication is only authorized for CritX-ai/ReGen")
    candidate_directory = candidate_directory.resolve(strict=True)
    candidate = prepare(candidate_directory, version(), candidate_commit(os.environ.get("GITHUB_SHA")), verify=True)
    repo = authorize(candidate, "publish-container", requested_run)
    published = published_candidate(repo, candidate)
    verify_release_assets(repo, published, candidate_directory)
    expected = {key: candidate[key] for key in ("commit", "version", "run_id")}
    expected["source"] = f"https://github.com/{repo}"
    tags = [candidate["tag"], *[f"{candidate['tag']}-{arch}" for arch in TARGETS]]
    token = registry_token()
    require_registry_state(tags, token, {})
    images = {}
    for arch in TARGETS:
        receipt = json.loads((directory / f"{arch}.json").read_text(encoding="utf-8"))
        image_id = receipt.get("image_id") if isinstance(receipt, dict) else None
        if not isinstance(image_id, str) or not DIGEST.fullmatch(image_id):
            raise RuntimeError("retained image ID is not a SHA256 digest")
        if receipt != {**expected, "arch": arch, "image_id": image_id}:
            raise RuntimeError("retained container does not belong to this candidate run and platform")
        run("docker", "image", "load", "--input", str(directory / f"{arch}.tar"))
        if inspect_image(reference(expected["commit"], arch), arch, expected)["Id"] != image_id:
            raise RuntimeError("loaded image differs from the smoke-tested image")
        images[arch] = image_id
    # Both architectures are loaded and checked before any registry write. The
    # source-SHA tags are archive handles only; promotion pins the retained IDs.
    references = []
    rows = []
    manifests = {}
    for arch, image_id in images.items():
        tag = f"{candidate['tag']}-{arch}"
        image = f"{IMAGE}:{tag}"
        run("docker", "image", "tag", image_id, image)
        if inspect_image(image, arch, expected)["Id"] != image_id:
            raise RuntimeError("version tag differs from the smoke-tested image")
        authorize(candidate, "publish-container", requested_run)
        published_candidate(repo, candidate)
        require_registry_state(tags, token, manifests)
        run("docker", "image", "push", image)
        result = registry_manifest(tag, token)
        if result is None:
            raise RuntimeError("pushed platform is not visible; inspect registry state, do not retry")
        digest, manifest = result
        if manifest.get("mediaType") not in MANIFEST_TYPES or manifest.get("config", {}).get("digest") != image_id:
            raise RuntimeError("pushed platform does not contain the tested image; inspect registry state, do not retry")
        manifests[tag] = digest
        references.append(f"{IMAGE}@{digest}")
        rows.append(f"| linux/{arch} | `{image}` | `{IMAGE}@{digest}` |")
    authorize(candidate, "publish-container", requested_run)
    published_candidate(repo, candidate)
    require_registry_state(tags, token, manifests)
    digest = push_index(f"{IMAGE}:{candidate['tag']}", references)
    result = registry_manifest(candidate["tag"], token)
    if result is None or result[0] != digest:
        raise RuntimeError("multi-platform publication was not confirmed; inspect registry state, do not retry")
    manifest = result[1]
    platforms = manifest.get("manifests", [])
    if (manifest.get("mediaType") not in INDEX_TYPES or len(platforms) != len(TARGETS)
            or {(item.get("platform", {}).get("os"), item.get("platform", {}).get("architecture"), item.get("digest"))
                for item in platforms}
            != {("linux", arch, manifests[f"{candidate['tag']}-{arch}"]) for arch in TARGETS}):
        raise RuntimeError("published index differs from the retained platforms; inspect registry state, do not retry")
    require_registry_state(tags[1:], token, manifests)
    rows.append(f"| Multi-platform release | `{IMAGE}:{candidate['tag']}` | `{IMAGE}@{digest}` |")
    summary("## Published verified release containers\n\n| Platform | Tag | Immutable pin |\n"
            "| --- | --- | --- |\n" + "\n".join(rows) + "\n\n"
            "Tags are mutable pointers; use the multi-platform digest to pin tested bytes. "
            "No rolling main, source-SHA or latest tags were published. Package visibility is an owner setting.\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    checker = commands.add_parser("local", help="Build/smoke a native package; never retain or publish a CI candidate")
    checker.add_argument("--directory", required=True, type=Path)
    checker.add_argument("--target", required=True, choices=TARGETS.values())
    checker.add_argument("--runtime", choices=("podman", "docker"), help="Override REGEN_CONTAINER_RUNTIME and Podman-first detection")
    source_checker = commands.add_parser("check", help="Compile this source in the pinned builder and smoke its runtime image")
    source_checker.add_argument("--target", required=True, choices=TARGETS.values())
    source_checker.add_argument("--cache", type=Path, default=ROOT / "target/container-check")
    source_checker.add_argument("--runtime", choices=("podman", "docker"))
    builder = commands.add_parser("build")
    builder.add_argument("--directory", required=True, type=Path)
    builder.add_argument("--arch", required=True, choices=TARGETS)
    builder.add_argument("--output", required=True, type=Path)
    publisher = commands.add_parser("publish")
    publisher.add_argument("--directory", required=True, type=Path)
    publisher.add_argument("--candidate-directory", required=True, type=Path)
    publisher.add_argument("--candidate-run", required=True)
    launcher = commands.add_parser("launch")
    launcher.add_argument("--image", required=True)
    launcher.add_argument("--runtime", choices=("podman", "docker"), default="docker")
    launcher.add_argument("arguments", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    if args.command == "local":
        local(args.directory, args.target, args.runtime)
    elif args.command == "check":
        check_source(args.target, args.cache, args.runtime)
    elif args.command == "build":
        build(args.directory, args.arch, args.output)
    elif args.command == "publish":
        publish(args.directory, args.candidate_directory, args.candidate_run)
    else:
        arguments = args.arguments[1:] if args.arguments[:1] == ["--"] else args.arguments
        launch(args.image, arguments, args.runtime)


if __name__ == "__main__":
    main()
