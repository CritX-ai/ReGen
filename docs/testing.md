# Testing

ReGen's tests check generated sites and failed builds: predictable output, with the previous site preserved when preparation fails.

## Site regression checks

Binaries with a minifier include optional [static regression checks](reference.md#regression-checks), run before output replacement. Choose enforced checks, warning-only checks or an explicit opt-out; the reference covers settings and comparisons.

The checks are deliberately conservative: they can reject safe-but-different rewrites. They do not execute JavaScript, compute styles, measure layout or prove equivalent behavior. Keep application-specific CI and browser tests for those cases.

The repository regression below is separate. It exercises the actual documentation and bilingual poem under ordinary release defaults.

## Rust behavior

The suite builds small, real sites in isolated temporary directories. It checks:

- **Inputs:** configuration, localized YAML, page identity, URL rules and ambiguous or unsafe paths.
- **Output:** translated routes, template context, asset bytes, metadata and deterministic ordering.
- **Transitions:** rebuilds, deleted pages, ownership checks, preparation failures and interrupted replacement.
- **CLI:** the real executable, exit status, diagnostics and generated files.

Repeated builds are compared byte for byte. Failure cases check that the previous site remains intact or that recovery files are available after an interrupted replacement.

### Optional minifier features

Verification covers all sixteen combinations of `minify-html`, `minify-css`, `minify-js` and `hooks`. Native jobs cover the default build (three minifiers, no hooks) on six platforms; the feature matrix covers the other fifteen combinations. This includes hook rejection in standard builds, clearing inherited hooks in the selected profile and hook-enabled execution. Coverage runs also include ignored fault-injection tests.

[Published builds](releasing.md) include all three minifiers; hooks require a custom build.

### Minifier confidence and upstream evidence

The [minifier reference](reference.md#minifier-controls) covers settings and rewrite risks. Rust fixtures exercise selected transforms, JavaScript grammar, check failures, explicit opt-outs and preservation of installed output.

The regression `tests::real_site_release_minification_preserves_static_content` lives beside the documentation builder in [`examples/build_docs.rs`](../examples/build_docs.rs). It exercises the actual documentation and English/German poem sources.

It builds an unminified baseline (release with HTML/CSS/JS disabled), ordinary release output and a repeated release build, then checks:

- Repeated release builds produce byte-identical output maps.
- Expected documentation and bilingual poem HTML routes retain their full rendered text, markup and inline script bytes. Only the exact discovered content-hashed asset URL prefixes are normalized.
- Non-CSS assets and public files remain byte-identical; manifest metadata and inventories match the generated files.
- Baseline stylesheets match the authored documentation and poem CSS. With `minify-css`, both release stylesheets are processed and remain parseable, with the same ordered resource URLs, import conditions and native license notices. Without that feature, CSS stays byte-identical.

These checks compare **static output and repeatability**, separately from the [checks inside each build](reference.md#regression-checks). CSS comparisons use [Lightning CSS](https://lightningcss.dev/)'s parser. Even pretty-printed CSS can differ for equivalent input: `transform-origin: center` can become `50%`. Browser behavior and CSP need separate checks.

The upstream APIs, test collections and issue trackers explain the engines and their known limits:

| Engine / configured API version | API and policies | Regression corpus | Issues and change history |
| --- | --- | --- | --- |
| minify-html 0.18.1 | [`Cfg` API](https://docs.rs/minify-html/0.18.1/minify_html/struct.Cfg.html), [versioned option source](https://github.com/wilsonzlin/minify-html/blob/v0.18.1/minify-html/src/cfg/mod.rs) | [Versioned native regression tests](https://github.com/wilsonzlin/minify-html/blob/v0.18.1/minify-html/src/tests/mod.rs) | [Issues](https://github.com/wilsonzlin/minify-html/issues), [releases](https://github.com/wilsonzlin/minify-html/releases) |
| LightningCSS 1.0.0-alpha.72 | [Optimizer API](https://docs.rs/lightningcss/1.0.0-alpha.72/lightningcss/stylesheet/struct.MinifyOptions.html), [optimization policies](https://lightningcss.dev/minification.html) | [Rust regression tests](https://github.com/parcel-bundler/lightningcss/blob/master/src/lib.rs), [integration tests/data](https://github.com/parcel-bundler/lightningcss/tree/master/tests) | [Issues](https://github.com/parcel-bundler/lightningcss/issues), [releases](https://github.com/parcel-bundler/lightningcss/releases) |
| Oxc 0.95.0 | [Compression options](https://docs.rs/oxc_minifier/0.95.0/oxc_minifier/struct.CompressOptions.html), [tree-shaking options](https://docs.rs/oxc_minifier/0.95.0/oxc_minifier/struct.TreeShakeOptions.html) | [Minifier tests](https://github.com/oxc-project/oxc/tree/main/crates/oxc_minifier/tests) | [Issues](https://github.com/oxc-project/oxc/issues), [minifier changelog](https://github.com/oxc-project/oxc/blob/main/crates/oxc_minifier/CHANGELOG.md), [releases](https://github.com/oxc-project/oxc/releases) |

HTML links pin the version used by ReGen; other test and issue links follow upstream development. Check `Cargo.lock` when investigating a bug—a fix upstream may not be in your binary yet.

## Distribution checks

CI checks six native targets, Cargo installation, archive contents, dependency notices and independent documentation builds. Example downloads must match the same source package and appear in the release checksums. See [Distribution](releasing.md).

Linux container checks inspect the complete archived payload and exercise site generation, replacement and deterministic output without network access and with a read-only root. The site is a writable mount. These checks do not establish registry availability, anonymous pull access or compatibility with every host runtime.

## Coverage

The target is **100% code coverage, earned through meaningful tests** of behavior, boundaries and failure recovery. Assertions should catch plausible bugs, rather than merely visit lines.

[Open the latest main CI run](https://github.com/CritX-ai/ReGen/actions/workflows/build.yml?query=branch%3Amain) for results tied to a specific commit. Its **coverage** download contains JSON reports and annotated Rust source.

Coverage includes production `src/` code and the CLI, excluding tests, the documentation builder and dependencies. All-features and no-default-feature runs are merged, covering hook execution and rejection of unavailable features. Both include ignored fault-injection tests. The required threshold is zero uncovered executable lines and source regions; the region metric combines executions of generic instantiations that share a source span.

The reports use cargo-llvm-cov's [multi-run measurement](https://github.com/taiki-e/cargo-llvm-cov/blob/v0.9.1/README.md#merge-coverages-generated-under-different-test-conditions). JSON, annotated source and the badge share that measurement. Line and source-region coverage is not branch coverage; review the cases as well as the percentage.

The **Rust coverage** badge shows production-line coverage from the most recently deployed, verified `main` run. CI writes `coverage.json` with the measured count, source commit and run URL; [Shields.io](https://shields.io/) renders it. Incomplete coverage never rounds up to 100%. Failed or pending runs leave the previous badge in place, and caches can delay updates.

## Boundaries

Coverage is measured on Linux. Windows and macOS have their own native build and test jobs. Failure tests exercise unreadable files, failed writes and interrupted renames; they do not simulate every disk, operating-system or power-loss failure.

Keep site-specific browser and hosting checks alongside the Rust suite.
