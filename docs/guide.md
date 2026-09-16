# User guide

YAML for the words. [Tera](https://keats.github.io/tera/#template) for the HTML. `regen build` for the repetitive bits.

## Start with a site

Start with the [Cargo quickstart](../README.md#quickstart). This guide picks up with a site folder containing `regen.toml`, content and templates. For other installation methods and the optional example archive, see [Distribution](releasing.md).

## Build and preview

From your site directory, run `regen build` after editing. The default `release` profile writes `dist/`; `regen build --profile dev` writes unminified output to `review/`.

For a final review without replacing `dist/`, use the [selected release profile in review mode](reference.md#review-the-final-profile). See the [command reference](reference.md#command-line) for overrides.

<details id="preview-locally-with-caddy">
<summary>Preview locally with Caddy</summary>

[Install Caddy](https://caddyserver.com/docs/install), then run from the site directory:

```sh
caddy file-server --root dist --listen 127.0.0.1:8000
```

Open `http://127.0.0.1:8000/`. Use `--root review` for a review build. Set `site.base_url` to that URL before building; switch it back for [deployment](deployment.md).

</details>

## Site layout

```text
regen.toml              site URL, languages and optional build settings
content/
  en/
    site.yaml           shared English strings
    pages/index.yaml    English home page
  de/
    site.yaml           shared German strings
    pages/index.yaml    German home page
templates/              Tera HTML templates
assets/                 hashed files, optionally minified — optional
public/                 unchanged, stable-path files — optional
dist/                   release output (review/ for review builds)
```

**Words → `content/`. Layout → `templates/`. Files → `assets/` or `public/`.** Rebuild after editing; hand-edited output won't survive. Browse the [bilingual example](../examples/minimal) and its [English](https://regen.critx.ai/guide/poem/) / [German](https://regen.critx.ai/guide/poem/de/) results.

## Localized YAML

Create a page such as `content/en/pages/about.yaml`:

```yaml
title: About
description: A short introduction.
template: page.html
data:
  paragraphs:
    - A small site with explicit content and templates.
```

**Same relative filename, same translated page.** An omitted `slug` uses the YAML filename without `.yaml`; set it explicitly to localize or nest the URL.

```text
en/pages/about.yaml  → slug omitted → /about/
de/pages/about.yaml  → slug: ueber → /de/ueber/
```

Every language needs the same page IDs and one home page with `slug: ""`; missing translations fail. `title`, `description` and `template` are required. `slug` is an optional string, not `null`; `data` is a mapping and defaults to `{}`.

Put shared localized strings in each language's `site.yaml` (or `{}` when empty). Use [portable ASCII filenames](reference.md#paths-and-filenames) with consistent case; Unicode is welcome in the text.

<details>
<summary>Nested pages and rejected inputs</summary>

- `pages/projects/one.yaml` has ID `projects/one` but defaults to slug `one`. Match the full ID in every language; use `slug: projects/one` for a nested URL.
- Slugs can contain `/`. Routes always end in `/` and produce an `index.html`.
- `index.yaml` is not special: without a slug it produces `/index/`. Keep `slug: ""` on the home page.

See [content and route rules](reference.md#content-and-routes) for path restrictions, YAML validation and collision checks.

</details>

## Tera templates

Render page data in `templates/page.html`:

```html
<h1>{{ page.title }}</h1>
{% for paragraph in page.data.paragraphs %}
<p>{{ paragraph }}</p>
{% endfor %}
```

Use [template inheritance](../examples/minimal/templates) for the document shell; start with the example's `base.html`.

<details>
<summary>Template context and prefixed URLs</summary>

These values describe the content being rendered and the current build:

| Value | Contents |
| --- | --- |
| `project` | Global `[site]` configuration |
| `site` | Current language's `site.yaml` |
| `page` | Page fields and `data` |
| `build` | Selected `profile` name and `review` boolean |
| `language` | `code`, `name`, `direction` |
| `navigation` | Current language's pages: `id`, `title`, `path`, `url` |
| `alternates` | This page's translations: `code`, `name`, `path`, `url`, `direction` |

Use these URL values instead of hardcoding the domain or hosting prefix:

| Value | Use |
| --- | --- |
| `site_root` | Home and public files: `{{ site_root }}robots.txt` |
| `current_path` | Current root-relative route |
| `canonical_url` | Absolute page URL |
| `asset_base` | Hashed assets: `{{ asset_base }}/site.css` |

URLs, navigation and alternates include the hosting prefix. `site_root` ends in `/`; `asset_base` does not. ReGen does not rewrite hardcoded HTML/CSS URLs. Templates supply canonical and hreflang tags.

Tera supports includes, components, filters and HTML autoescaping. ReGen's `group_by` sorts keys and preserves input order within groups; no time/random functions are registered.

`safe` bypasses escaping: use it only for trusted HTML. HTML escaping does not secure JavaScript, CSS or arbitrary URLs. See [trust boundaries](../SECURITY.md#runtime-trust-boundaries).

</details>

## Assets and output

Put CSS, scripts and images in `assets/`, then link them with `{{ asset_base }}`. ReGen keeps relative paths under one content-hashed directory; any asset change updates that directory's URL. Use `public/` for unchanged files at stable paths, such as `robots.txt`.

Release builds compact-print CSS when CSS support is compiled in. HTML and JavaScript processing are opt-in. [Regression checks](reference.md#regression-checks) run automatically for unsafe minification and can be configured per profile.

Set the URL and languages in `regen.toml` using the [configuration reference](reference.md#configuration). ReGen also generates a sitemap and an ownership manifest. Publish the output directory, not the source files.

[Choose output settings](reference.md#output-and-optimization) · [Explore the architecture](architecture.md) · [Deploy the result](deployment.md)
