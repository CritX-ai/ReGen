//! Offline, deterministic generation of multilingual static sites.
//!
//! [`build`] coordinates validated configuration and translations, asset preparation,
//! rendering, and transactional output replacement. Input policy and filesystem
//! recovery live in separate modules so rendering cannot bypass those boundaries.
//! Inputs and the site directory must remain unchanged for the duration of a build;
//! templates are trusted local code, not sandboxed content.

mod assets;
mod config;
mod content;
mod files;

use anyhow::{Context as _, Result, ensure};
use config::Config;
use content::{Content, route};
use files::{Transaction, files, output_file, read_text, reject_symlinks, write_output};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tera::{Context, Tera};

/// Counts and destination from a successfully installed output tree.
#[derive(Debug)]
pub struct BuildSummary {
    /// Rendered pages across all configured languages.
    pub pages: usize,
    /// Configured languages, each with the same set of translated page IDs.
    pub languages: usize,
    /// Source files in `assets/`, excluding unchanged files copied from `public/`.
    pub assets: usize,
    /// Installed `dist/` directory beneath the canonicalized site root.
    pub output: PathBuf,
}

#[derive(Serialize)]
struct Alternate<'a> {
    code: &'a str,
    name: &'a str,
    direction: &'a str,
    path: String,
    url: String,
}

#[derive(Serialize)]
struct Navigation<'a> {
    id: &'a str,
    title: &'a str,
    path: String,
    url: String,
}

