# Security policy

ReGen emits static files. Good luck exploiting the backend that isn't there. The relevant attack surface is **the builder, its dependencies and the release pipeline**, not a ReGen server listening on the internet.

## Runtime trust boundaries

- **Build trusted source in a private, unchanged working tree.** Templates and content are author-controlled; ReGen is not a hostile-upload sandbox or a concurrent-filesystem security boundary.
- **Paths and replacement are checked.** Input symlinks, special files, traversal, nonportable names and output collisions are rejected. Preparation failures preserve the previous site. The ownership manifest is a local marker, not authentication; interrupted replacement needs [manual recovery](docs/architecture.md#output-ownership-and-interruption-recovery).
- **Generated HTML can do what its author writes.** Scripts, remote URLs, `safe` markup and passthrough media are intentional capabilities. HTML autoescaping does not sanitize JavaScript, CSS or URL schemes. Review what you publish.
- **Generation is offline.** CSS imports and URLs are not fetched by the builder. Toolchain/dependency installation, release operations and visitors' browsers have separate network behavior.

## Relevant security reports

Report reproducible **unexpected file access or overwrite, unintended code execution, escaping failures in supported HTML contexts, or release/artifact-integrity bypasses**. A parser or dependency defect can matter without a web backend.

Authored scripts, deliberately unescaped HTML, a compromised host, and hostile concurrent source mutation are not failures of those boundaries. Resource-intensive templates need build isolation and external limits.

## Version scope and reporting

For **ReGen 1.0.0**, [report a vulnerability privately on GitHub](https://github.com/CritX-ai/ReGen/security/advisories/new). Confidential reporting is enabled. Do not disclose vulnerabilities in public issues or pull requests.

Include the version, OS/architecture, what went wrong, its impact and a minimal shareable reproduction. Keep credentials, personal paths and exploit details out of public discussions.

## Exact pins

Rust is **1.98.1**. Direct crates use exact versions; `Cargo.lock` records the dependency graph and checksums.

The dependency ecosystem is a mess. Pins prevent surprise upgrades — not pinned malware. Native archives include [third-party notices](docs/releasing.md#archive-contents-and-notices).

Download checksums detect changed bytes; they are not signatures. Native binaries are not code-signed or notarized.

[Testing](docs/testing.md) describes what is checked and what those checks leave out.
