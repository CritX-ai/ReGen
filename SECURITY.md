# Security policy

ReGen's attack surface is **the builder, its dependencies and the release pipeline**, plus the content you choose to publish.

## Runtime trust boundaries

- **Build trusted source in a private, unchanged working tree.** Templates, content and configured hook commands are author-controlled; ReGen is not a hostile-upload sandbox or a concurrent-filesystem security boundary. Pre hooks may prepare inputs, but those inputs must remain unchanged during core generation.
- **Paths and replacement are checked.** Input symlinks, special files, traversal, nonportable names and output collisions are rejected. Core preparation failures preserve the previous selected `dist/` or `review/`; cleanup or required post-hook errors can occur after new output is installed. The ownership manifest is a local marker, not authentication; interrupted replacement needs [manual recovery](docs/architecture.md#output-ownership-and-interruption-recovery). Trusted hook effects are outside this transaction.
- **Hooks require the non-default `hooks` feature and trusted configuration.** Standard binaries cannot execute hooks and reject nonempty effective hook lists. See [hook setup](docs/reference.md#trusted-build-hooks). Commands run with the build user's privileges, environment and credentials: they are not sandboxed or resource-limited and may access the network. Review inherited lists and use external isolation for untrusted projects.
- **Post-hook errors do not roll back an installed site.** Eligible hooks all run against the fixed primary build result. Changing generated files invalidates the already-written manifest; ReGen does not rehash those changes. Failure details are opt-in via `error_details`, bounded to 8192 UTF-8 bytes, and may disclose local paths or content. Inherited error-detail variables are cleared. Invalid configuration or unavailable requested features fail before any hook runs. See the [hook lifecycle](docs/architecture.md#trusted-process-boundary).
- **Generated HTML can do what its author writes.** Templates can include scripts, remote URLs and unescaped `safe` markup. HTML autoescaping does not sanitize JavaScript, CSS or URL schemes. Review what you publish.
- **Minification is not sanitization or a guarantee of equivalent behavior.** Enabling HTML accepts native whitespace and attribute normalization; enabled CSS/JS processing parses and serializes rather than copying bytes. Destructive opt-ins can discard notices or observable work. Review [minifier controls and risks](docs/reference.md#minifier-controls), understand the limits of [static regression checks](docs/reference.md#regression-checks), and compute CSP hashes from final output.
- **The generation core is offline and deterministic.** With no configured hooks, generation launches no external commands and makes no network requests, including CSS import/URL fetching. The same inputs, executable and effective settings determine core output; arbitrary hook effects are outside that guarantee. Toolchain/dependency installation, release operations and visitors' browsers have separate network behavior.

## Relevant security reports

Report reproducible **unexpected file access or overwrite, unintended code execution, escaping failures in supported HTML contexts, or release/artifact-integrity bypasses**.

Authored scripts, deliberately unescaped HTML, explicitly configured trusted commands, a compromised host, and hostile concurrent source mutation are not failures of those boundaries. Resource-intensive templates and hooks need build isolation and external limits.

## Version scope and reporting

[Report a vulnerability privately on GitHub](https://github.com/CritX-ai/ReGen/security/advisories/new). Confidential reporting is enabled. Do not disclose vulnerabilities in public issues or pull requests.

Include the ReGen version, OS/architecture, what went wrong, its impact and a minimal shareable reproduction. Keep credentials, personal paths and exploit details out of public discussions.

## Exact pins

The [Rust](https://www.rust-lang.org/) toolchain is pinned to **1.98.1**. Direct crates use exact versions; `Cargo.lock` records the dependency graph and checksums.

Pins prevent surprise upgrades, not malicious dependencies. Native archives include [third-party notices](docs/releasing.md#archive-contents-and-notices).

Download checksums detect changed bytes; they are not signatures. Native binaries are not code-signed or notarized.

[Testing](docs/testing.md) describes what is checked and what those checks leave out.
