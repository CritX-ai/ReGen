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

For **1.0.0**, the maintainer reviewed these disclosures and accepted them as **non-blocking**, using the upstream license declarations and best-effort notice collection. This release decision does not resolve the gaps or certify legal compliance:

- `escape-simd 0.1.0` and `json-escape-simd 3.1.2` declare MIT but omit complete upstream notices. Their JSON kernel is attributed to Apache-2.0-licensed `sonic-rs v0.5.5`; the supplemental record identifies the exact source and license, which are not included in those packages' retained notices.
- `parcel_sourcemap 2.1.1` declares MIT but provides no complete upstream license/copyright notice in the inspected source.
- `seahash 4.1.0` declares MIT; its immediate upstream successor adds MIT terms but still no copyright line.

The original declarations, available attribution, separate standard terms and unresolved provenance records remain available for downstream review. No copyright holder or year has been invented.
