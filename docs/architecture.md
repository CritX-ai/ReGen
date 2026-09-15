# Architecture

Rust reads a local source tree and writes a static site. The diagrams trace the build; the sections below explain how the modules fit together and what happens when a build fails.

## Crate composition

One Cargo package produces the `regen` binary and the `regen` library. Only `build(&Path)` and its `BuildSummary` result form the public Rust API. The four implementation modules stay private so their internal models can change without becoming a second configuration interface.

### CLI boundary

[`main.rs`](../src/main.rs) parses arguments and prints results. It calls the same library function available to Rust applications, so command-line and embedded use follow the same build process.

### Build coordinator

[`lib.rs`](../src/lib.rs) loads and validates inputs, prepares assets and template context, renders pages, copies public files, writes the sitemap and manifest, then installs the finished site. HTML/XML escaping and ordered grouping live here too. It controls the build sequence; the filesystem module handles writes and replacement.

### Input model

| Module | Responsibility | Why separate |
| --- | --- | --- |
| [`config.rs`](../src/config.rs) | Deserialize TOML; validate languages, URL prefixes and source paths. | Validate once; downstream consumers use a coherent configuration. |
| [`content.rs`](../src/content.rs) | Inspect the locale layout, decode YAML, pair page IDs and derive routes. | Translation and route rules belong to the content model, not the renderer or CLI. |

Content consumes the configuration model and the shared filesystem inventory. Neither module renders templates or installs output.

### Asset pipeline

[`assets.rs`](../src/assets.rs) minifies standalone CSS and copies other assets while hashing their bytes. It computes one digest for the whole asset tree. Keeping this work separate lets asset tests run without rendering pages. It reuses buffers rather than loading entire media files into memory.

### Filesystem boundary

[`files.rs`](../src/files.rs) checks paths, lists files in a stable order, limits text reads and prevents accidental overwrites. Configuration, content, assets and the coordinator share these helpers.

Its `Transaction` tracks the staged output and previous site so a failed build can either restore the old site or leave files available for manual recovery.

### Outside the runtime

- [`examples/build_docs.rs`](../examples/build_docs.rs) preprocesses this repository’s Markdown into normal site inputs, then calls the public API. Its Markdown dependency is development-only.
- [`tests/generation.rs`](../tests/generation.rs) checks builds and the real CLI. [`tests/unit/`](../tests/unit) exercises narrow byte, escaping and transaction boundaries inside their owning modules.

## Invariants worth knowing

- **Identity is not a URL.** The relative filename under `pages/`, minus `.yaml`, is the translation ID. Every configured language needs the same IDs and exactly one empty-slug home page. Slugs may differ; missing translations fail. [Content source](../src/content.rs).
- **Reject ambiguity.** Strict typed fields, duplicate-key rejection, portable lowercase ASCII input paths, no symlinks or special files, no YAML includes. Unknown content files, extra language directories and output collisions fail. Unicode belongs in content; generic `site` and `data` mappings accept JSON-compatible values. [Content](../src/content.rs), [paths](../src/files.rs), [configuration](../src/config.rs).
- **One asset digest.** Sorted paths, explicit lengths and optimized bytes determine the whole tree’s SHA-256. Standalone CSS is minified; JavaScript and media bytes are preserved. Relative asset paths survive; any changed asset invalidates the tree. No import bundling, remote fetching, JavaScript transformation or image processing. [Asset source](../src/assets.rs).
- **Stage before replacement.** Public files are copied unchanged. Tera renders localized HTML, then HTML is minified with comments retained. Inline CSS/JavaScript are not separately minified, but surrounding whitespace can be normalized; do not assume their source bytes survive for CSP hashes. The sitemap is written before the manifest; the manifest records sorted SHA-256 hashes and byte counts, excluding itself. [Rendering source](../src/lib.rs).

## Hosting paths are not disk paths

For `site.base_url = "https://example.com/ReGen/"`, a German page may live at `/ReGen/de/ueber/` but is written to `dist/de/ueber/index.html`. Your host mounts the tree at `/ReGen/`; ReGen does not nest another `ReGen/` directory inside it.

The prefix is unencoded ASCII and case-preserving. It appears in `site_root`, `current_path`, `navigation`, `alternates`, `asset_base`, canonical and sitemap URLs — not page IDs or disk paths. Use the [template context](guide.md#tera-templates), not hardcoded root links. [Implementation](../src/lib.rs).

## Reproducible site output

**The same source paths and bytes, built with the same ReGen executable, produce the same output paths and bytes.** ReGen sorts file discovery and map iteration, keeps asset hashing deterministic and adds no timestamps. Its `group_by` replacement sorts group keys while preserving item order within each group. [Implementation](../src/lib.rs).

Do not change source files or run another builder against the same site during a build. ReGen does not take a filesystem snapshot. Rebuilding the executable with a different compiler, dependency set or linker may produce different results; reproducible site output does not mean identical binaries or archives across machines.

## Output ownership and interruption recovery

`dist/` must be absent or carry a recognized `regen-manifest.json`. The marker checks ownership; it is **not an authenticated signature**. Existing `.regen-stage` or `.regen-previous` blocks a build. ReGen never assumes leftovers are disposable. [Transaction source](../src/files.rs).

- **Before installation:** old output stays untouched; ReGen attempts to remove this invocation’s stage.
- **Promotion fails:** ReGen attempts to restore the old tree.
- **Cleanup fails:** the new site may already be installed.
- **Interrupted build:** a stage may be incomplete. Inspect before changing directories.

Replacement uses two renames, so `dist/` can briefly be absent. Serve a separate deployed copy if visitors must never see that gap. ReGen does not guarantee recovery from a power loss.

### Manual recovery

1. Stop all builders for the site; resolve processes blocking renames.
2. Inspect `dist/`, `.regen-stage` and `.regen-previous`. Back up needed files **outside the site root**. Reject unexpected symlinks or unrecognized contents; a familiar name is not permission to delete it.
3. Choose a trusted output from its manifest **and files**. If `dist/` is absent, restore a known old `.regen-previous`. To roll back an existing `dist/`, move it outside the site first, then restore the previous tree. A stage may be incomplete.
4. After backup, clear only identified stale recovery output. Both reserved paths must be absent before rebuilding. With no trusted output, move questionable trees aside and rebuild from intact source into an absent `dist/`.
5. Run `regen build --site PATH`; inspect its manifest and serve the result before publishing.

## Trust and scope

Authors control templates, HTML, JavaScript and media. ReGen is not a hostile-upload sandbox, sanitizer, resource-quota system or metadata scrubber. Escaping is context-dependent; review `safe`, scripts, styles and URL attributes. Offline generation does not prevent generated URLs from making browser requests. Build dependencies and their scripts run with the build user’s privileges. [Security boundaries](../SECURITY.md).

The CLI omits Markdown conversion, automatic translation, remote data, plugins, watching, serving and deployment. The documentation you are reading uses repository-only Rust preprocessing around `regen::build(&Path)`, not a second production input format.
