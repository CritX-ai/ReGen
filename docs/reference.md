# Build reference

Commands, configuration and optional features. For content and templates, use the [User guide](guide.md); for installation choices, see [Distribution](releasing.md).

## Command line

```sh
regen build --site my-site
```

Omit `--site` to build the current directory. The default `release` profile writes `dist/`; `--profile dev` writes `review/` with development settings. Add `--review` to keep the selected profile's settings while writing to `review/`.

| `regen build` option | Value / default | Effect |
| --- | --- | --- |
| `--site` | Path; `.` | Directory containing `regen.toml` |
| `--profile` | Profile name; `[build].profile`, otherwise `release` | Select built-in or custom settings |
| `--review` | No value; inherit when omitted | Force `review/` output without changing the selected profile or its settings |
| `--minify-html` | `true` or `false`; inherit | Override rendered HTML minification |
| `--minify-css` | `true` or `false`; inherit | Override CSS minification |
| `--minify-js` | `true` or `false`; inherit | Override JavaScript minification |
| `--minify-assets` | `true` or `false`; inherit | Gate minification in `assets/` |
| `--regression-checks` | `true`, `false` or `warn`; automatic when omitted | Enforce static minification checks, disable them or report differences as warnings |
| `--help` | No value | Show command help; `regen --version` shows the executable version |

The four `--minify-*` switches take `true` or `false`; `--regression-checks` also accepts `warn`. `--review` is a flag. CLI overrides apply last and leave `regen.toml` unchanged. Profiles cannot override `site.base_url`.

### Review the final profile

`regen build --profile release --review` keeps release settings and hooks but writes `review/`; substitute a custom profile name when needed. Core generation leaves `dist/` untouched. Templates, the manifest and the hook environment still report the selected profile, with review output enabled.

**Review is not a dry run or hook-suppression flag.** With a hooks-enabled binary, selected hooks still execute and can publish, mutate files or hardcode `dist/`.

<details>
<summary>Build overrides and the Rust API</summary>

`regen::build(&Path)` uses configured defaults. `regen::build_with_options(&Path, &BuildOptions)` accepts these overrides:

- `profile: Option<&str>` and the four `minify_*: Option<bool>` fields mirror the CLI. `regression_checks: Option<RegressionCheckMode>` accepts `Off`, `Warn` or `Enforce`. `None` inherits; regression checks use the automatic policy if no explicit setting is inherited.
- `review: bool` defaults to `false`, meaning inherit. `true` forces review output without changing the profile or hooks. It cannot force a review profile back to `dist/`.

</details>


## Configuration

`regen.toml` requires `[site]` and at least one `[[languages]]` entry. Everything under `[build]` and `[profiles.NAME]` is optional. Unknown fields are errors, including in nested tables and hook entries.

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

### Site settings

Settings in the `[site]` table describe the whole site:

| Field | Type / default | Meaning |
| --- | --- | --- |
| `title` | Required string | Shared project title, exposed as `project.title` |
| `base_url` | Required string | Absolute HTTP(S) origin and optional hosting prefix, such as `/ReGen/` |
| `default_language` | Required string | Configured language served without a language prefix |

### Languages

Declare one `[[languages]]` table per language, including the default language:

| Field | Type / default | Meaning |
| --- | --- | --- |
| `code` | Required string | Unique lowercase language tag; content directory and non-default URL prefix |
| `name` | Required string | Authored display name |
| `direction` | String; `ltr` | `ltr` or `rtl` |

### Common build settings

Choose the default profile and a shared regression-check policy in the optional `[build]` table:

