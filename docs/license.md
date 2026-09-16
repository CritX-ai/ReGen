# License

**WTFPL covers ReGen's original code and documentation — not every file distributed with the project.** Fonts, third-party icons, dependency code and the CritX logo have the separate terms described below.

## ReGen code and documentation

Except for the material identified below or carrying its own notice, ReGen's original code and documentation are licensed under [WTFPL version 2](../LICENSE). You may modify and redistribute that material under those terms.

## Rust dependencies

Rust dependencies retain their own licenses. Native ReGen executables incorporate third-party code; downloading those dependencies through Cargo does not remove their redistribution conditions. Native archives include `THIRD-PARTY-NOTICES.txt`, license files and a dependency inventory. The inventory is conservative and may include build, development or inactive dependencies; it is not a list of code proven to be linked into the executable.

Cargo source packages obtain dependencies separately. Generated HTML does not acquire the generator's dependency licenses merely because ReGen produced it. Assets included in a generated site still retain their own terms.

### Dependency notice provenance

Third-party terms remain intact. [Notice provenance](../licenses/supplemental.json) records retained upstream sources and the four known notice-source gaps; inventories expose them as `notice_source_gap`.
Some upstream sources omit original license or copyright files. Retained declarations and best-effort notice collection do not resolve these gaps or certify legal compliance:

- [`escape-simd 0.1.0`](https://crates.io/crates/escape-simd/0.1.0) and [`json-escape-simd 3.1.2`](https://crates.io/crates/json-escape-simd/3.1.2) declare MIT but omit complete upstream notices. Their JSON kernel is attributed to Apache-2.0-licensed [`sonic-rs v0.5.5`](https://github.com/cloudwego/sonic-rs/tree/v0.5.5); the supplemental record identifies the exact source and license, which are not included in those packages' retained notices.
- [`parcel_sourcemap 2.1.1`](https://crates.io/crates/parcel_sourcemap/2.1.1) declares MIT but provides no complete upstream license/copyright notice in the inspected source.
- [`seahash 4.1.0`](https://crates.io/crates/seahash/4.1.0) declares MIT; its immediate upstream successor adds MIT terms but still no copyright line.

The original declarations, available attribution, separate standard terms and unresolved provenance records remain available for downstream review.

## Fonts

The bundled **[Space Grotesk](https://github.com/floriankarsten/space-grotesk)**, **[IBM Plex Sans and Mono](https://github.com/IBM/plex)** font files are licensed under the **SIL Open Font License 1.1**, not WTFPL.

- [Space Grotesk copyright notice and OFL](../site/assets/fonts/space-grotesk-ofl.txt)
- [IBM Plex copyright notice and OFL](../site/assets/fonts/ibm-plex-ofl.txt)
- [Bundled font provenance and file hashes](../site/assets/fonts/provenance.json)

Retain the applicable copyright notices and OFL when redistributing the fonts. The font files may not be relicensed under WTFPL. The OFL does not apply to a document merely because it uses these fonts.

## GitHub icon

[`site/assets/github-mark.svg`](../site/assets/github-mark.svg) is the `mark-github-16` icon from [Primer Octicons](https://github.com/primer/octicons), licensed under the [MIT License](https://github.com/primer/octicons/blob/main/LICENSE), not WTFPL. Its copyright notice and full MIT license are embedded in the SVG metadata and must be retained when redistributing it.

The MIT copyright license does not grant GitHub trademark rights or imply endorsement. Use of GitHub branding remains subject to applicable trademark law and [GitHub's brand guidance](https://brand.github.com/foundations/logo).

## CritX branding

**The CritX logo in [`site/assets/critx-mark.svg`](../site/assets/critx-mark.svg) is expressly excluded from WTFPL.** All rights are reserved except for this limited permission:

You may reproduce, distribute and display the unchanged CritX logo **only as part of unmodified copies of ReGen and its accompanying documentation**, with this notice retained.

**Modified copies of ReGen or its documentation must remove or replace the CritX logo and must not state or imply affiliation with or endorsement by CritX.** Including the logo in a modified distribution, reusing it separately, modifying it, or using it as another project's branding requires separate prior written permission from the rights holder.

These restrictions apply to CritX branding, not to ReGen's WTFPL-licensed code and documentation. No trademark rights are granted. Uses permitted by applicable law remain unaffected.