/// Build an optimized multilingual site from `regen.toml`, Tera templates, and YAML.
///
/// Reads required `templates/` and `content/` trees and optional `assets/` and
/// `public/` trees beneath `root`. Replaces `dist/` only when its manifest identifies
/// it as ReGen output; unrelated output and interrupted build state are preserved.
///
/// No network, process execution, environment expansion, or current timestamp
/// participates in generation. The caller must keep the site tree unchanged until
/// this function returns. This is not a sandbox for hostile templates.
///
/// # Errors
///
/// Returns an error for invalid or unreadable inputs, unsafe paths, conflicting
/// routes, rendering or optimization failures, unrecognized output ownership, or
/// filesystem failures. Input and rendering errors leave the previous site intact.
///
/// Output promotion attempts to restore the previous site on failure. A failed
/// rollback or interrupted build requires manual recovery of `.regen-previous`
/// and `.regen-stage`. An error removing the backup can occur **after** the new
/// site is installed; the error identifies this case.
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// let summary = regen::build(Path::new("my-site"))?;
/// println!("Built {} pages into {}", summary.pages, summary.output.display());
/// # Ok::<(), anyhow::Error>(())
/// ```
pub fn build(root: &Path) -> Result<BuildSummary> {
    reject_symlinks(root)?;
    let root = fs::canonicalize(root).context("site directory does not exist")?;
    let config = Config::load(&root)?;
    let content = Content::load(&root, &config)?;
    let template_root = root.join("templates");
    let mut templates = Vec::new();
    for source in files(&template_root, false)? {
        ensure!(
            source
                .extension()
                .is_some_and(|extension| extension == "html"),
            "only .html Tera templates are supported: {}",
            source.display()
        );
        let name = source
            .strip_prefix(&template_root)
            .expect("files returns descendants")
            .to_str()
            .expect("files validates UTF-8 paths")
            .replace(std::path::MAIN_SEPARATOR, "/");
        templates.push((name, read_text(&source)?));
    }
    let mut tera = Tera::new();
    tera.autoescape_on([".html"]);
    tera.set_escape_fn(escape_html);
    tera.register_filter("group_by", ordered_group_by);
    tera.add_raw_templates(templates)
        .map_err(|error| anyhow::anyhow!("cannot load templates: {error}"))?;
    let public_root = root.join("public");
    let public = files(&public_root, true)?;
    let transaction = Transaction::begin(&root)?;
    let stage = transaction.stage();
    let assets = assets::prepare(&root, stage)?;
    let asset_base = format!("{}{}", config.base_path, assets.base);
    let site_root = format!("{}/", config.base_path);
    copy_public(&public_root, public, stage)?;
    let mut html_config = minify_html::Cfg::new();
    // Preserve license/attribution comments. CSS assets are optimized separately;
    // inline CSS and JS retain their original semantics and attribution.
    html_config.keep_comments = true;
    html_config.keep_closing_tags = true;
    html_config.keep_html_and_head_opening_tags = true;
    let mut sitemap = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\" xmlns:xhtml=\"http://www.w3.org/1999/xhtml\">\n",
    );
    let mut page_count = 0;
    for language in &config.languages {
        let localized = &content.localized[&language.code];
        let navigation: Vec<_> = localized
            .pages
            .iter()
            .map(|(id, page)| {
                let path = route(&language.code, &config.site.default_language, &page.slug);
                Navigation {
                    id,
                    title: &page.title,
                    url: format!("{}{path}", config.site.base_url),
                    path: format!("{}{path}", config.base_path),
                }
            })
            .collect();
        let mut context = Context::new();
        context.insert("project", &config.site);
        context.insert("site", &localized.site);
        context.insert("language", language);
        context.insert("navigation", &navigation);
        context.insert("asset_base", &asset_base);
        context.insert("site_root", &site_root);
        for (id, page) in &localized.pages {
            let path = route(&language.code, &config.site.default_language, &page.slug);
            let url = format!("{}{path}", config.site.base_url);
            let alternates: Vec<_> = config
                .languages
                .iter()
                .map(|other| {
                    // Content::load guarantees every locale contains the same page IDs.
                    let translated = &content.localized[&other.code].pages[id];
                    let path = route(&other.code, &config.site.default_language, &translated.slug);
                    Alternate {
                        code: &other.code,
                        name: &other.name,
                        direction: &other.direction,
                        url: format!("{}{path}", config.site.base_url),
                        path: format!("{}{path}", config.base_path),
                    }
                })
                .collect();
            context.insert("page", page);
            context.insert("current_path", &format!("{}{path}", config.base_path));
            context.insert("canonical_url", &url);
            context.insert("alternates", &alternates);
            let html = tera.render(&page.template, &context).map_err(|error| {
                anyhow::anyhow!("cannot render {} page {id}: {error}", language.code)
            })?;
            let relative = format!("{}index.html", path.trim_start_matches('/'));
            write_output(
                stage,
                &relative,
                &minify_html::minify(html.as_bytes(), &html_config),
            )?;
            sitemap.push_str("<url><loc>");
            sitemap.push_str(&xml_escape(&url));
            sitemap.push_str("</loc>");
            for alternate in &alternates {
                sitemap.push_str(&format!(
                    "<xhtml:link rel=\"alternate\" hreflang=\"{}\" href=\"{}\"/>",
                    alternate.code,
                    xml_escape(&alternate.url)
                ));
            }
            sitemap.push_str("</url>\n");
            page_count += 1;
        }
    }
    sitemap.push_str("</urlset>\n");
    write_output(stage, "sitemap.xml", sitemap.as_bytes())?;
    write_manifest(stage)?;
    transaction.commit().map(|output| BuildSummary {
        pages: page_count,
        languages: config.languages.len(),
        assets: assets.count,
        output,
    })
}

/// Copy the validated public inventory without transforming its bytes.
fn copy_public(root: &Path, sources: Vec<PathBuf>, stage: &Path) -> Result<()> {
    for source in sources {
        let name = source
            .strip_prefix(root)
            .expect("files returns descendants")
            .to_str()
            .expect("files validates UTF-8 paths")
            .replace(std::path::MAIN_SEPARATOR, "/");
        ensure!(
            name.split('/').next() != Some("assets"),
            "public/assets is reserved for versioned assets"
        );
        std::io::copy(&mut File::open(&source)?, &mut output_file(stage, &name)?)?;
    }
    Ok(())
}

