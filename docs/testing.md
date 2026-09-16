# Testing

ReGen's tests check the generated site and what happens when a build fails. The aim is predictable output without losing the previous site.

## Rust behavior

The suite builds small, real sites in isolated temporary directories. It checks:

- **Inputs:** configuration, localized YAML, page identity, URL rules and ambiguous or unsafe paths.
- **Output:** translated routes, template context, asset bytes, metadata and deterministic ordering.
- **Transitions:** rebuilds, deleted pages, ownership checks, preparation failures and interrupted replacement.
- **CLI:** the real executable, exit status and generated files — not a mocked command parser.

Repeated builds are compared byte for byte. Failure cases check that the previous site remains intact or that recovery files are available after an interrupted replacement.

## Distribution checks

CI runs builds and executable checks on six native targets. It also installs the Cargo source package, checks archive contents and dependency notices, and compares independent builds of this documentation.

ReGen and the generated sites do not need Python or the project's test tools.

## Coverage

[Open the latest main CI run](https://github.com/CritX-ai/ReGen/actions/workflows/build.yml?query=branch%3Amain) for results tied to a specific commit. Its **coverage** download contains JSON reports and annotated Rust source.

Coverage includes production `src/` code and the CLI, but excludes tests, the documentation builder and dependencies. CI requires every executable source line and region to be exercised. The source-region count combines executions across binaries and generic instantiations; LLVM's separate native summary uses a different aggregation and can show lower totals. These are line and region measurements, not branch coverage.

The **Rust coverage** badge in the README and documentation shows production-line coverage from the most recently deployed, verified `main` CI run. CI generates `coverage.json` from the measured line counts, including the source commit and run URL. Documentation deployment consumes that same run's coverage artifact only after every verification job succeeds. Shields.io renders the percentage; incomplete coverage is never rounded up to 100%. A failed or pending run leaves the previous badge in place, and Shields.io caching can delay updates.

## Boundaries

Coverage is measured on Linux. Windows and macOS have their own native build and test jobs. Failure tests exercise unreadable files, failed writes and interrupted renames; they do not simulate every disk, operating-system or power-loss failure.

Browser checks cover the documentation's layout and controls separately. A passing test run does not guarantee that every template or hosting configuration will work.
