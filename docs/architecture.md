# Architecture

Two views of the same generator: **software architecture** maps the package and private modules; **build process** follows execution order and failure boundaries.

## Crate composition

The [`regen-ssg` Cargo package](../Cargo.toml) produces the `regen` binary and library. Rust callers use `build(&Path)` or `build_with_options(&Path, &BuildOptions)` and receive `BuildSummary`. Implementation modules stay private; the CLI and Rust callers share configuration.

<div class="crate-composition">

| Module | Responsibility |
| --- | --- |
| <span id="cli-boundary"></span>[`main.rs`](../src/main.rs) | Parse CLI arguments and report results; call the same library API used by Rust applications. |
| <span id="build-coordinator"></span>[`lib.rs`](../src/lib.rs) | Coordinate [Tera](https://keats.github.io/tera/) context, rendering, escaping, grouping, sitemap, manifest and combined errors. |
| <span id="input-model"></span>[`config.rs`](../src/config.rs) | Read strict TOML; validate languages, URLs, profiles and hooks; resolve inherited settings and available features. |
| [`content.rs`](../src/content.rs) | Inspect locale layout, decode YAML, pair translation IDs and derive routes using configuration and the shared filesystem inventory. |
| [`assets.rs`](../src/assets.rs) | Process standalone CSS/JavaScript, check native canonical output and hash final bytes. See [engine policy](#asset-pipeline). |
| [`html.rs`](../src/html.rs) | Adapt the native HTML engine and compare parsed trees with `minify-html`. |
| [`hooks.rs`](../src/hooks.rs) | Execute trusted commands only with the non-default `hooks` feature. See the [process boundary](#trusted-process-boundary). |
| <span id="filesystem-boundary"></span>[`files.rs`](../src/files.rs) | Check paths, inventory files, bound text reads and write exclusively; `Transaction` stages output and tracks recovery. |

</div>

Configuration, content, assets and the coordinator share filesystem helpers; [site replacement and recovery](#output-ownership-and-interruption-recovery) explain the transaction's limits.

<span id="outside-the-runtime"></span>**Documentation and tests:** [`examples/build_docs.rs`](../examples/build_docs.rs) converts repository Markdown into ordinary site inputs and reads direct dependency/feature declarations from [`Cargo.toml`](../Cargo.toml) for this overview. [`tests/generation.rs`](../tests/generation.rs) covers builds and the CLI; [`tests/unit/`](../tests/unit) covers bytes, escaping and transactions within their owning modules. See [test scope](testing.md#minifier-confidence-and-upstream-evidence).

## Asset pipeline

The default Cargo features `minify-html`, `minify-css` and `minify-js` make engines **available**, not automatically active. Resolved settings select processing; requesting an unavailable engine fails before generation. CSS/JS need both the asset gate and their language switch. See [profile defaults and precedence](reference.md#configuration) and [minifier controls](reference.md#minifier-controls).

**Native-engine limits:** [LightningCSS](https://lightningcss.dev/) and [Oxc](https://oxc.rs/) parse and print standalone assets; neither exact bytes nor equivalent behavior for every site is guaranteed when enabled. Oxc uses configured `source_type` for `.js` (default `script`) and always module grammar for `.mjs`, with no inference or parse-error fallback. There is no bundling, remote fetching, transpilation, source-map generation or image processing.

HTML uses [minify-html 0.18.1](https://docs.rs/minify-html/0.18.1/minify_html/). Enabling it accepts native whitespace, attribute and quote normalization even with exposed removal/omission controls off; the engine cannot disable all such changes. Inline CSS/JS minifiers remain disabled. Only `html = false` preserves exact rendered Tera bytes; calculate CSP hashes from final output.

[Regression checks](reference.md#regression-checks) compare processed output before installation. Enforced checks reject differences; warning-only checks report them and continue. Browser behavior needs separate testing.

## Trusted process boundary

[`hooks.rs`](../src/hooks.rs) is compiled **only with the non-default Cargo feature `hooks`**. Standard binaries validate declarations but reject nonempty effective hook lists before commands run or output changes. See [hook setup and inheritance](reference.md#trusted-build-hooks).

Commands run as literal argv using `std::process::Command`, with the canonical site root as cwd, inherited environment/stdout/stderr and closed stdin. There is no implicit shell or interpolation. Commands run with the current user's privileges, can access the network and secrets, and can cause nondeterministic, **nontransactional effects**. A hooks-enabled binary is not a sandbox. Review the effective commands before building an untrusted site.

Pre hooks run in declaration order before content/template/asset loading. A required failure stops later pre hooks and core generation. Post hooks match `success`, `failure` or `always` against that fixed primary result, including a pre-hook failure. A required post failure neither changes that result nor skips later eligible commands. Allowed failures warn; required failures return an error, keeping primary errors alongside post diagnostics. An installed site is **not undone**.

Unreadable or invalid configuration, failed profile resolution or unavailable requested features run **no hooks**, including failure hooks: there is no validated configuration to execute.

<details>
<summary>Hook environment and diagnostic disclosure</summary>

| Hook environment | Meaning |
| --- | --- |
| `REGEN_SITE` | Canonical absolute site root; also the command's working directory. |
| `REGEN_OUTPUT` | Intended absolute `dist/` or `review/` destination, even if generation fails. |
| `REGEN_PROFILE` | Selected profile, unchanged by `--review`. |
| `REGEN_STATUS` | `pre` for pre hooks; `success` or `failure` from the primary result for post hooks. |
| `REGEN_ERROR` | Primary failure details only for a post hook with `error_details = true`. At most 8192 UTF-8 bytes; NUL is escaped as literal `\0`. |
| `REGEN_ERROR_TRUNCATED` | `1` if those details were truncated, otherwise `0`; present only with `REGEN_ERROR`. |

Inherited `REGEN_ERROR` and `REGEN_ERROR_TRUNCATED` are removed before every command. Error details can contain source data or paths; disclose them only to trusted tools. [Hook configuration](reference.md#trusted-build-hooks), [security boundary](../SECURITY.md).

</details>

## Build sequence

[`build_with_options`](../src/lib.rs) runs these steps in order:

1. **Resolve.** Read strict configuration, resolve the selected profile and overrides, check that requested features are available and select the destination.
2. **Optional pre hooks.** Run configured commands only when the `hooks` feature is compiled in.
3. **Read and validate.** Load localized content, pair translations and derive routes; load Tera templates and inventory public files.
4. **Prepare the stage.** Check output ownership and recovery state; prepare/hash assets, copy public files and render pages with optional HTML processing. Write the sitemap and manifest. Run selected [regression checks](reference.md#regression-checks); enforced differences stop installation, while warning mode continues.
5. **Install.** Promote the completed stage into the selected destination and remove the previous tree.
6. **Optional post hooks.** Evaluate the primary result after success or a failure from steps 2–5. Resolution failure never reaches hooks.

`regen build --profile release --review` keeps release settings and hooks but redirects core output to `review/`, leaving `dist/` untouched. Hooks receive that destination but are not restricted to it.

## Invariants worth knowing

### Input gate

Strict fields, duplicate-key rejection and portable lowercase ASCII paths reject ambiguous inputs. Symlinks, special files, YAML includes, unknown content files, extra language directories and output collisions are rejected. Unicode belongs in content; generic `site` and `data` mappings accept JSON-compatible values. See [content](../src/content.rs), [paths](../src/files.rs) and [configuration](../src/config.rs).

### Translation routes

The relative filename under `pages/`, minus `.yaml`, is the translation ID, not a URL. An omitted `slug` defaults to the YAML basename without `.yaml`: `projects/one.yaml` uses `one`. Every language needs the same IDs and exactly one explicit empty-slug home page; `index.yaml` has no special homepage behavior. Slugs may differ; missing translations fail. [Content source](../src/content.rs).

### Final bytes and inventory

Sorted paths, explicit lengths and final bytes determine one SHA-256 digest for the asset tree. Relative asset paths survive; changing any asset changes the tree's hash. Other assets and `public/` remain unchanged. [Asset source](../src/assets.rs).

The manifest records profile/review metadata and sorted SHA-256 hashes and byte counts, excluding itself. Post hooks run after this inventory; modifying output invalidates its hashes and does not trigger rehashing. [Rendering source](../src/lib.rs).

## Hosting paths are not disk paths

For `site.base_url = "https://example.com/ReGen/"`, a German page may live at `/ReGen/de/ueber/` but is written to `dist/de/ueber/index.html` (or the same relative path in `review/`). Your host mounts the tree at `/ReGen/`; ReGen does not nest another `ReGen/` directory inside it.

The prefix is unencoded ASCII and case-preserving. It appears in `site_root`, `current_path`, `navigation`, `alternates`, `asset_base`, canonical and sitemap URLs; page IDs and disk paths stay unchanged. Profiles leave `site.base_url` unchanged. Templates also receive `build.profile` and `build.review`. Use the [template context](guide.md#tera-templates) for links and metadata. [Implementation](../src/lib.rs).

## Reproducible site output

**The deterministic core produces the same output paths and bytes from the same input paths and bytes, ReGen executable and effective build settings.** ReGen sorts file discovery and map iteration, keeps asset hashing deterministic and adds no timestamps. Its `group_by` replacement sorts group keys while preserving item order within each group. Manifest format `1` includes generator/version, profile/review and a sorted file inventory; the manifest excludes itself. [Implementation](../src/lib.rs).

Do not change source files or run another builder against the same site during core generation. Pre hooks may prepare inputs before they are loaded, but arbitrary hook effects are not covered by the deterministic guarantee: commands can read clocks and environment, access the network or alter files after hashing. ReGen does not take a filesystem snapshot or undo those effects. Rebuilding the executable with a different compiler, dependency set or linker may produce different results; reproducible core output does not mean identical binaries or archives across machines.

## Output ownership and interruption recovery

The selected output (`dist/` by default; `review/` when effective `review = true`) must be absent or carry a recognized `regen-manifest.json`. The marker checks ownership; it is **not an authenticated signature**. Built-in `dev` selects review; `--review` forces that destination without changing the selected profile or its settings. Existing `.regen-stage` or `.regen-previous` blocks every profile: ReGen never assumes leftovers are disposable. [Transaction source](../src/files.rs).

- **Before installation:** core preparation leaves old output untouched; ReGen attempts to remove this invocation's stage. Trusted hook effects are not covered.
- **Promotion fails:** ReGen attempts to restore the old tree.
- **Cleanup fails:** the new site may already be installed.
- **Required post hook fails:** the command reports an error, but an installed site remains installed; eligible later post hooks still run.
- **Interrupted build:** a stage may be incomplete. Inspect before changing directories.

Replacement uses two renames, so the selected output can briefly be absent. Serve a separate deployed copy if visitors must never see that gap. ReGen does not guarantee recovery from a power loss.

### Manual recovery

1. Stop all builders for the site; resolve processes blocking renames.
2. Identify the intended `dist/` or `review/`; inspect it, `.regen-stage` and `.regen-previous`. Recovery paths are shared across profiles: inspect the manifest's profile/review metadata and files, not just directory names. Back up needed files **outside the site root**. Reject unexpected symlinks or unrecognized contents; a familiar name is not permission to delete it.
3. Choose a trusted output from its manifest **and files**. If the intended output is absent, restore a known old `.regen-previous` to the correct destination. To roll back an existing output, move it outside the site first, then restore the previous tree. A stage may be incomplete.
4. After backup, clear only identified stale recovery output. Both reserved paths must be absent before rebuilding. With no trusted output, move questionable trees aside and rebuild from intact source into an absent selected output.
5. Run `regen build --site PATH` with the intended `--profile` if needed; inspect its manifest and serve the result before publishing.

## Trust and scope

Authors control templates, HTML, JavaScript and media. ReGen is not a hostile-upload sandbox, sanitizer, resource-quota system or metadata scrubber. Escaping is context-dependent; review `safe`, scripts, styles and URL attributes. Core generation makes no network requests or external process calls; [hooks](#trusted-process-boundary) can do both. Generated URLs may still cause browser requests, and build dependencies and their scripts run with the build user's privileges. See [security boundaries](../SECURITY.md).

The CLI omits built-in Markdown conversion, automatic translation, remote data fetching, plugins, watching, serving and deployment. Opt-in hooks are arbitrary external commands, not built-in implementations of those features. The documentation you are reading uses repository-only Rust preprocessing around `regen::build(&Path)`, not a second production input format.
