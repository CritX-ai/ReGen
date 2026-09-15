# Distribution

**ReGen 1.0.0** · Cargo package `regen-ssg` · executable `regen` · tag `v1.0.0`

## Cargo

```sh
cargo install regen-ssg --version 1.0.0 --locked
regen --version
```

Compiles with Rust **1.98.1** and a native linker. Cargo installs the executable; the [example](../examples/minimal) lives in the source and native archives.

## Native binaries

[Download from GitHub Releases](https://github.com/CritX-ai/ReGen/releases/latest). Verify `SHA256SUMS`, unpack, and put `regen` on your `PATH`.

| System | CPU | Target | Archive |
| --- | --- | --- | --- |
| Linux | x86_64 | `x86_64-unknown-linux-gnu` | `.tar.gz` |
| Linux | ARM64 | `aarch64-unknown-linux-gnu` | `.tar.gz` |
| macOS | Intel | `x86_64-apple-darwin` | `.tar.gz` |
| macOS | Apple Silicon | `aarch64-apple-darwin` | `.tar.gz` |
| Windows | x86_64 | `x86_64-pc-windows-msvc` | `.zip` |
| Windows | ARM64 | `aarch64-pc-windows-msvc` | `.zip` |

Archives are named `regen-1.0.0-TARGET`. Linux builds use GNU libc; Windows builds use MSVC and may need the matching Visual C++ runtime.

## Archive contents and notices

- The executable, documentation, changelog and bilingual example.
- Project license and lockfile; original dependency and Rust notices.
- Per-target dependency inventories and a separate license archive.
- Release-wide `SHA256SUMS`, `DEPENDENCIES.json`, source crate and candidate receipts.

Third-party terms remain intact. [Notice provenance](../licenses/supplemental.json) records retained upstream sources and the four known notice-source gaps; inventories expose them as `notice_source_gap`.
