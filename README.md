[![ReGen logo](site/assets/regen-logo.svg)](site/assets/regen-logo.svg)

# ReGen: Reproducible Static Site Generator

[![GitHub release](https://img.shields.io/github/v/release/CritX-ai/ReGen?label=GitHub&style=flat-square)](https://github.com/CritX-ai/ReGen/releases/latest) [![crates.io release](https://img.shields.io/crates/v/regen-ssg?label=crates.io&style=flat-square)](https://crates.io/crates/regen-ssg) [![GHCR container image](https://img.shields.io/badge/GHCR-container-blue?logo=github&style=flat-square)](https://github.com/orgs/CritX-ai/packages/container/package/regen-ssg) [![CI status](https://img.shields.io/github/actions/workflow/status/CritX-ai/ReGen/build.yml?branch=main&label=CI&style=flat-square)](https://github.com/CritX-ai/ReGen/actions/workflows/build.yml) [![Rust line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fregen.critx.ai%2Fcoverage.json&style=flat-square)](https://github.com/CritX-ai/ReGen/actions/workflows/build.yml?query=branch%3Amain)

ReGen is an opinionated, battle-tested static site generator written in Rust. Black magic, with the curtains open.

Write localized YAML, add [Tera](https://keats.github.io/tera/) HTML templates, and run `regen build`. Out comes a static site, with optional HTML, CSS and JavaScript minification in Rust. Core generation stays offline.

The **[documentation](https://regen.critx.ai/)** follows Goethe's "Ein Gleiches" from YAML to a German and English site. Poetry in, static files out.

Your templates, your content, your design. The bundled example gets the files in place; the look is up to you.

## Features

- **Translations that travel together** — write each language's content separately, then connect translations by page ID. ReGen builds [localized routes, navigation and language links](docs/guide.md#localized-yaml), catching missing pages before they reach readers.
- **Your markup, your design** — build pages from [reusable Tera templates](docs/guide.md#tera-templates), localized YAML and your own assets. Keep shared layouts in one place and give each page the data it needs.
- **Build, review, repeat** — [profiles](docs/reference.md#profiles-and-precedence) keep development and release settings together. Preview the final settings in [separate review output](docs/reference.md#review-the-final-profile); content-hashed asset URLs handle changed files.
- **Minify with a second opinion** — tune the [native minifiers](docs/reference.md#minifier-controls), then compare their output with [Rust regression checks](docs/reference.md#regression-checks). Risky processing enables checks by default; choose enforced checks, warnings or an explicit opt-out.
- **Automate the chores** — opt-in [build hooks](docs/reference.md#trusted-build-hooks) run trusted commands before generation and after success or failure. Prepare inputs, run your own checks or connect the build to CI.

## Quickstart

With [Rust](https://rustup.rs/) and a native linker installed:

```sh
cargo install regen-ssg --version 1 --locked
```

Download the [minimal example ZIP](https://github.com/CritX-ai/ReGen/releases/download/v1.1.0/regen-example-1.1.0.zip) and extract `regen-example-1.1.0/`.

```sh
regen build --site regen-example-1.1.0
```

The build writes English and German pages into `regen-example-1.1.0/dist/`. [Edit the localized content](docs/guide.md#localized-yaml), rebuild, and [preview locally](docs/guide.md#preview-locally-with-caddy).

Rather skip the compiler? Use [cargo-binstall](docs/releasing.md#cargo-binstall), a [native binary](docs/releasing.md#native-binaries) or a [container](docs/releasing.md#container-image).

## Built with ReGen

ReGen already builds real sites:

- **[jan.toennemann.net](https://jan.toennemann.net/)** — a bilingual portfolio of values, photography, music and research.
- **[ReGen's documentation](https://regen.critx.ai/)** — built by the tool it explains. [See the source](examples/build_docs.rs).

## Architecture overview

Three opinions keep the build predictable:

- **Translations are explicit.** Matching page IDs connect languages even when their slugs differ. A missing translation stops the build instead of substituting another language.
- **Same inputs, same output.** Source paths and bytes, the executable and selected settings determine the core build.
- **Prepare first, replace second.** ReGen builds beside the previous site. A failure before installation leaves it in place.

[Explore the architecture](docs/architecture.md) · [Deploy your site](docs/deployment.md)

## Project policy

ReGen's original code and documentation use [WTFPL v2](LICENSE). Fonts, the GitHub icon, dependencies and the CritX logo retain their [separate licensing terms](docs/license.md). The CritX logo may remain only in unmodified bundled copies unless separately permitted.

**Pull requests and issues are disabled**; see the [contribution policy](CONTRIBUTING.md) and [security policy](SECURITY.md).