| Field | Type / default | Meaning |
| --- | --- | --- |
| `profile` | String; `release` | Default selected profile |
| `regression_checks` | Boolean or `"warn"`; automatic when omitted | Enforce, disable or warn on [static minification checks](#regression-checks) |

The child tables below provide common settings before profile overrides.

#### Minification

The `[build.minify]` table selects language processing; child option tables tune each engine:

| Field / child table | Type / default | Meaning |
| --- | --- | --- |
| `html` | Optional boolean; inherited | Enable rendered HTML processing |
| `css` | Optional boolean; inherited | Enable standalone CSS compact printing |
| `js` | Optional boolean; inherited | Enable standalone JavaScript processing |
| `html_options` | Optional child table; fields inherit individually | [HTML controls and risks](#html-options) |
| `css_options` | Optional child table; fields inherit individually | [CSS optimization and liveness controls](#css-options) |
| `js_options` | Optional child table; fields inherit individually | [JavaScript grammar and compression controls](#javascript-options) |

For example, common CSS options belong in `[build.minify.css_options]`.

#### Assets

The `[build.assets]` table gates processing of files under `assets/`:

| Field | Type / default | Meaning |
| --- | --- | --- |
| `minify` | Optional boolean; inherited | Permit enabled CSS/JS processing; does not activate minifiers, transform bytes by itself or disable hashing |

#### Hooks

Hook lists under `[build.hooks]` order trusted commands:

| Field | Type / default | Meaning |
| --- | --- | --- |
| `pre` | Array of hook tables; inherited, initially empty | Ordered trusted commands before generation |
| `post` | Array of hook tables; inherited, initially empty | Ordered trusted commands selected by the primary result |

Each explicit list replaces the inherited list; `[]` clears it. Execution requires the opt-in Cargo feature: see [trusted build hooks](#trusted-build-hooks).

There is no arbitrary output path. `review` belongs to profiles, not `[build]`; profiles cannot change site URLs, languages or content layout.

<details>
<summary>URL and language rules</summary>

The base path is unencoded ASCII and case-sensitive, with letters, digits, `-`, `_` and `.` in normal path segments. No credentials, whitespace/control characters, query, fragment, backslashes, empty interior segments or traversal. A trailing slash is normalized away. `/ReGen/about/` is still written to `dist/about/index.html` (or `review/about/index.html`); the host supplies the mount.

Language codes are unique, portable lowercase BCP 47-style tags such as `en`, `pt-br` or `zh-hant-tw`. The supported order is language (2–8 letters), optional extlangs, script, region and variants; singleton extensions, private-use and grandfathered forms are not supported. Registry membership is not checked. The code `assets` is reserved. Declare the default language in the list. Every configured language needs its own content directory.

</details>

### Content and routes

Store each language's shared strings in `content/CODE/site.yaml` and its pages under `content/CODE/pages/`. Every language must have the same page IDs and exactly one home page; missing translations fail.

| Page field | Type / default | Meaning |
| --- | --- | --- |
| `title` | Required string | Page title |
| `description` | Required string | Page description |
| `template` | Required string | Tera template to render |
| `slug` | Optional string; YAML filename without `.yaml` | URL path; `""` selects the home page; `null` is invalid |
| `data` | Mapping; `{}` | Page-specific template data |

Page IDs use the path relative to `pages/`, without `.yaml`, and must match across languages. Slugs may differ between translations:

- `pages/projects/one.yaml` has ID `projects/one` but defaults to slug `one`. Set `slug: projects/one` for a nested URL.
- Slugs may contain `/`. Routes end in `/` and produce an `index.html`.
- `index.yaml` has no special status: omitting its slug produces `/index/`. Use `slug: ""` for the home page.
- Defaulted and explicit slugs undergo the same collision and reserved-route checks.

`site.yaml` and `data` must be string-keyed, JSON-compatible mappings; use `{}` for an empty `site.yaml`. Arbitrary YAML objects, duplicate keys, unknown typed fields, YAML includes, extra content files, symlinks and output collisions are errors. See the [authoring examples](guide.md#localized-yaml).

### Paths and filenames

ReGen preserves ASCII case in filenames, folders and slugs: `public/CNAME`, `assets/fonts/OFL.txt` and `/About/` keep their spelling. Match it in template references, includes and HTML/CSS links.

- Use letters (`A–Z`, `a–z`), digits, `-`, `_` and `.`; `/` separates folders. Leading/trailing dots, traversal and Windows device names such as `CON` or `COM1.txt` are rejected.
- Names must also be unique when compared without case. `Guide/one` and `guide/two` conflict at the folder level, even though their last segments differ. These checks keep the output consistent on case-sensitive and case-insensitive filesystems.
- File types recognize `.yaml`, `.html`, `.css`, `.js` and `.mjs` regardless of extension case. Required layout names such as `regen.toml`, `site.yaml` and `pages/` retain their documented spelling. [Language codes](#languages) have their own lowercase format.

Unicode is welcome in page text and metadata.

### Profiles and precedence

#### Profile fields

Fields under `[profiles.NAME]` select build settings, not site content or URLs:

| Field | Type / default | Meaning |
| --- | --- | --- |
| `extends` | String; `release` for custom profiles | Parent built-in or custom profile; forbidden on built-ins |
| `review` | Boolean; inherited | `true` writes `review/`; `false` writes `dist/`; `--review` forces `true` |
| `regression_checks` | Boolean or `"warn"`; inherited, otherwise automatic | Override the common or parent check policy |

#### Inherited profile children

Replace `NAME` with your profile name. Each child uses the same fields and types as its common build table:

| Child under `profiles.NAME` | Fields | Inheritance |
| --- | --- | --- |
| `minify` | `html`, `css`, `js` | Each boolean inherits independently |
| `minify.html_options` | [HTML option fields](#html-options) | Each field inherits independently |
| `minify.css_options` | [CSS option fields](#css-options) | Each field inherits; an explicit `unused_symbols` list replaces its parent |
| `minify.js_options` | [JavaScript option fields](#javascript-options) | Each field inherits independently |
| `assets` | `minify` | Inherits the asset permission gate |
| `hooks` | `pre`, `post` | Each explicit list replaces that whole inherited list |

For example, profile CSS options belong in `[profiles.custom-publish.minify.css_options]`.

#### Built-in defaults and resolution

| Built-in profile | `review` / destination | `minify.html`, `.css`, `.js` | `assets.minify` | Hooks |
| --- | --- | --- | --- | --- |
| `release` | `false` / `dist/` | HTML `false`; CSS `true` when compiled in; JS `false` | `true` | None |
| `dev` | `true` / `review/` | All `false` | `false` | None |

`assets.minify` permits standalone CSS/JS processing; it neither enables a language nor affects rendered HTML. Fine-grained options never activate a minifier. Release leaves CSS optimization off and `unused_symbols = []`. HTML and JS stay off because their native rewrites and grammar choices need explicit review.

Customize either built-in without `extends`. Custom profiles default to `release` and may extend `dev` or another custom profile.

Precedence is **built-in defaults → common build tables → ancestors, oldest first → selected profile → explicit CLI/API overrides**.

Fields inherit individually; a partial option table does not reset its siblings. Explicit `unused_symbols`, `pre` and `post` arrays replace their respective inherited arrays. `[]` clears one without affecting the others.

Profile names are case-sensitive, single [portable path segments](#paths-and-filenames). Unknown parents, cycles and invalid `[build].profile` fail even when another profile is selected. All hook declarations are validated; compiled-in features are checked only against effective settings.

<details>
<summary>Profile example</summary>

```toml
[profiles.preview]
extends = "dev"

[profiles.preview.assets]
minify = true

[profiles.preview.minify]
css = true

[profiles.preview.hooks]
pre = []
post = []
```

`regen build --profile preview` compact-prints CSS into `review/` and clears inherited hooks. Other settings still inherit; see [build sequencing](architecture.md#build-sequence).

</details>

### Regression checks

ReGen compares minified content before installing output.

If `regression_checks` is unset in common settings and the selected profile chain, checks turn **on** when effective settings enable HTML minification, CSS optimization or JavaScript compression. CSS/JS also require `assets.minify` and their language switch. Inactive profiles and disabled processors do not activate checks; ordinary release CSS compact printing leaves them off.

Set `regression_checks` under `[build]` or `[profiles.NAME]`:

| Value | Result |
| --- | --- |
| `true` | Run checks and stop on a difference, preserving previous output |
| `"warn"` | Run the same comparisons, print warnings and continue installing output |
| `false` | Skip regression checks |

Profile inheritance applies, then `--regression-checks true|false|warn` or `BuildOptions::regression_checks` wins. An explicit setting overrides automatic selection. Input, minifier and filesystem errors still fail the build in warning mode.

Each check compares the input with the processed result:

| Processed content | Check and boundary |
| --- | --- |
| HTML | Compare HTML5-parsed trees and document mode, including attributes, comments, text and text whitespace. Equivalent quote/entity spelling can pass; native whitespace or attribute removal can fail even when the minifier considers it acceptable. |
| CSS | Reparse emitted CSS and compare its native compact, non-optimizing representation with the input's. Optimizer changes, including deliberate symbol removal, can be rejected. |
| JavaScript | Reparse with the same script/module grammar and compare native compact output without compression, retaining legal notices. Changed statements, names or removed effects can be rejected. |

Diagnostics name the page or asset. Enforced differences stop the build **before output replacement** and select eligible failure hooks. Warning-only differences leave the build successful, so success/always hooks run if the rest of the build succeeds.

These checks do not execute JavaScript, measure browser layout or prove semantic equivalence. CSS/JS comparisons use their native parsers, not independent engines; HTML recovery can normalize malformed input. Conservative checks may reject intentional equivalent rewrites.

Use `--regression-checks warn --review` to inspect differences in separate output, then decide which rewrites to accept. Keep application-specific browser/CI tests for behavior these static checks cannot observe. See [test scope](testing.md#site-regression-checks).

### Minifier controls

Set HTML options in `[build.minify.html_options]`, or `[profiles.NAME.minify.html_options]` for a profile. Substitute `css_options` or `js_options` for the other engines.

Replace `NAME` with your profile name. Fields inherit individually; the language and asset switches decide whether processing runs. CLI/API booleans do not reset these options.

#### HTML options

<details>
<summary>HTML controls and native rewrite risks</summary>

`html_options` maps to [minify-html 0.18.1's `Cfg`](https://docs.rs/minify-html/0.18.1/minify_html/struct.Cfg.html), turning upstream `keep_*` flags into opt-ins for removal. All eleven fields default to `false`.

**HTML is off by default.** Enabling it accepts native whitespace rewriting and attribute/quote normalization; there is no switch to retain all whitespace, attributes or quotes. Even with every option false, DOM, inline spacing, CSS `white-space`, selectors and string comparisons may change. `html = false` preserves rendered Tera bytes, not template source.

| `html_options` field | Type / default | Effect when enabled and risks |
| --- | --- | --- |
| `remove_comments` | Boolean; `false` | Remove ordinary HTML comments, including legal notices, conditional comments and application markers; SSI comments have their own preservation control |
| `remove_ssi_comments` | Boolean; `false` | Permit server-side-include comment removal; effective only with `remove_comments = true`, because keeping all comments takes precedence |
| `omit_closing_tags` | Boolean; `false` | Allow native optional closing-tag omission; changes markup consumed by non-browser tools |
| `omit_html_head_opening_tags` | Boolean; `false` | Allow omission of attribute-free `html` and `head` opening tags; changes explicit document structure in source |
| `remove_input_type_text` | Boolean; `false` | Remove `type=text` from inputs; attribute selectors and DOM attribute inspection can observe the difference |
| `minify_doctype` | Boolean; `false` | Compact the doctype using native rules; output may not be specification-compliant |
| `allow_noncompliant_unquoted_attribute_values` | Boolean; `false` | Allow otherwise prohibited characters in unquoted values; relies on browser tolerance and may fail validators or other consumers |
| `allow_optimal_entities` | Boolean; `false` | Allow shorter entity forms that may not validate; relies on browser tolerance |
| `allow_removing_spaces_between_attributes` | Boolean; `false` | Allow attribute separators to be removed where the library permits; may not be specification-compliant |
| `remove_bangs` | Boolean; `false` | Remove native `<!...>` bang constructs, not the doctype; can discard author/tool-specific markup |
| `remove_processing_instructions` | Boolean; `false` | Remove `<?...>` processing instructions; can discard downstream processor instructions |

ReGen does not sanitize HTML, analyze runtime use or reinsert notices. Inline CSS/JS minifiers are always off, but surrounding HTML constructs may change. Compute CSP hashes from final output; keep HTML processing off when unavoidable rewrites are unacceptable.

</details>

#### CSS options

<details>
<summary>CSS compact printing and optimization controls</summary>

CSS compact printing is separate from optimization:

| `css_options` field | Type / default | Meaning |
| --- | --- | --- |
| `optimize` | Boolean; `false` | Run [Lightning CSS](https://lightningcss.dev/)'s stylesheet optimizer before compact printing |
| `unused_symbols` | String array; `[]` | Explicitly declare symbols unused for optimizer removal |

Enabled CSS always parses and compact-prints through Lightning CSS, even without [native optimization](https://lightningcss.dev/minification.html). Nonempty `unused_symbols` requires `optimize = true` when CSS processing runs.

ReGen does no whole-site liveness analysis: never declare symbols used by templates, scripts or dynamic states unused. Release defaults to compact printing only; disable `minify.css` or the asset gate to preserve authored bytes.

</details>

#### JavaScript options

<details>
<summary>JavaScript grammar, compression and risks</summary>

Choose the grammar and any compression steps explicitly:

| `js_options` field | Type / default | Meaning |
| --- | --- | --- |
| `compress` | Boolean; `false` | Run [Oxc](https://oxc.rs/)'s compressor before compact emission |
| `drop_debugger` | Boolean; `false` | Remove `debugger` statements |
| `drop_console` | Boolean; `false` | Remove console calls, including their argument effects |
| `join_vars` | Boolean; `false` | Join consecutive variable declarations |
| `sequences` | Boolean; `false` | Combine statements with the comma operator |
| `remove_unused` | Boolean; `false` | Allow removal of bindings Oxc considers unreferenced |
| `pure_annotations` | Boolean; `false` | Trust authored purity annotations for tree shaking |
| `keep_names` | Boolean; `true` | Preserve function and class names during compression |
| `source_type` | String; `"script"` | Grammar for `.js`: `"script"` or `"module"`; `.mjs` always uses module grammar |

Enabled JS parses and compact-emits even with `compress = false`; this does not preserve authored bytes. Compression uses [Oxc 0.95.0's `safest()`](https://docs.rs/oxc_minifier/0.95.0/oxc_minifier/struct.CompressOptions.html#method.safest), not a universal safety guarantee, with no identifier mangling or assumed-pure functions. It does not implicitly enable other destructive options.

When JS processing runs, other true compression options or `keep_names = false` require `compress = true`. Grammar is independent of compression.

**Destructive opt-ins:** console removal can discard argument effects (`console.log(updateState())`); incorrect purity annotations can discard work; removing unused globals can break other scripts; dropping names changes reflection/registries. ReGen does not analyze whole-site liveness.

`.js` defaults to classic script grammar; `.mjs` always uses module grammar. There is no syntax guessing or parser-error fallback: invalid enabled input fails. For module-only `.js`, use the settings below; `dev` also needs `assets.minify = true`.

```toml
[build.minify]
js = true

[build.minify.js_options]
source_type = "module"
```

Module grammar is strict and changes top-level semantics. One profile selects one grammar for all `.js` assets; for mixed sites, keep `.js` as scripts and use `.mjs` for modules, or disable JS processing. HTML `script` tags do not select grammar.

</details>


### Trusted build hooks

Standard binaries **do not include hooks**. To allow configured commands to run, install:

```sh
cargo install regen-ssg --version 1 --locked --features hooks
```

From source, use `cargo install --path . --locked --features hooks`. There is no separate CLI enable flag. Without this feature, nonempty effective pre/post lists fail before commands run or output changes; hooks in inactive profiles are allowed, and effective `[]` disables them.

Hooks run with your user privileges and inherited environment. They can access secrets and the network; ReGen cannot roll back their effects. Review configured and inherited commands before building an unfamiliar site.

<details>
<summary>Hook fields and execution boundaries</summary>

Each hook supplies a command and optional failure handling:

| Hook field | Pre | Post | Meaning |
| --- | --- | --- | --- |
| `command` | Required string array | Required string array | Nonempty argv; first element is a nonempty executable, followed by literal arguments. No embedded NUL |
| `allow_failure` | Boolean; `false` | Boolean; `false` | Warn and continue instead of returning a hook error |
| `when` | Not allowed | `success`, `failure`, `always`; default `always` | Match the primary build outcome |
| `error_details` | Not allowed | Boolean; `false` | Opt in to the primary failure's error chain via environment |

Commands run sequentially at the canonical site root with inherited environment/stdout/stderr and closed stdin. Arguments are literal: no implicit shell, globbing or expansion; `"$REGEN_OUTPUT"` is not substituted. Install required tools yourself.

Post hooks use the fixed primary outcome. A required post failure can occur **after output installation** and cannot roll it back; hook edits invalidate the written manifest. See [lifecycle, failure handling and environment variables](architecture.md#trusted-process-boundary), including sensitive opt-in error details.

The [example reporter](../examples/minimal/hooks/report.py) is runnable with `regen build --site examples/minimal --profile logged` using a hooks-enabled binary and Python 3.

</details>

## Output and optimization

Processing depends on the selected profile, overrides and compiled-in features:

| Input | Result |
| --- | --- |
| Rendered HTML | Rendered Tera bytes unchanged by default; native minify-html processing only when `minify.html = true`, including unavoidable whitespace and attribute normalization |
| Inline CSS / JavaScript / JSON | Not passed through language minifiers; opting into HTML processing still permits surrounding native HTML rewrites |
| `assets/**/*.css` | Compact-printed when both `assets.minify` and `minify.css` are true; the release default when compiled in |
| `assets/**/*.js`, `assets/**/*.mjs` | Minified only when both `assets.minify` and `minify.js` are true |
| Other assets, or assets with their minification disabled | Authored bytes copied unchanged |
| Whole asset tree | `assets/<sha256>/`, preserving relative paths, whether minified or not |
| `public/` | Unchanged bytes and stable paths |

One changed asset changes the whole asset-tree URL. ReGen also writes `sitemap.xml` and `regen-manifest.json`. There is no bundling, remote fetching, transpilation, source-map generation or image processing.

JS preserves legal comments, including trailing notices; review the [grammar and compression risks](#javascript-options) before enabling it. Compute CSP hashes from final output.

See [architecture](architecture.md#output-ownership-and-interruption-recovery) for reproducibility, manifest fields, transactional output and interruption recovery. Hooks sit outside the deterministic core's guarantees.
