#!/usr/bin/env python3
"""Prepare a local candidate; remote release operations require the protected workflow."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
import tempfile
import urllib.error
import urllib.request
import zipfile

from check_cargo import verify_source_bundle
from package import ROOT, TARGETS, TOOLCHAIN, json_file, version


def expected_files(current):
    names = []
    for target in TARGETS:
        base = f"regen-{current}-{target}"
        names.extend([base + (".zip" if "windows" in target else ".tar.gz"), base + "-licenses.tar.gz", base + "-dependencies.json"])
    return sorted(names)


def archive_files(path):
    if path.suffix == ".zip":
        with zipfile.ZipFile(path) as archive:
            if len(archive.namelist()) != len(set(archive.namelist())):
                raise RuntimeError(f"duplicate archive entries: {path.name}")
            return {name: archive.read(name) for name in archive.namelist() if not name.endswith("/")}
    with tarfile.open(path, "r:gz") as archive:
        files = {}
        for member in archive:
            if not member.isfile():
                raise RuntimeError(f"unexpected non-file archive entry: {path.name}")
            if member.name in files:
                raise RuntimeError(f"duplicate archive entries: {path.name}")
            files[member.name] = archive.extractfile(member).read()
        return files


def validate_packages(directory, current):
    inventories = []
    for target in TARGETS:
        base = f"regen-{current}-{target}"
        inventory = json.loads((directory / f"{base}-dependencies.json").read_text(encoding="utf-8"))
        if inventory.get("format") != 1 or inventory.get("package") != "regen-ssg" or inventory.get("version") != current or inventory.get("target") != target:
            raise RuntimeError(f"incorrect dependency inventory: {target}")
        if not inventory.get("packages") or inventory.get("rust", {}).get("version") != TOOLCHAIN:
            raise RuntimeError(f"incomplete dependency inventory: {target}")
        package = archive_files(directory / (base + (".zip" if "windows" in target else ".tar.gz")))
        licenses = archive_files(directory / f"{base}-licenses.tar.gz")
        binary = "regen.exe" if "windows" in target else "regen"
        if not package.get(f"{base}/{binary}") or not package.get(f"{base}/LICENSE") or not package.get(f"{base}/THIRD-PARTY-NOTICES.txt"):
            raise RuntimeError(f"package lacks executable or notices: {target}")
        for archive, prefix in ((package, base), (licenses, f"{base}-licenses")):
            if json.loads(archive[f"{prefix}/licenses/inventory.json"]) != inventory:
                raise RuntimeError(f"archive inventory differs from sidecar: {target}")
            for dependency in [*inventory["packages"], inventory["rust"]]:
                if not dependency.get("notices"):
                    raise RuntimeError(f"dependency has no notices: {target}")
                for notice in dependency["notices"]:
                    data = archive.get(f"{prefix}/licenses/{notice}")
                    if not data or not data.strip():
                        raise RuntimeError(f"missing dependency notice in archive: {target}: {notice}")
                    if package.get(f"{base}/licenses/{notice}") != data:
                        raise RuntimeError(f"notice differs between archives: {target}: {notice}")
        inventories.append(inventory)
    return inventories


def checksums(directory, names):
    return "".join(f"{hashlib.sha256((directory / name).read_bytes()).hexdigest()}  {name}\n" for name in sorted(names))


def candidate_commit(requested):
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    if not re.fullmatch(r"[0-9a-f]{40}", commit) or (requested and requested != commit):
        raise RuntimeError("checkout differs from the explicitly tested commit")
    return commit


def prepare(directory, current, commit, verify=False):
    names = expected_files(current) + [f"regen-ssg-{current}.crate", "SOURCE.json"]
    generated = {"DEPENDENCIES.json", "CANDIDATE.json", "SHA256SUMS", "release-notes.txt"}
    present = {path.name for path in directory.iterdir() if path.is_file()}
    required = set(names) | (generated if verify else set())
    if present != required or any(not path.is_file() or path.is_symlink() for path in directory.iterdir()):
        raise RuntimeError(f"release asset set differs: missing={sorted(required - present)}, extra={sorted(present - required)}")
    inventories = validate_packages(directory, current)
    source = verify_source_bundle(directory, commit)
    dependencies = {"format": 1, "package": "regen-ssg", "version": current, "targets": inventories}
    if not verify:
        candidate = {
            "format": 1, "package": "regen-ssg", "version": current, "tag": f"v{current}",
            "commit": commit, "run_id": os.environ.get("GITHUB_RUN_ID", "local"),
            "run_attempt": os.environ.get("GITHUB_RUN_ATTEMPT", "local"),
            "crate_sha256": source["crate_sha256"],
        }
        json_file(directory / "DEPENDENCIES.json", dependencies)
        json_file(directory / "CANDIDATE.json", candidate)
        notes = [
            f"ReGen {current}", "",
            f"Tested source commit: {commit}",
            f"Candidate workflow run: {candidate['run_id']} (attempt {candidate['run_attempt']})", "",
            "Each binary archive contains the executable, example, documentation, Cargo.lock, and original third-party license notices.",
            "Separate license archives and dependency inventories are provided for every target.",
            "The source crate and SOURCE.json retain the inspected Cargo source inventory.",
            "Verify downloads with SHA256SUMS; hashes detect changed bytes but are not signatures.",
            "Binaries are not code-signed or notarized. Review platform loader requirements in docs/releasing.md.",
            "A successful runner matrix does not establish bit-identical binaries across compilers or operating systems.", "",
            "Targets:", *[f"- {target}" for target in TARGETS], "",
        ]
        gaps = sorted({f"{package['name']} {package['version']}" for inventory in inventories for package in inventory["packages"] if package.get("notice_source_gap")})
        if gaps:
            notes.extend(["Upstream notice-source gaps require maintainer review before publication.",
                          "Archives retain exact licensing declarations and separately labeled standard terms, not fabricated copyright notices.",
                          *[f"- {identity}" for identity in gaps], ""])
        (directory / "release-notes.txt").write_text("\n".join(notes), encoding="utf-8", newline="\n")
        (directory / "SHA256SUMS").write_text(checksums(directory, names + sorted(generated - {"SHA256SUMS"})), encoding="utf-8", newline="\n")
        print(f"Prepared {current} at {commit}: six native targets, inspected crate, inventories and SHA256SUMS; no remote writes")
        if os.environ.get("GITHUB_OUTPUT"):
            with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
                output.write(f"version={current}\n")
        return candidate
    candidate = json.loads((directory / "CANDIDATE.json").read_text(encoding="utf-8"))
    expected = {
        "format": 1, "package": "regen-ssg", "version": current, "tag": f"v{current}",
        "commit": commit, "run_id": candidate.get("run_id"), "crate_sha256": source["crate_sha256"],
        "run_attempt": candidate.get("run_attempt"),
    }
    if candidate != expected or not isinstance(candidate["run_id"], str) or not isinstance(candidate["run_attempt"], str):
        raise RuntimeError("release candidate version, tag, tested SHA, or crate has drifted")
    if json.loads((directory / "DEPENDENCIES.json").read_text(encoding="utf-8")) != dependencies:
        raise RuntimeError("aggregate inventory differs from target inventories")
    if (directory / "SHA256SUMS").read_text(encoding="utf-8") != checksums(directory, names + sorted(generated - {"SHA256SUMS"})):
        raise RuntimeError("release checksum verification failed")
    return candidate


def github(endpoint, payload=None, paginate=False):
    """GitHub errors, including 404/auth failures, are never absence signals."""
    command = ["gh", "api", "--hostname", "github.com", "--method", "POST" if payload is not None else "GET",
               "-H", "Accept: application/vnd.github+json", "-H", "X-GitHub-Api-Version: 2026-03-10", endpoint]
    if paginate:
        command.extend(["--paginate", "--slurp"])
    if payload is not None:
        command.extend(["--input", "-"])
    result = subprocess.run(command, input=json.dumps(payload) if payload is not None else None,
                            text=True, stdout=subprocess.PIPE, check=True)
    return json.loads(result.stdout)


def public_index(path, missing=False):
    url = f"https://index.crates.io/{path}"
    request = urllib.request.Request(url, headers={"User-Agent": "ReGen-release/1.0 (https://github.com/CritX-ai/ReGen)", "Cache-Control": "no-cache"})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            if response.status != 200 or response.url != url:
                raise RuntimeError("unexpected public registry index response")
            return response.read().decode("utf-8")
    except urllib.error.HTTPError as error:
        with error:
            # Only the unauthenticated sparse package index has documented 404 absence.
            if missing and error.code == 404 and error.url == url:
                return None
            raise RuntimeError(f"registry index lookup failed with HTTP {error.code}; release state is unknown") from error


def registry_version(current):
    config = json.loads(public_index("config.json"))
    if config.get("api") != "https://crates.io" or config.get("auth-required", False):
        raise RuntimeError("unexpected crates.io registry configuration")
    text = public_index("re/ge/regen-ssg", missing=True)
    if text is None:
        return None
    rows = [json.loads(line) for line in text.splitlines()]
    if not rows or any(row.get("name") != "regen-ssg" or not isinstance(row.get("vers"), str)
                       or not re.fullmatch(r"[0-9a-f]{64}", row.get("cksum", ""))
                       or not isinstance(row.get("yanked"), bool) for row in rows):
        raise RuntimeError("malformed crates.io index; release state is unknown")
    if len({row["vers"] for row in rows}) != len(rows):
        raise RuntimeError("duplicate registry versions; release state is unknown")
    return next((row for row in rows if row["vers"] == current), None)


def release_state(repo, tag):
    pages = github(f"repos/{repo}/releases?per_page=100", paginate=True)
    if not isinstance(pages, list) or any(not isinstance(page, list) for page in pages):
        raise RuntimeError("malformed release listing; release state is unknown")
    releases = [release for page in pages for release in page if release["tag_name"] == tag]
    refs = github(f"repos/{repo}/git/matching-refs/tags/{tag}")
    if not isinstance(refs, list):
        raise RuntimeError("malformed tag listing; release state is unknown")
    refs = [ref for ref in refs if ref["ref"] == f"refs/tags/{tag}"]
    if len(releases) > 1 or len(refs) > 1:
        raise RuntimeError("ambiguous release/tag state")
    release = releases[0] if releases else None
    if release and not isinstance(release.get("draft"), bool):
        raise RuntimeError("malformed release state")
    return release, refs[0] if refs else None


def require_activation(commit, activation):
    """Only a one-commit, byte-identical move can activate the reviewed workflow."""
    if not re.fullmatch(r"[0-9a-f]{40}", activation) or not re.fullmatch(r"[0-9a-f]{40}", commit):
        raise RuntimeError("candidate and activation must be full Git commit SHAs")
    parents = subprocess.check_output(["git", "rev-list", "--parents", "-n", "1", activation], cwd=ROOT, text=True).split()
    if parents != [activation, commit]:
        raise RuntimeError("release activation must have exactly the tested candidate as its sole parent")
    dormant, active = ".github/release.yml", ".github/workflows/release.yml"
    changes = subprocess.check_output([
        "git", "diff-tree", "--no-commit-id", "--name-status", "--no-renames", "-r", commit, activation,
    ], cwd=ROOT, text=True).splitlines()
    if sorted(changes) != [f"A\t{active}", f"D\t{dormant}"]:
        raise RuntimeError("activation may only move .github/release.yml to .github/workflows/release.yml; source and README must not change")
    original = subprocess.check_output(["git", "ls-tree", commit, "--", dormant], cwd=ROOT, text=True).split("\t")[0]
    activated = subprocess.check_output(["git", "ls-tree", activation, "--", active], cwd=ROOT, text=True).split("\t")[0]
    if not original.startswith("100644 blob ") or activated != original:
        raise RuntimeError("activated release workflow must retain the original regular-file mode and identical bytes")


def authorize(candidate, operation, requested_version, requested_run):
    current = candidate["version"]
    repo = os.environ.get("GH_REPO", "")
    activation = os.environ.get("GITHUB_SHA", "")
    if (os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("GITHUB_SERVER_URL") != "https://github.com"
            or os.environ.get("GITHUB_REF") != "refs/heads/main" or repo != os.environ.get("GITHUB_REPOSITORY")
            or os.environ.get("GITHUB_WORKFLOW_REF") != f"{repo}/.github/workflows/release.yml@refs/heads/main"
            or os.environ.get("GITHUB_WORKFLOW_SHA") != activation):
        raise RuntimeError("remote actions require the activated release workflow on main")
    if os.environ.get("REGEN_RELEASE_VERSION") != current:
        raise RuntimeError("REGEN_RELEASE_VERSION must explicitly authorize this exact package version")
    if (os.environ.get("GITHUB_EVENT_NAME") != "workflow_dispatch" or requested_version != current
            or operation not in {"authorize", "draft", "publish-crate", "publish-github"}):
        raise RuntimeError("remote actions require a separate manual operation and the exact release_version input")
    if (not isinstance(requested_run, str) or not re.fullmatch(r"[1-9][0-9]*", requested_run)
            or candidate["run_id"] != requested_run or requested_run == os.environ.get("GITHUB_RUN_ID")):
        raise RuntimeError("candidate_run must name this original retained candidate from a different completed run")
    candidate_commit(candidate["commit"])
    require_activation(candidate["commit"], activation)
    repository = github(f"repos/{repo}")
    if repository.get("permissions", {}).get("push") is not True:
        raise RuntimeError("GitHub credential must have push access so existing drafts cannot be hidden")
    run = github(f"repos/{repo}/actions/runs/{requested_run}")
    # Reruns preserve the run ID but can replace expired artifacts. Only a
    # separately reviewed, first-attempt run can establish a release candidate.
    if candidate.get("run_attempt") != "1" or run.get("run_attempt") != 1:
        raise RuntimeError("rerun artifacts cannot replace the original candidate; review a new first-attempt build run")
    if (run.get("id") != int(requested_run) or run.get("status") != "completed" or run.get("conclusion") != "success"
            or run.get("head_sha") != candidate["commit"] or run.get("head_branch") != "main"
            or run.get("head_repository", {}).get("full_name") != repo
            or run.get("path") != ".github/workflows/build.yml" or run.get("event") not in {"push", "workflow_dispatch"}):
        raise RuntimeError("original candidate run did not pass the complete main verification workflow")
    environment = github(f"repos/{repo}/environments/release")
    reviewers = [rule for rule in environment.get("protection_rules", []) if rule.get("type") == "required_reviewers"]
    if not any(rule.get("reviewers") and rule.get("prevent_self_review") is True for rule in reviewers):
        raise RuntimeError("release environment needs required reviewers with self-review prevented")
    if environment.get("deployment_branch_policy") != {"protected_branches": False, "custom_branch_policies": True}:
        raise RuntimeError("release environment must select only the main branch")
    branches = github(f"repos/{repo}/environments/release/deployment-branch-policies?per_page=100")
    policies = branches.get("branch_policies", [])
    if branches.get("total_count") != 1 or len(policies) != 1 or policies[0].get("name") != "main" or policies[0].get("type") != "branch":
        raise RuntimeError("release environment must have exactly one deployment rule: branch main")
    require_main(repo, activation)
    return repo, activation


def require_tag(ref, candidate):
    if not ref or ref.get("object", {}).get("type") != "commit" or ref["object"].get("sha") != candidate["commit"]:
        raise RuntimeError("release tag is absent, annotated, or does not point to the tested candidate SHA")


def require_main(repo, commit):
    ref = github(f"repos/{repo}/git/ref/heads/main")
    if ref.get("object", {}).get("type") != "commit" or ref["object"].get("sha") != commit:
        raise RuntimeError("main no longer points to the reviewed activation commit; no release write is authorized")


def verify_draft(repo, release, directory):
    if not release or release["draft"] is not True or release.get("prerelease") is not False:
        raise RuntimeError("an existing unpublished full-release draft is required; published releases are never replaced")
    if release.get("body") != (directory / "release-notes.txt").read_text(encoding="utf-8"):
        raise RuntimeError("draft body does not match the original candidate")
    pages = github(f"repos/{repo}/releases/{release['id']}/assets?per_page=100", paginate=True)
    assets = [asset for page in pages for asset in page]
    expected = {path.name: path for path in directory.iterdir()}
    if len(assets) != len(expected) or {asset["name"] for asset in assets} != set(expected):
        raise RuntimeError("draft assets are incomplete or unexpected; repair requires separate maintainer review")
    for asset in assets:
        path = expected[asset["name"]]
        digest = f"sha256:{hashlib.sha256(path.read_bytes()).hexdigest()}"
        if asset.get("state") != "uploaded" or asset.get("size") != path.stat().st_size or asset.get("digest") != digest:
            raise RuntimeError(f"draft asset differs from original candidate or has no verifiable digest: {path.name}")


def cargo_publish(candidate, dry_run):
    upload_env = {key: value for key, value in os.environ.items() if key not in {"GH_TOKEN", "GITHUB_TOKEN"}}
    verification_env = {key: value for key, value in upload_env.items()
                        if not re.fullmatch(r"CARGO_(REGISTRY|REGISTRIES_.+)_TOKEN", key)}
    with tempfile.TemporaryDirectory(prefix="regen-publish-") as temporary:
        crate = Path(temporary) / "package" / f"regen-ssg-{candidate['version']}.crate"
        command = ["cargo", f"+{TOOLCHAIN}", "publish", "--registry", "crates-io", "--locked", "--target-dir", temporary]
        if dry_run:
            command.append("--dry-run")
        else:
            if not upload_env.get("CARGO_REGISTRY_TOKEN"):
                raise RuntimeError("the protected release environment must supply CRATES_IO_TOKEN for this step")
            subprocess.run(["cargo", f"+{TOOLCHAIN}", "package", "--locked", "--no-verify",
                            "--target-dir", temporary], cwd=ROOT, env=verification_env, check=True)
            if hashlib.sha256(crate.read_bytes()).hexdigest() != candidate["crate_sha256"]:
                raise RuntimeError("repackaged source differs from the original candidate; nothing uploaded")
            # The credential-free dry-run already compiled this exact source.
            # Do not run dependency build scripts with the publication token.
            command.append("--no-verify")
        try:
            subprocess.run(command, cwd=ROOT, env=verification_env if dry_run else upload_env, check=True)
        except subprocess.CalledProcessError as error:
            if dry_run:
                raise
            raise RuntimeError("Cargo publication did not finish cleanly; upload may have succeeded. Inspect crates.io; never automatically retry.") from error
        # Pinned Cargo publishes from tmp-crate, not cargo package's final path.
        published = crate.parent / "tmp-crate" / crate.name
        if hashlib.sha256(published.read_bytes()).hexdigest() != candidate["crate_sha256"]:
            raise RuntimeError("Cargo publication archive differs from the original candidate; stop and inspect registry state")
    print("Cargo dry-run matched the original candidate; nothing uploaded" if dry_run else "Cargo upload completed; checking immutable registry checksum")


def remote_operation(directory, candidate, operation, requested_version, requested_run):
    repo, activation = authorize(candidate, operation, requested_version, requested_run)
    if operation == "authorize":
        print(f"Authorized original candidate {candidate['commit']} from run {requested_run} via activation {activation}; no remote writes")
        return
    current, tag = candidate["version"], candidate["tag"]
    release, ref = release_state(repo, tag)
    crate = registry_version(current)
    if operation == "draft":
        if release or ref or crate is not None:
            raise RuntimeError("version already has a tag, draft, release, or crate; nothing overwritten. Inspect partial/existing state before a separate action.")
        require_main(repo, activation)
        created = github(f"repos/{repo}/git/refs", {"ref": f"refs/tags/{tag}", "sha": candidate["commit"]})
        require_tag(created, candidate)
        require_main(repo, activation)
        assets = [str(path) for path in sorted(directory.iterdir())]
        subprocess.run(["gh", "release", "create", tag, *assets, "--repo", repo,
                        "--draft", "--verify-tag", "--title", f"ReGen {current}",
                        "--notes-file", str(directory / "release-notes.txt")], check=True)
        release, ref = release_state(repo, tag)
        require_tag(ref, candidate)
        verify_draft(repo, release, directory)
        print(f"Created tag {tag} at {candidate['commit']} and verified its draft assets; neither channel published")
        return
    require_tag(ref, candidate)
    verify_draft(repo, release, directory)
    if operation == "publish-crate":
        if crate is not None:
            raise RuntimeError("this immutable crate version already exists; never upload it again")
        require_main(repo, activation)
        cargo_publish(candidate, dry_run=False)
        crate = registry_version(current)
        if crate is None or crate["cksum"] != candidate["crate_sha256"] or crate["yanked"]:
            raise RuntimeError("upload may have completed but a matching unyanked index entry was not observed; inspect manually, do not retry upload")
        print(f"Published regen-ssg {current}; registry checksum matches candidate. GitHub release remains a draft.")
        return
    if crate is None or crate["cksum"] != candidate["crate_sha256"] or crate["yanked"]:
        raise RuntimeError("publish-github requires the matching, unyanked immutable crate to be visible first")
    # No edits to notes/assets: publish only the exact draft already inspected.
    require_main(repo, activation)
    command = ["gh", "api", "--hostname", "github.com", "--method", "PATCH",
               f"repos/{repo}/releases/{release['id']}", "--input", "-"]
    result = subprocess.run(command, input=json.dumps({"draft": False}), text=True, stdout=subprocess.PIPE, check=True)
    published = json.loads(result.stdout)
    if published.get("draft") is not False or published.get("tag_name") != tag:
        raise RuntimeError("GitHub publication outcome was not confirmed; inspect remotely before any further action")
    print(f"Published GitHub release {tag}; both channels now have the reviewed candidate")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", required=True, type=Path)
    parser.add_argument("--commit", help="Require the checkout and candidate to match this full tested Git SHA")
    parser.add_argument("--operation", choices=("prepare", "verify", "check-publish", "authorize", "draft", "publish-crate", "publish-github"), default="prepare")
    parser.add_argument("--release-version", help="Exact workflow_dispatch version authorization")
    parser.add_argument("--candidate-run", help="Exact successful original build.yml workflow run ID")
    args = parser.parse_args()
    current = version()
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", current):
        raise RuntimeError("this release lane requires an exact stable version")
    commit = candidate_commit(args.commit)
    directory = args.directory.resolve(strict=True)
    candidate = prepare(directory, current, commit, verify=args.operation != "prepare")
    if args.operation in {"prepare", "verify"}:
        return
    if args.operation == "check-publish":
        cargo_publish(candidate, dry_run=True)
        return
    remote_operation(directory, candidate, args.operation, args.release_version, args.candidate_run)


if __name__ == "__main__":
    main()