/// Tera's grouping filter still iterates an internal HashMap even with
/// preserve_order. Group by ordered keys instead; keep input order within groups.
fn ordered_group_by(
    values: &[tera::Value],
    kwargs: tera::Kwargs,
    _: &tera::State,
) -> tera::TeraResult<tera::Map> {
    use std::collections::btree_map::Entry;
    use tera::value::Key;

    if values.is_empty() {
        return Ok(tera::Map::new());
    }
    let attribute = kwargs.must_get::<&str>("attribute")?;
    let mut groups: BTreeMap<&tera::Value, (Key<'static>, Vec<tera::Value>)> = BTreeMap::new();
    for value in values {
        let key = value.get_from_path(attribute).ok_or_else(|| {
            tera::Error::message(format!("group_by: missing attribute {attribute}"))
        })?;
        if key.is_none() {
            continue;
        }
        // Value ordering equates 1 and 1.0. Validate every value before lookup,
        // including keys that compare equal to an existing, valid group.
        if key.as_str().is_none() && !key.is_bool() && key.as_i128().is_none() {
            return Err(tera::Error::message(
                "group_by: keys must be strings, integers or booleans",
            ));
        }
        let entries = match groups.entry(key) {
            Entry::Vacant(entry) => {
                let owned_key = if let Some(text) = key.as_str() {
                    Key::String(text.into())
                } else if key.is_bool() {
                    Key::Bool(*key == tera::Value::from(true))
                } else {
                    Key::I128(key.as_i128().expect("integer key validated before lookup"))
                };
                &mut entry.insert((owned_key, Vec::new())).1
            }
            Entry::Occupied(entry) => &mut entry.into_mut().1,
        };
        entries.push(value.clone());
    }
    Ok(groups
        .into_values()
        .map(|(key, values)| (key, values.into()))
        .collect())
}

/// Escape interpolated text for HTML text and quoted attributes, not raw markup.
fn escape_html(input: &str, output: &mut dyn Write) -> std::io::Result<()> {
    let mut utf8 = [0; 4];
    for character in input.chars() {
        let escaped: &[u8] = match character {
            '&' => b"&amp;",
            '<' => b"&lt;",
            '>' => b"&gt;",
            '"' => b"&quot;",
            '\'' => b"&#x27;",
            _ => character.encode_utf8(&mut utf8).as_bytes(),
        };
        output.write_all(escaped)?;
    }
    Ok(())
}

/// Escape sitemap text and attributes independently of HTML minification.
fn xml_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[derive(Serialize)]
struct FileDigest {
    sha256: String,
    bytes: u64,
}

/// Deterministic output inventory and the ownership marker for later replacements.
#[derive(Serialize)]
struct Manifest {
    format: u32,
    generator: &'static str,
    version: &'static str,
    files: BTreeMap<String, FileDigest>,
}

fn file_digest(mut input: impl Read, buffer: &mut [u8]) -> std::io::Result<FileDigest> {
    let mut digest = Sha256::new();
    let mut bytes = 0_u64;
    loop {
        let count = input.read(buffer)?;
        if count == 0 {
            break;
        }
        bytes += count as u64;
        digest.update(&buffer[..count]);
    }
    Ok(FileDigest {
        sha256: format!("{:x}", digest.finalize()),
        bytes,
    })
}

/// Inventory final bytes before writing the marker, which cannot hash itself.
fn write_manifest(stage: &Path) -> Result<()> {
    let mut entries = BTreeMap::new();
    let mut buffer = [0_u8; 65536];
    for source in files(stage, false)? {
        let relative = source
            .strip_prefix(stage)
            .expect("files returns descendants")
            .to_str()
            .expect("files validates UTF-8 paths")
            .replace(std::path::MAIN_SEPARATOR, "/");
        let digest = File::open(&source).and_then(|file| file_digest(file, &mut buffer))?;
        entries.insert(relative, digest);
    }
    let manifest = Manifest {
        format: 1,
        generator: "ReGen",
        version: env!("CARGO_PKG_VERSION"),
        files: entries,
    };
    let mut bytes =
        serde_json::to_vec_pretty(&manifest).expect("manifest contains only strings and integers");
    bytes.push(b'\n');
    write_output(stage, "regen-manifest.json", &bytes)
}

#[cfg(test)]
#[path = "../tests/unit/rendering.rs"]
mod tests;
