[![ReGen logo](site/assets/regen-logo.svg)](site/assets/regen-logo.svg)

# ReGen: Reproducible Static Site Generator

[![GitHub release](https://img.shields.io/github/v/release/CritX-ai/ReGen?label=GitHub&style=flat-square)](https://github.com/CritX-ai/ReGen/releases/latest) [![crates.io release](https://img.shields.io/crates/v/regen-ssg?label=crates.io&style=flat-square)](https://crates.io/crates/regen-ssg) [![CI status](https://img.shields.io/github/actions/workflow/status/CritX-ai/ReGen/build.yml?branch=main&label=CI&style=flat-square)](https://github.com/CritX-ai/ReGen/actions/workflows/build.yml) [![Rust line coverage](https://img.shields.io/endpoint?url=https%3A%2F%2Fregen.critx.ai%2Fcoverage.json&style=flat-square)](https://github.com/CritX-ai/ReGen/actions/workflows/build.yml?query=branch%3Amain)

ReGen is an opinionated, battle-tested static site generator written in Rust. Black magic, with the curtains open.

Write your pages in localized YAML, give them Tera HTML templates, and run `regen build`. ReGen turns those local files into a complete static site, with asset optimization and HTML minification handled by Rust. Generation needs no server, JavaScript tooling, or network requests. No plugin circus to assemble before the first page.

**[Documentation](https://regen.critx.ai/)** is built with ReGen itself. Starting with Goethe's poem "Ein Gleiches" in German and English, it traces the build process, giving a glimpse into the use and architecture.

## Features

- **Localized content** — paired YAML pages, translated slugs and Tera templates.
- **Reproducible output** — content-addressed assets, sitemaps and SHA-256 manifests.
- **Built-in optimization** — HTML and standalone CSS minification; JavaScript and media bytes preserved.
- **Offline builds** — one Rust executable, with no server or JavaScript build tooling required.

## Quickstart


With [Rust](https://rustup.rs/) installed:

```sh
cargo install regen-ssg --version 1.0.0 --locked
git clone --depth 1 --branch v1.0.0 https://github.com/CritX-ai/ReGen.git
regen build --site ReGen/examples/minimal
```

Your site is in `ReGen/examples/minimal/dist/`. Edit the YAML, rebuild, repeat. See the [guide](https://regen.critx.ai/guide/) for local preview, or grab a [native binary](https://github.com/CritX-ai/ReGen/releases/latest) instead of compiling.

## Built with ReGen

**1.0.0 is our first major release — not our first working build.** ReGen already builds real sites. 

- **[jan.toennemann.net](https://jan.toennemann.net/)** — A personal portfolio spanning values, photography, music, and research - bilingual support built in.
- **[ReGen's documentation](https://regen.critx.ai/)** — These pages are built by the tool they explain. [Inspect the source](examples/build_docs.rs).

## Architecture overview

Three opinions keep the build predictable:

- **Translations are explicit.** Matching page IDs connect languages even when their slugs differ. A missing translation stops the build instead of substituting another language.
- **The inputs decide the output.** Unchanged source paths and bytes, built with the same ReGen executable, produce the same output paths and bytes.
- **Prepare first, replace second.** ReGen builds beside the previous site. A failure before installation leaves it in place.

[Architecture](https://regen.critx.ai/architecture/) · [Deployment](https://regen.critx.ai/deployment/)

## Project policy

ReGen's original code and documentation use [WTFPL v2](LICENSE). Fonts, the GitHub icon, dependencies and the CritX logo retain their [separate licensing terms](docs/license.md). The CritX logo may remain only in unmodified bundled copies unless separately permitted.

**Pull requests are disabled by project policy**; see [contribution policy](https://regen.critx.ai/contributing/) and [security policy](https://regen.critx.ai/security/).
