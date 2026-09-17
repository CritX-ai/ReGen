# Distribution

Cargo builds it; binaries and containers skip the compiler. The package is `regen-ssg`, and the command is always `regen`.

## Cargo

```sh
cargo install regen-ssg --version 1 --locked
regen --version
```

You'll need [Rust](https://rustup.rs/) and a native linker; the [toolchain file](../rust-toolchain.toml) records the tested compiler. Cargo installs the executable. Grab the [optional example](#optional-example) if you're starting from scratch.

`--version 1` follows compatible releases. Pin an exact version when you need repeatable installs.

<details>
<summary>Choose minifiers at installation</summary>

The default Cargo features include all three minifiers: `minify-html`, `minify-css` and `minify-js`. Leave them out for a smaller build, or keep just the one you need:

```sh
cargo install regen-ssg --version 1 --locked --no-default-features
cargo install regen-ssg --version 1 --locked --no-default-features --features minify-css
```

Cargo features decide which engines are in the executable; [profiles and minifier options](reference.md#minifier-controls) decide when they run. Configuration cannot switch on an engine you left out. The HTML feature also includes its [regression-check parser](reference.md#regression-checks).

</details>

### Cargo binstall

[cargo-binstall](https://github.com/cargo-bins/cargo-binstall#installation) downloads a native executable instead of compiling one. Install it first, then run:

```sh
cargo binstall regen-ssg --version 1 --strategies crate-meta-data
regen --version
```

`--strategies crate-meta-data` restricts downloads to official release assets: no third-party fallback or surprise compilation. Add `--no-confirm` for unattended installation.

## Native binaries

Choose your target from [GitHub Releases](https://github.com/CritX-ai/ReGen/releases), verify `SHA256SUMS`, unpack, and put `regen` on your `PATH`.

| System | CPU | Target | Archive |
| --- | --- | --- | --- |
| Linux | x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux | ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS | Intel | `x86_64-apple-darwin` | `.tar.gz` |
| macOS | Apple Silicon | `aarch64-apple-darwin` | `.tar.gz` |
| Windows | x86_64 | `x86_64-pc-windows-msvc` | `.zip` |
| Windows | ARM64 | `aarch64-pc-windows-msvc` | `.zip` |

Archives use `regen-VERSION-TARGET` and the extension above. Linux needs GNU libc, not Alpine's musl; Windows may need the matching [Visual C++ runtime](https://learn.microsoft.com/en-us/cpp/windows/latest-supported-vc-redist).

<details id="archive-contents-and-notices">
<summary>Archive contents and notices</summary>

- The executable, documentation, changelog and bilingual example.
- Project, dependency and Rust notices, plus the lockfile.
- A dependency inventory and separate license archive for each target.
- Release-wide checksums, `DEPENDENCIES.json`, the source crate and build receipts.

**Before redistributing:** read the [known dependency notice-source gaps](license.md#dependency-notice-provenance). Retained notices and checksums do not certify license compliance.

</details>

## Container image

The [Linux image](https://github.com/orgs/CritX-ai/packages/container/package/regen-ssg) supports amd64 and arm64. It carries the matching release's executable, documentation and notices in a digest-pinned GNU-libc runtime. Image tags use the full release version; pin an inspected digest when you need exact bytes.

From your site directory on Linux, use [Podman](https://podman.io/docs) or [Docker](https://docs.docker.com/engine/):

<fieldset class="container-commands">
<legend>Container runtime</legend>
<input class="container-runtime" type="radio" name="container-runtime" id="container-podman" value="podman" aria-controls="container-panel-podman" checked>
<label class="container-tab" for="container-podman">Podman</label>
<input class="container-runtime" type="radio" name="container-runtime" id="container-docker" value="docker" aria-controls="container-panel-docker">
<label class="container-tab" for="container-docker">Docker</label>
<div class="container-panel container-podman" id="container-panel-podman" aria-labelledby="container-podman-heading">
<h3 class="container-runtime-name" id="container-podman-heading">Podman</h3>

```sh
podman run --rm --network none --read-only \
  --read-only-tmpfs=false \
  --userns keep-id --user "$(id -u):$(id -g)" \
  -v "$PWD:/site:rw,Z" \
  ghcr.io/critx-ai/regen-ssg:v1.1.1
```

Use rootless Podman. `keep-id` preserves your user identity; `:Z` gives SELinux a private mount label.

</div>
<div class="container-panel container-docker" id="container-panel-docker" aria-labelledby="container-docker-heading">
<h3 class="container-runtime-name" id="container-docker-heading">Docker</h3>

```sh
docker run --rm --network none --read-only \
  --user "$(id -u):$(id -g)" \
  -v "$PWD:/site:rw" \
  ghcr.io/critx-ai/regen-ssg:v1.1.1
```

The user mapping avoids root-owned output files. Docker Desktop users can omit `--user`.

</div>
</fieldset>

**Configuration stays on your machine.** The mounted `regen.toml` appears at `/site/regen.toml`; content, templates and assets follow the [usual site layout](guide.md#site-layout). Edit the host files and rerun the container.

To choose a profile, append `build --site /site --profile NAME` after the image name. Only the site mount is writable with these commands. Image downloads still need network access; the isolated build does not.

<details id="container-hooks">
<summary>Enable hooks in a custom CI image</summary>

Official images have hooks disabled. For CI jobs that need [trusted build hooks](reference.md#trusted-build-hooks), build your own image with `--features hooks`.

Save this as `Containerfile` in its own directory. The compiler and ReGen release are pinned for repeatable CI builds:

```dockerfile
FROM docker.io/library/rust:1.98.1-bookworm@sha256:9a73a5088750b4c95158ab26629c854c3d6fc4b173cb7bc8079ad252d8ed7bfa
RUN cargo install regen-ssg --version 1.1.1 --locked --features hooks
WORKDIR /site
USER 65532:65532
ENTRYPOINT ["regen"]
CMD ["build", "--site", "/site"]
```

From that directory, run `podman build -t localhost/regen-hooks .` or `docker build -f Containerfile -t localhost/regen-hooks .`. Use `localhost/regen-hooks` in the run command above. Installation needs network access; generation can keep `--network none`.

This custom image retains Rust and the downloaded crate sources. Add any other tools your hooks need, then configure them in the mounted `regen.toml`.

**Isolation reduces exposure; it does not make hooks safe.** Commands can edit or delete the whole writable site, including CI files, and read exposed secrets. Do not give untrusted code a container-engine socket, secrets or network access. Rust/Cargo hooks need explicitly writable build and cache directories. Before redistributing your image, review its dependencies and [notices](license.md#dependency-notice-provenance).

</details>

## Optional example

Download the bilingual example as a [ZIP](https://github.com/CritX-ai/ReGen/releases/download/v1.1.1/regen-example-1.1.1.zip) or [tar.gz](https://github.com/CritX-ai/ReGen/releases/download/v1.1.1/regen-example-1.1.1.tar.gz). Both unpack to `regen-example-1.1.1/` and contain the [minimal site](../examples/minimal) plus `LICENSE`.

Verify the download against the [release's `SHA256SUMS`](https://github.com/CritX-ai/ReGen/releases/tag/v1.1.1), then follow the [quickstart](../README.md#quickstart) and [authoring guide](guide.md).
