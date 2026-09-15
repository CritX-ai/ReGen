# User guide

YAML for the words. Tera for the HTML. `regen build` for the repetitive bits.

## Installation

With Rust **1.98.1** and a native linker:

```sh
cargo install regen-ssg --version 1.0.0 --locked
```

Prefer no compiler? Download a [native binary](releasing.md#native-binaries), verify the checksum and put `regen` on your `PATH`.

<details>
<summary>Install from a source checkout</summary>

```sh
cargo install --path . --locked
```

The checkout selects the pinned Rust toolchain.

</details>

## Command line

```sh
regen build --site my-site
```

Omit `--site` to build the current directory. The result is `dist/`.

<details>
<summary>Preview locally with Caddy</summary>

From the site directory:

```sh
caddy file-server --root dist --listen 127.0.0.1:8000
```

Open `http://127.0.0.1:8000/`. Set `site.base_url` to that URL before building; switch it back for [deployment](deployment.md). [Install Caddy](https://caddyserver.com/docs/install).

</details>

## Example

[The complete bilingual example](../examples/minimal) · [English result](https://regen.critx.ai/guide/poem/) · [German result](https://regen.critx.ai/guide/poem/de/)

## Site layout

```text
regen.toml              site URL and languages
content/
  en/
    site.yaml           shared English strings
    pages/index.yaml    English home page
  de/
    site.yaml           shared German strings
    pages/index.yaml    German home page
templates/              Tera HTML templates
assets/                 optimized, hashed files — optional
public/                 unchanged, stable-path files — optional
dist/                   generated site
```

**Words → `content/`. Layout → `templates/`. Files → `assets/` or `public/`.** Edit those inputs, then rebuild. Leave `dist/` alone.

## Configuration

`regen.toml`:

```toml
[site]
title = "Example"
base_url = "https://example.com"
default_language = "en"

[[languages]]
code = "en"
name = "English"

[[languages]]
code = "de"
name = "Deutsch"
```

- `base_url`: your HTTP(S) origin, plus any hosting prefix such as `/ReGen/`.
- `default_language`: the language served without a language prefix.
- `direction`: optional `ltr` or `rtl` on each language; defaults to `ltr`.

<details>
<summary>URL and language rules</summary>

The base path is unencoded ASCII and case-sensitive. No credentials, query, fragment, backslashes, empty interior segments or traversal. `/ReGen/about/` is still written to `dist/about/index.html`; the host supplies the mount.

Language codes are unique, lowercase BCP 47-style tags such as `en` or `pt-br`. Declare the default language in the list. Every configured language needs its own content directory.

</details>

## Localized YAML

A page in `content/en/pages/about.yaml`:

```yaml
title: About
description: A short introduction.
template: page.html
slug: about
data:
  paragraphs:
    - A small site with explicit content and templates.
```

**Same filename, same page. Different slug, different URL.**

```text
en/pages/about.yaml  → slug: about → /about/
de/pages/about.yaml  → slug: ueber → /de/ueber/
```

Every language needs the same page IDs and one home page with `slug: ""`. A missing translation stops the build.

<details>
<summary>Page fields and shared strings</summary>

`title`, `description`, `template` and `slug` are required. `data` is your mapping of page-specific values; it defaults to `{}`. Unicode belongs in your text.

Put repeated localized text in each language's `site.yaml`:

```yaml
navigation_label: Navigation
footer_text: Made with care.
```

Use `{}` when there are no shared strings.

</details>

<details>
<summary>Nested pages and rejected inputs</summary>

- `pages/projects/one.yaml` has ID `projects/one`; match that ID in every language.
- Slugs can contain `/`. Routes always end in `/` and produce an `index.html`.
- Paths use portable lowercase ASCII segments: no traversal, leading dots, Windows device names or trailing dots.
- Duplicate YAML keys, unknown typed fields, YAML includes, extra content files, symlinks and output collisions are errors.
- `site.yaml` and `data` accept string-keyed, JSON-compatible mappings. Arbitrary YAML objects are not supported.

</details>

## Tera templates

Render page data in `templates/page.html`:

```html
<h1>{{ page.title }}</h1>
{% for paragraph in page.data.paragraphs %}
<p>{{ paragraph }}</p>
{% endfor %}
```

Use inheritance for your document shell, as the [example templates](../examples/minimal/templates) do. ReGen provides the following context:

<details>
<summary>Content and language values</summary>

| Value | Contents |
| --- | --- |
| `project` | Global `[site]` configuration |
| `site` | Current language's `site.yaml` |
| `page` | Page fields and `data` |
| `language` | `code`, `name`, `direction` |
| `navigation` | Current language's pages: `id`, `title`, `path`, `url` |
| `alternates` | This page's translations: `code`, `name`, `path`, `url`, `direction` |

</details>

<details>
<summary>URLs that work under a hosting prefix</summary>

| Value | Use |
| --- | --- |
| `site_root` | Home and public files: `{{ site_root }}robots.txt` |
| `current_path` | Current root-relative route |
| `canonical_url` | Absolute page URL |
| `asset_base` | Hashed assets: `{{ asset_base }}/site.css` |

These values, navigation and alternates include the hosting prefix. `asset_base` has no trailing slash; `site_root` does. ReGen does not rewrite hardcoded URLs in your HTML or CSS.

Templates control canonical and hreflang tags. The example's `base.html` shows both.

</details>

<details>
<summary>Filters, includes and trusted HTML</summary>

Tera inheritance, template includes, components, filters and HTML autoescaping are available. ReGen's `group_by` sorts group keys and keeps input order within each group; no time or random functions are registered.

Use `safe` only for HTML you trust: it skips escaping. HTML escaping does not make JavaScript, CSS or arbitrary URLs safe. See the [security policy](../SECURITY.md#runtime-trust-boundaries).

</details>

## Output and optimization

| Input | Result |
| --- | --- |
| Rendered HTML | Minified; comments retained. Inline CSS/JavaScript not separately minified, but surrounding whitespace can change |
| `assets/*.css` | Minified; no import bundling or remote fetching |
| Other assets | Unchanged bytes |
| Whole asset tree | `assets/<sha256>/`, preserving relative paths |
| `public/` | Unchanged bytes and stable paths |

One changed asset gives the whole asset tree a new URL. The build also writes `sitemap.xml` and `regen-manifest.json`.

<details>
<summary>Determinism, manifests and interrupted builds</summary>

The same source paths and bytes, built with the same ReGen executable, produce the same output paths and bytes. Keep inputs unchanged during a build.

The manifest records format `1`, generator, version, and sorted file hashes and byte counts. It excludes itself and has no timestamp. The sitemap records canonical URLs and language alternates.

ReGen prepares output beside the old site before replacing it. Unrecognized `dist/` and existing recovery directories block the build. After an interruption, follow the [recovery procedure](architecture.md#output-ownership-and-interruption-recovery).

</details>

[Explore the architecture](architecture.md) · [Deploy the result](deployment.md)
