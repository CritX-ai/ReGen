//! Offline, deterministic generation of multilingual static sites.
//!
//! [`build`] coordinates validated configuration and translations, asset preparation,
//! rendering, and transactional output replacement. Input policy and filesystem
//! recovery live in separate modules so rendering cannot bypass those boundaries.
//! Inputs must remain unchanged during generation. Configured hooks and templates
//! are trusted local code, not sandboxed content; hooks can have external effects.

mod assets;
mod config;
mod content;
mod files;
#[cfg(feature = "hooks")]
mod hooks;
#[cfg(feature = "minify-html")]
mod html;
#[cfg(test)]
#[path = "../tests/common/mod.rs"]
mod test_support;

use anyhow::{Context as _, Result, ensure};
use config::Config;
use config::ResolvedBuild;
pub use config::{BuildOptions, RegressionCheckMode};
use content::{Content, route};
use files::{PathCases, Transaction, files, output_file, read_text, reject_symlinks, write_output};
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
    /// Installed `dist/` or review-only `review/` beneath the canonical site root.
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

/// Build a multilingual site using its configured default profile.
///
/// Reads required `templates/` and `content/` trees and optional `assets/` and
/// `public/` trees beneath `root`. Replaces `dist/` (or `review/` for review profiles)
/// only when owned by ReGen; unrelated output and interrupted state are preserved.
///
/// Hooks require the non-default Cargo feature `hooks` and explicit configuration.
/// Standard builds cannot execute hooks. Enabled hooks run trusted commands with
/// the caller's privileges; without hooks, generation is process-free and offline.
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
/// Required post-hook failures also return an error **after** successful output
/// installation. Hooks must not mutate generated output and invalidate its manifest.
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
    build_with_options(root, &BuildOptions::default())
}

/// Build with an explicit profile, review destination, and minification overrides.
///
/// CLI/API overrides win over inherited project configuration. `release` writes
/// `dist/` and compact-prints CSS when compiled; `dev` writes unminified `review/`.
/// Setting [`BuildOptions::review`] forces `review/` while retaining the selected
/// profile's settings and hooks. HTML/JS processing and destructive optimizations
/// require explicit opt-in. Hook execution and recovery follow [`build`]'s contract.
///
/// # Errors
///
/// In addition to [`build`]'s errors, rejects unknown profiles, inheritance cycles
/// and effective hooks or minification unavailable in this executable's features.
pub fn build_with_options(root: &Path, options: &BuildOptions<'_>) -> Result<BuildSummary> {
    reject_symlinks(root)?;
    let root = fs::canonicalize(root).context("site directory does not exist")?;
    let config = Config::load(&root)?;
    let settings = config.resolve_build(options)?;
    #[cfg(not(feature = "hooks"))]
    {
        build_site(&root, &config, &settings)
    }
    #[cfg(feature = "hooks")]
    {
        let output = root.join(if settings.review { "review" } else { "dist" });
        let built = hooks::run_pre(&settings, &root, &output)
            .and_then(|()| build_site(&root, &config, &settings));
        let post = hooks::run_post(&settings, &root, &output, built.as_ref().err());
        match (built, post) {
            (Ok(summary), Ok(())) => Ok(summary),
            (Err(error), Ok(())) => Err(error),
            (Err(error), Err(post)) => {
                Err(error.context(format!("post-build hooks also failed: {post:#}")))
            }
            (Ok(_), Err(post)) => Err(post.context(format!(
                "site installed at {}, but post-build hooks failed",
                output.display()
            ))),
        }
    }
}

fn build_site(root: &Path, config: &Config, settings: &ResolvedBuild) -> Result<BuildSummary> {
    let mut content = Content::load(root, config)?;
    let template_root = root.join("templates");
    let mut templates = Vec::new();
    for source in files(&template_root, false)? {
        ensure!(
            source
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("html")),
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
    // Every loaded template is HTML, including mixed-case filename extensions.
    tera.autoescape_on([""]);
    tera.set_escape_fn(escape_html);
    tera.register_filter("group_by", ordered_group_by);
    tera.add_raw_templates(templates)
        .map_err(|error| anyhow::anyhow!("cannot load templates: {error}"))?;
    let public_root = root.join("public");
    let public = files(&public_root, true)?;
    let transaction = Transaction::begin(root, if settings.review { "review" } else { "dist" })?;
    let stage = transaction.stage();
    let assets = assets::prepare(
        root,
        stage,
        &settings.minify,
        settings.minify_assets,
        settings.regression_checks,
    )?;
    let asset_base = format!("{}{}", config.base_path, assets.base);
    let site_root = format!("{}/", config.base_path);
    copy_public(&public_root, public, stage, &mut content.output_cases)?;
    drop(content.output_cases);
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
        context.insert(
            "build",
            &serde_json::json!({"profile": settings.profile, "review": settings.review}),
        );
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
            #[cfg(feature = "minify-html")]
            let bytes = if settings.minify.html {
                let optimized = html::optimize(&html, &settings.minify.html_options);
                if settings.regression_checks != RegressionCheckMode::Off {
                    settings.regression_checks.check(
                        html::check_regression(&html, &optimized),
                        "HTML",
                        format_args!("{} page {id}", language.code),
                        "minification changed the parsed HTML tree, text or attributes",
                    )?;
                }
                std::borrow::Cow::Owned(optimized)
            } else {
                std::borrow::Cow::Borrowed(html.as_bytes())
            };
            #[cfg(not(feature = "minify-html"))]
            let bytes = std::borrow::Cow::Borrowed(html.as_bytes());
            write_output(stage, &relative, &bytes)?;
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
    write_manifest(stage, &settings.profile, settings.review)?;
    transaction.commit().map(|output| BuildSummary {
        pages: page_count,
        languages: config.languages.len(),
        assets: assets.count,
        output,
    })
}

/// Copy the validated public inventory without transforming its bytes.
fn copy_public(
    root: &Path,
    sources: Vec<PathBuf>,
    stage: &Path,
    output_cases: &mut PathCases,
) -> Result<()> {
    for source in sources {
        let name = source
            .strip_prefix(root)
            .expect("files returns descendants")
            .to_str()
            .expect("files validates UTF-8 paths")
            .replace(std::path::MAIN_SEPARATOR, "/");
        ensure!(
            !name
                .split('/')
                .next()
                .is_some_and(|part| part.eq_ignore_ascii_case("assets")),
            "public/assets is reserved for versioned assets"
        );
        output_cases.insert(&name)?;
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
struct Manifest<'a> {
    format: u32,
    generator: &'static str,
    version: &'static str,
    profile: &'a str,
    review: bool,
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
fn write_manifest(stage: &Path, profile: &str, review: bool) -> Result<()> {
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
        profile,
        review,
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
