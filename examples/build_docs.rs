//! Repository-only preparation of Markdown documentation and the localized poem.
//!
//! Enumerated authored inputs become temporary YAML/Tera sites, then the production
//! library builds both outputs. Markdown conversion and repository-link rewriting
//! stay here rather than expanding the generator's input contract or dependencies.
//! The checkout and its raw HTML/templates are trusted and must remain unchanged.
//!
//! Requires `--base-url` and a new `--output` directory. This prepares static files
//! only; it does not publish them. Both builds finish before claiming that directory,
//! but a final copy failure retains incomplete output for inspection and manual removal.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use clap::Parser as _;
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;
use walkdir::WalkDir;

const REPOSITORY_URL: &str = "https://github.com/CritX-ai/ReGen";
const DOCUMENTATION_URL: &str = "https://regen.critx.ai/";
/// Complete set of authored documents that `site/pages.json` must route exactly once.
const SOURCES: &[&str] = &[
    "README.md",
    "docs/guide.md",
    "docs/reference.md",
    "docs/architecture.md",
    "docs/deployment.md",
    "docs/releasing.md",
    "docs/testing.md",
    "SECURITY.md",
    "CONTRIBUTING.md",
    "CHANGELOG.md",
    "docs/license.md",
];
// Deliberately enumerate public authored inputs, never a checkout or an old dist tree.
const POEM_INPUTS: &[&str] = &[
    "regen.toml",
    "content/en/site.yaml",
    "content/en/pages/index.yaml",
    "content/de/site.yaml",
    "content/de/pages/index.yaml",
    "templates/base.html",
    "templates/page.html",
    "assets/site.css",
    "assets/mark.svg",
    "public/robots.txt",
];

#[derive(clap::Parser)]
#[command(about = "Build the public repository documentation and localized guide poem")]
struct Arguments {
    /// HTTP(S) deployment origin and optional project path; no default deployment.
    #[arg(long)]
    base_url: String,
    /// Completed static directory. The destination must not already exist.
    #[arg(long)]
    output: PathBuf,
    /// CI-generated Shields endpoint, included in the generated hash manifest.
    #[arg(long)]
    coverage_badge: Option<PathBuf>,
}

/// Authored navigation groups with a fixed presentation order.
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq)]
enum MenuGroup {
    #[serde(rename = "Learn, build, ship")]
    Learn,
    Project,
}

/// Strict route-manifest entry connecting a repository document to its public page.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DocPage {
    source: String,
    slug: String,
    title: String,
    group: MenuGroup,
}

fn main() -> Result<()> {
    build(Arguments::parse(), &regen::BuildOptions::default())
}

/// Prepare independent temporary sites, then assemble them into a new static tree.
fn build(arguments: Arguments, options: &regen::BuildOptions<'_>) -> Result<()> {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let output = std::path::absolute(&arguments.output).context("cannot resolve output path")?;
    ensure_absent(&output)?;
    let pages: Vec<DocPage> = serde_json::from_str(&read_source(repository, "site/pages.json")?)?;
    let mut sources = BTreeSet::new();
    let mut slugs = BTreeSet::new();
    for page in &pages {
        ensure!(
            SOURCES.contains(&page.source.as_str()),
            "undocumented source: {}",
            page.source
        );
        ensure!(
            sources.insert(page.source.as_str()),
            "duplicate source: {}",
            page.source
        );
        ensure!(
            slugs.insert(page.slug.as_str()),
            "duplicate route: {}",
            page.slug
        );
    }
    ensure!(
        sources.len() == SOURCES.len() && slugs.contains(""),
        "incomplete documentation route manifest"
    );

    // Keep the original URL spelling until ReGen has validated it. In particular,
    // Url::parse must not silently normalize away a caller's dot segments.
    let base_url = arguments
        .base_url
        .strip_suffix('/')
        .unwrap_or(&arguments.base_url);
    let poem_url = format!("{base_url}/guide/poem");
    // Resolve the OS temporary parent, not authored inputs: macOS aliases /var
    // to /private/var, while ReGen deliberately rejects symlinked input paths.
    let temporary = tempfile::Builder::new()
        .prefix("regen-docs-")
        .tempdir_in(fs::canonicalize(std::env::temp_dir())?)?;
    let poem_root = temporary.path().join("poem-input");
    let docs_root = temporary.path().join("docs-input");
    for relative in POEM_INPUTS {
        copy_source(
            repository,
            &format!("examples/minimal/{relative}"),
            &poem_root.join(relative),
        )?;
    }
    let poem_config = read_source(&poem_root, "regen.toml")?;
    fs::write(
        poem_root.join("regen.toml"),
        with_base_url(&poem_config, &poem_url)?,
    )?;
    // Canonicalize metadata only; the configuration above retains raw input and
    // ReGen must still reject an invalid spelling before any output is copied.
    let poem_canonical = Url::parse(&poem_url)?;
    let robots = read_source(&poem_root, "public/robots.txt")?;
    let mut sitemap_count = 0;
    let mut poem_robots = String::new();
    for line in robots.lines() {
        if line.starts_with("Sitemap:") {
            sitemap_count += 1;
            poem_robots.push_str(&format!("Sitemap: {poem_canonical}/sitemap.xml\n"));
        } else {
            poem_robots.push_str(line);
            poem_robots.push('\n');
        }
    }
    ensure!(
        sitemap_count == 1,
        "minimal example must have exactly one robots sitemap"
    );
    fs::write(poem_root.join("public/robots.txt"), poem_robots)?;
    let poem = regen::build_with_options(&poem_root, options)
        .context("cannot build localized guide poem")?;

    let normalized = Url::parse(base_url)?;
    let base_path = normalized.path().trim_end_matches('/');
    let canonical_base = normalized.as_str().trim_end_matches('/');
    let docs_config = format!(
        "[site]\ntitle = \"ReGen\"\nbase_url = {}\ndefault_language = \"en\"\n\n[[languages]]\ncode = \"en\"\nname = \"English\"\n",
        serde_json::to_string(base_url)?
    );
    fs::create_dir_all(docs_root.join("content/en/pages"))?;
    fs::write(docs_root.join("regen.toml"), docs_config)?;
    for template in [
        "page.html",
        "pipeline.html",
        "architecture.html",
        "poem-example.html",
    ] {
        copy_source(
            repository,
            &format!("site/templates/{template}"),
            &docs_root.join("templates").join(template),
        )?;
    }
    for asset in [
        "docs.css",
        "critx-mark.svg",
        "github-mark.svg",
        "fonts/space-grotesk-latin-variable.woff2",
        "fonts/ibm-plex-sans-latin-variable.woff2",
        "fonts/ibm-plex-mono-latin-400.woff2",
        "fonts/space-grotesk-ofl.txt",
        "fonts/ibm-plex-ofl.txt",
        "fonts/provenance.json",
    ] {
        copy_source(
            repository,
            &format!("site/assets/{asset}"),
            &docs_root.join("assets").join(asset),
        )?;
    }
    for asset in ["regen-logo.svg", "regen-logo-static.svg", "regen-mark.svg"] {
        copy_source(
            repository,
            &format!("site/assets/{asset}"),
            &docs_root.join("public/brand").join(asset),
        )?;
    }
    copy_generated(&poem.output, &docs_root.join("public/guide/poem"))?;
    if let Some(badge) = &arguments.coverage_badge {
        fs::copy(badge, docs_root.join("public/coverage.json"))
            .context("cannot copy CI coverage endpoint")?;
    }
    fs::write(
        docs_root.join("public/robots.txt"),
        format!(
            "User-agent: *\nAllow: /\nSitemap: {canonical_base}/sitemap.xml\nSitemap: {poem_canonical}/sitemap.xml\n"
        ),
    )?;
    let mut routes: BTreeMap<_, _> = pages
        .iter()
        .map(|page| (page.source.as_str(), route(base_path, &page.slug)))
        .collect();
    routes.extend([
        (
            "site/assets/regen-logo.svg",
            format!("{base_path}/brand/regen-logo.svg"),
        ),
        (
            "site/assets/regen-logo-static.svg",
            format!("{base_path}/brand/regen-logo-static.svg"),
        ),
        (
            "site/assets/regen-mark.svg",
            format!("{base_path}/brand/regen-mark.svg"),
        ),
    ]);
    let menu: Vec<_> = [MenuGroup::Learn, MenuGroup::Project]
        .into_iter()
        .map(|group| {
            let items: Vec<_> = pages
                .iter()
                .filter(|page| page.group == group)
                .map(|page| json!({"title": page.title, "path": routes[page.source.as_str()]}))
                .collect();
            json!({"title": group, "items": items})
        })
        .collect();
    write_json(
        &docs_root.join("content/en/site.yaml"),
        &json!({
            "menu": menu,
            "repository_url": REPOSITORY_URL,
            "cargo": cargo_metadata(repository)?,
            "poem_en_url": format!("{poem_canonical}/"),
            "poem_de_url": format!("{poem_canonical}/de/"),
            "poem_en_source": poem_source(&poem_root, "content/en/pages/index.yaml")?,
            "poem_de_source": poem_source(&poem_root, "content/de/pages/index.yaml")?
        }),
    )?;
    for page in &pages {
        let mut source = read_source(repository, &page.source)?;
        if page.source == "docs/license.md" {
            // Keep scope separate from the verbatim license, with no copied text to drift.
            let license = read_source(repository, "LICENSE")?;
            write!(source, "\n## WTFPL v2\n\n```text\n{license}\n```\n")?;
        }
        let (body_html, toc, heading_id) = render_markdown(&source, page, repository, &routes)?;
        let id = if page.slug.is_empty() {
            "index"
        } else {
            &page.slug
        };
        write_json(
            &docs_root.join(format!("content/en/pages/{id}.yaml")),
            &json!({
                "title": page.title,
                "description": format!("{} — ReGen documentation", page.title),
                "template": "page.html",
                "slug": page.slug,
                "data": {
                    "menu_group": page.group,
                    "body_html": body_html,
                    "toc": toc,
                    "heading_id": heading_id
                }
            }),
        )?;
    }
    let documentation = regen::build_with_options(&docs_root, options)
        .context("cannot build prepared documentation")?;

    // Claim only a new destination after both complete builds. create_dir refuses
    // an existing file, directory or symlink, including one appearing mid-build.
    let parent = output.parent().context("output has no parent directory")?;
    fs::create_dir_all(parent)?;
    fs::create_dir(&output)
        .with_context(|| format!("refusing existing output: {}", output.display()))?;
    copy_generated(&documentation.output, &output).with_context(|| {
        format!(
            "cannot install output; incomplete new directory retained at {}",
            output.display()
        )
    })?;
    println!(
        "Built {} documentation pages and {} poem pages into {}",
        documentation.pages,
        poem.pages,
        output.display()
    );
    Ok(())
}

/// Refuse an existing destination, including a dangling symlink, before doing work.
fn ensure_absent(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error).context("cannot inspect output destination"),
        Ok(_) => bail!("refusing existing output: {}", path.display()),
    }
}

/// Resolve normal relative components beneath a trusted, stable root without links.
///
/// Repository file names are not subject to the generator's lowercase URL alphabet.
/// This does not protect against concurrent replacement after inspection.
fn source_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let mut path = root.to_path_buf();
    for component in Path::new(relative).components() {
        let Component::Normal(name) = component else {
            bail!("invalid source path: {relative}");
        };
        path.push(name);
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("cannot read source {relative}"))?;
        ensure!(
            !metadata.file_type().is_symlink(),
            "source symlink: {relative}"
        );
        ensure!(
            metadata.is_file() || metadata.is_dir(),
            "special source file: {relative}"
        );
    }
    Ok(path)
}

/// Read authored UTF-8 text through the repository source-path boundary.
fn read_source(root: &Path, relative: &str) -> Result<String> {
    fs::read_to_string(source_path(root, relative)?)
        .with_context(|| format!("cannot read {relative}"))
}

/// Project the manifest's direct dependencies and feature declarations for the docs.
///
/// Keep Cargo's section names and feature expressions; normalize only dependency
/// shorthand so the template can present registry names, versions and optionality.
fn cargo_metadata(root: &Path) -> Result<serde_json::Value> {
    let manifest: serde_json::Value = toml::from_str(&read_source(root, "Cargo.toml")?)
        .context("invalid Cargo.toml documentation input")?;
    let mut dependencies = Vec::new();
    for section in ["dependencies", "dev-dependencies"] {
        let entries = manifest[section]
            .as_object()
            .with_context(|| format!("missing Cargo.toml [{section}]"))?;
        for (name, specification) in entries {
            let version = specification
                .as_str()
                .or_else(|| specification["version"].as_str())
                .with_context(|| format!("missing version for Cargo.toml {section}.{name}"))?;
            dependencies.push(json!({
                "name": name,
                "package": specification["package"].as_str().unwrap_or(name),
                "version": version,
                "optional": specification["optional"].as_bool().unwrap_or(false),
                "section": section
            }));
        }
    }
    ensure!(
        manifest["features"].is_object(),
        "missing Cargo.toml [features]"
    );
    Ok(json!({
        "dependencies": dependencies,
        "features": manifest["features"]
    }))
}

/// Present the poem's relevant YAML fields without copying the whole example page.
fn poem_source(root: &Path, relative: &str) -> Result<String> {
    let text = read_source(root, relative)?;
    let options = serde_saphyr::options! {
        duplicate_keys: serde_saphyr::DuplicateKeyPolicy::Error,
        reject_unsupported_tags: true,
    };
    let page: serde_json::Value = serde_saphyr::from_str_with_options(&text, options)
        .with_context(|| format!("invalid YAML in {relative}"))?;
    let mut excerpt = String::new();
    for key in ["title", "template", "slug"] {
        writeln!(excerpt, "{key}: {}", page[key])?;
    }
    excerpt.push_str("data:\n  lines:\n");
    for line in page["data"]["lines"]
        .as_array()
        .with_context(|| format!("missing poem lines in {relative}"))?
    {
        writeln!(excerpt, "    - {line}")?;
    }
    Ok(excerpt)
}

/// Copy one enumerated authored file without replacing an existing destination.
fn copy_source(root: &Path, relative: &str, destination: &Path) -> Result<()> {
    let source = source_path(root, relative)?;
    ensure!(source.is_file(), "not a source file: {relative}");
    fs::create_dir_all(
        destination
            .parent()
            .context("copy destination has no parent")?,
    )?;
    let mut output = File::create_new(destination)?;
    std::io::copy(&mut File::open(source)?, &mut output)?;
    Ok(())
}

/// Copy a completed private build tree, rejecting special files and output collisions.
fn copy_generated(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in WalkDir::new(source).sort_by_file_name() {
        let entry = entry?;
        let target = destination.join(entry.path().strip_prefix(source)?);
        if entry.file_type().is_dir() {
            fs::create_dir_all(target)?;
        } else {
            ensure!(
                entry.file_type().is_file(),
                "non-regular generated file: {}",
                entry.path().display()
            );
            let mut output = File::create_new(target)?;
            std::io::copy(&mut File::open(entry.path())?, &mut output)?;
        }
    }
    Ok(())
}

/// Emit JSON as a YAML-compatible input without a second serialization policy.
fn write_json(path: &Path, value: &serde_json::Value) -> Result<()> {
    fs::create_dir_all(path.parent().context("JSON destination has no parent")?)?;
    let mut file = File::create_new(path)?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n")?;
    Ok(())
}

/// Substitute only the known fixture's site URL, leaving raw URL validation to ReGen.
///
/// This intentionally is not a general TOML editor; the authored fixture has one
/// single-line `site.base_url`, and a missing or repeated assignment is an error.
fn with_base_url(config: &str, base_url: &str) -> Result<String> {
    let mut in_site = false;
    let mut replacements = 0;
    let mut output = String::new();
    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_site = trimmed == "[site]";
        }
        if in_site
            && trimmed
                .split_once('=')
                .is_some_and(|(key, _)| key.trim() == "base_url")
        {
            replacements += 1;
            output.push_str(&format!(
                "base_url = {}\n",
                serde_json::to_string(base_url)?
            ));
        } else {
            output.push_str(line);
            output.push('\n');
        }
    }
    ensure!(
        replacements == 1,
        "minimal config must contain one site.base_url"
    );
    Ok(output)
}

/// Match the documentation site's default-language routing beneath its public prefix.
fn route(base_path: &str, slug: &str) -> String {
    if slug.is_empty() {
        format!("{base_path}/")
    } else {
        format!("{base_path}/{slug}/")
    }
}

/// Derive readable heading anchors; the caller resolves repeated stems in source order.
fn heading_slug(title: &str) -> String {
    let slug: String = title
        .chars()
        .flat_map(char::to_lowercase)
        .filter_map(|character| {
            if character.is_whitespace() {
                Some('-')
            } else if character.is_alphanumeric() || character == '-' || character == '_' {
                Some(character)
            } else {
                None
            }
        })
        .collect();
    if slug.is_empty() {
        "section".to_owned()
    } else {
        slug
    }
}

/// Return trusted HTML, section navigation, and the heading ID owned by the template.
///
/// Rewrite parser events rather than Markdown text so code examples stay literal.
/// Raw authored HTML is retained, not sanitized.
fn render_markdown(
    source: &str,
    page: &DocPage,
    repository: &Path,
    routes: &BTreeMap<&str, String>,
) -> Result<(String, Vec<serde_json::Value>, String)> {
    let options =
        Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS;
    let mut events: Vec<_> = Parser::new_ext(source, options).collect();
    let mut headings = Vec::new();
    let mut first_h1 = None;
    let mut first_section = None;
    let mut depth = 0;
    let mut index = 0;
    while index < events.len() {
        if let Event::Start(Tag::Heading { level, .. }) = &events[index] {
            let level = *level;
            let end = (index + 1..events.len())
                .find(|&next| matches!(events[next], Event::End(TagEnd::Heading(_))))
                .context("heading has no closing event")?;
            let mut title = String::new();
            for event in &events[index + 1..end] {
                match event {
                    Event::Text(text) | Event::Code(text) => title.push_str(text),
                    Event::SoftBreak | Event::HardBreak => title.push(' '),
                    _ => {}
                }
            }
            if level == HeadingLevel::H1 && depth == 0 && first_h1.is_none() {
                first_h1 = Some((index, end));
            }
            if level == HeadingLevel::H2 && depth == 0 && first_section.is_none() {
                first_section = Some(index);
            }
            headings.push((index, level, title));
            index = end + 1;
        } else {
            match &events[index] {
                Event::Start(_) => depth += 1,
                Event::End(_) => depth -= 1,
                _ => {}
            }
            index += 1;
        }
    }
    // The homepage template owns its introduction; the README owns its sections.
    let body_start = if page.slug.is_empty() {
        Some(first_section.context("homepage source requires a top-level H2 section")?)
    } else {
        None
    };
    let mut heading_id = heading_slug(&page.title);
    let mut used = BTreeSet::new();
    if first_h1.is_none() {
        used.insert(heading_id.clone());
    }
    let mut toc = Vec::new();
    // Assign IDs before removing template-owned headings so links retain the same
    // duplicate-heading numbering as the complete source document.
    for (index, level, title) in headings {
        let stem = heading_slug(&title);
        let mut id = stem.clone();
        let mut suffix = 0;
        while !used.insert(id.clone()) {
            suffix += 1;
            id = format!("{stem}-{suffix}");
        }
        if first_h1.is_some_and(|(start, _)| start == index) {
            heading_id.clone_from(&id);
        }
        if (level == HeadingLevel::H2 || level == HeadingLevel::H3)
            && body_start.is_none_or(|start| index >= start)
        {
            toc.push(json!({"id": id, "title": title, "level": level as u8}));
        }
        if let Event::Start(Tag::Heading { id: anchor, .. }) = &mut events[index] {
            *anchor = Some(id.into());
        }
    }
    if let Some(start) = body_start {
        events.drain(..start);
    } else if let Some((start, end)) = first_h1 {
        events.drain(start..=end);
    }
    for event in &mut events {
        if let Event::Start(Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. }) = event
            && let Some(rewritten) = rewrite_link(dest_url, page, repository, routes)?
        {
            *dest_url = rewritten.into();
        }
    }
    // Remote Markdown images become alt text inside any enclosing link, avoiding
    // changing third-party badges. Trusted raw HTML is outside this event policy.
    let mut images = Vec::new();
    events.retain(|event| match event {
        Event::Start(Tag::Image { dest_url, .. }) => {
            let local = !dest_url.starts_with("//") && Url::parse(dest_url).is_err();
            images.push(local);
            local
        }
        Event::End(TagEnd::Image) => images.pop().unwrap_or(false),
        _ => true,
    });
    let mut output = String::new();
    // No text replacement or smart punctuation: code events remain untouched.
    // Keep native table sizing while letting the surrounding region scroll.
    let events = events.into_iter().flat_map(|event| {
        let before = matches!(&event, Event::Start(Tag::Table(_)))
            .then(|| Event::Html("<div class=\"table-scroll\">".into()));
        let after =
            matches!(&event, Event::End(TagEnd::Table)).then(|| Event::Html("</div>".into()));
        [before, Some(event), after].into_iter().flatten()
    });
    html::push_html(&mut output, events);
    Ok((output, toc, heading_id))
}

/// Resolve authored relative links to published routes or existing repository targets.
///
/// Queries and fragments survive rewriting. Other absolute URLs remain authored;
/// this resolves repository navigation, not arbitrary URL safety or reachability.
fn rewrite_link(
    destination: &str,
    page: &DocPage,
    repository: &Path,
    routes: &BTreeMap<&str, String>,
) -> Result<Option<String>> {
    if let Some(relative) = destination.strip_prefix(DOCUMENTATION_URL) {
        let root = routes
            .get("README.md")
            .context("documentation home route is missing")?;
        return Ok(Some(format!("{root}{relative}")));
    }
    if destination.is_empty()
        || destination.starts_with(['#', '?', '/'])
        || Url::parse(destination).is_ok()
    {
        return Ok(None);
    }
    let split = destination.find(['?', '#']).unwrap_or(destination.len());
    let (path, suffix) = destination.split_at(split);
    ensure!(
        !path.contains('\\'),
        "backslash in link from {}: {destination}",
        page.source
    );
    let decoded = decode_path(path)?;
    let mut components: Vec<_> = page.source.split('/').collect();
    components.pop();
    for component in decoded.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                ensure!(
                    components.pop().is_some(),
                    "link escapes repository from {}: {destination}",
                    page.source
                );
            }
            name => components.push(name),
        }
    }
    let relative = components.join("/");
    if let Some(route) = routes.get(relative.as_str()) {
        return Ok(Some(format!("{route}{suffix}")));
    }
    let target = source_path(repository, &relative)
        .with_context(|| format!("broken repository link in {}: {destination}", page.source))?;
    let view = if target.is_dir() { "tree" } else { "blob" };
    let mut url = Url::parse(&format!("{REPOSITORY_URL}/{view}/main/"))?;
    url.path_segments_mut()
        .map_err(|()| anyhow::anyhow!("repository URL cannot contain paths"))?
        .pop_if_empty()
        .extend(components);
    Ok(Some(format!("{url}{suffix}")))
}

/// Decode link paths before traversal checks so encoded separators cannot hide `..`.
fn decode_path(path: &str) -> Result<String> {
    let mut bytes = Vec::with_capacity(path.len());
    let mut input = path.bytes();
    while let Some(byte) = input.next() {
        if byte == b'%' {
            let high = input
                .next()
                .and_then(|value| (value as char).to_digit(16))
                .context("invalid percent escape in link")?;
            let low = input
                .next()
                .and_then(|value| (value as char).to_digit(16))
                .context("invalid percent escape in link")?;
            bytes.push((high * 16 + low) as u8);
        } else {
            bytes.push(byte);
        }
    }
    let decoded = String::from_utf8(bytes).context("link path is not UTF-8")?;
    ensure!(!decoded.contains(['\\', '\0']), "invalid decoded link path");
    Ok(decoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    // A deployment prefix exercises both the documentation and nested poem URLs.
    const BASE_URL: &str = "https://example.invalid/regen";
    const BASE_PATH: &str = "/regen/";
    const CSS_SOURCES: &[(&str, &str)] = &[
        ("assets/<hash>/docs.css", "site/assets/docs.css"),
        (
            "guide/poem/assets/<hash>/site.css",
            "examples/minimal/assets/site.css",
        ),
    ];

    fn output_files(root: &Path) -> Result<BTreeMap<String, Vec<u8>>> {
        let mut files = BTreeMap::new();
        for entry in WalkDir::new(root).sort_by_file_name() {
            let entry = entry?;
            if entry.file_type().is_dir() {
                continue;
            }
            ensure!(
                entry.file_type().is_file(),
                "non-regular output: {}",
                entry.path().display()
            );
            let path = entry
                .path()
                .strip_prefix(root)?
                .to_str()
                .context("output path is not UTF-8")?
                .replace(std::path::MAIN_SEPARATOR, "/");
            files.insert(path, fs::read(entry.path())?);
        }
        Ok(files)
    }

    // Discover only the two actual asset trees, never arbitrary hex strings in
    // authored HTML, scripts, examples, public content, or manifest digests.
    fn asset_namespaces(files: &BTreeMap<String, Vec<u8>>) -> Result<Vec<(String, String)>> {
        let mut namespaces = Vec::new();
        for root in ["assets/", "guide/poem/assets/"] {
            let hashes: BTreeSet<_> = files
                .keys()
                .filter_map(|path| path.strip_prefix(root))
                .filter_map(|path| path.split_once('/').map(|(hash, _)| hash))
                .collect();
            ensure!(
                hashes.len() == 1,
                "expected one actual asset namespace beneath {root}, found {hashes:?}"
            );
            let hash = hashes.first().context("missing asset namespace")?;
            ensure!(
                hash.len() == 64
                    && hash
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "invalid asset digest beneath {root}: {hash}"
            );
            namespaces.push((format!("{root}{hash}/"), format!("{root}<hash>/")));
        }
        Ok(namespaces)
    }

    fn normalize_path(path: &str, namespaces: &[(String, String)]) -> String {
        for (actual, normalized) in namespaces {
            if let Some(suffix) = path.strip_prefix(actual) {
                return format!("{normalized}{suffix}");
            }
        }
        path.to_owned()
    }

    fn normalize_html(bytes: &[u8], namespaces: &[(String, String)]) -> Result<String> {
        let mut html = std::str::from_utf8(bytes)?.to_owned();
        for (actual, normalized) in namespaces {
            html = html.replace(
                &format!("{BASE_PATH}{actual}"),
                &format!("{BASE_PATH}{normalized}"),
            );
        }
        Ok(html)
    }

    #[cfg(feature = "minify-css")]
    fn compare_css_resources(before: &[u8], after: &[u8], path: &str) -> Result<()> {
        use lightningcss::dependencies::Dependency;
        use lightningcss::stylesheet::{ParserOptions, PrinterOptions, StyleSheet};

        let parse = |bytes| {
            StyleSheet::parse(std::str::from_utf8(bytes)?, ParserOptions::default())
                .map_err(|error| anyhow::anyhow!("cannot parse CSS {path}: {error}"))
        };
        let before = parse(before)?;
        let after = parse(after)?;
        ensure!(
            before.license_comments == after.license_comments,
            "release changed CSS license notices: {path}"
        );
        let dependencies = |stylesheet: &StyleSheet| -> Result<_> {
            let printed = stylesheet
                .to_css(PrinterOptions {
                    analyze_dependencies: Some(Default::default()),
                    ..PrinterOptions::default()
                })
                .map_err(|error| anyhow::anyhow!("cannot inspect CSS resources {path}: {error}"))?;
            Ok(printed
                .dependencies
                .context("native CSS dependency inventory is missing")?
                .into_iter()
                .map(|dependency| match dependency {
                    Dependency::Import(import) => {
                        ("import", import.url, import.supports, import.media)
                    }
                    Dependency::Url(url) => ("url", url.url, None, None),
                })
                .collect::<Vec<_>>())
        };
        ensure!(
            dependencies(&before)? == dependencies(&after)?,
            "release changed ordered CSS resource URLs or import conditions: {path}"
        );
        Ok(())
    }

    #[test]
    fn real_site_release_minification_preserves_static_content() -> Result<()> {
        let temporary = tempfile::Builder::new()
            .prefix("regen-docs-regression-")
            .tempdir_in(fs::canonicalize(std::env::temp_dir())?)?;
        let baseline_options = regen::BuildOptions {
            profile: Some("release"),
            minify_html: Some(false),
            minify_css: Some(false),
            minify_js: Some(false),
            ..regen::BuildOptions::default()
        };
        let release_options = regen::BuildOptions {
            profile: Some("release"),
            ..regen::BuildOptions::default()
        };
        for (name, options) in [
            ("baseline", &baseline_options),
            ("release", &release_options),
            ("repeat", &release_options),
        ] {
            build(
                Arguments {
                    base_url: BASE_URL.to_owned(),
                    output: temporary.path().join(name),
                    coverage_badge: None,
                },
                options,
            )
            .with_context(|| format!("cannot build actual documentation and poem: {name}"))?;
        }
        let baseline = output_files(&temporary.path().join("baseline"))?;
        let release = output_files(&temporary.path().join("release"))?;
        let repeat = output_files(&temporary.path().join("repeat"))?;
        ensure!(
            release.keys().eq(repeat.keys()),
            "repeated release changed the complete output inventory"
        );
        for (path, bytes) in &release {
            ensure!(
                bytes == &repeat[path],
                "repeated release changed output bytes: {path}"
            );
        }

        let baseline_namespaces = asset_namespaces(&baseline)?;
        let release_namespaces = asset_namespaces(&release)?;
        let baseline: BTreeMap<_, _> = baseline
            .into_iter()
            .map(|(path, bytes)| (normalize_path(&path, &baseline_namespaces), bytes))
            .collect();
        let release: BTreeMap<_, _> = release
            .into_iter()
            .map(|(path, bytes)| (normalize_path(&path, &release_namespaces), bytes))
            .collect();
        ensure!(
            baseline.keys().eq(release.keys()),
            "release changed output paths beyond the two actual asset-hash namespaces"
        );
        let expected_pages = BTreeSet::from([
            "index.html",
            "guide/index.html",
            "reference/index.html",
            "deployment/index.html",
            "architecture/index.html",
            "releasing/index.html",
            "testing/index.html",
            "security/index.html",
            "contributing/index.html",
            "changelog/index.html",
            "license/index.html",
            "guide/poem/index.html",
            "guide/poem/de/index.html",
        ]);
        let actual_pages: BTreeSet<_> = baseline
            .keys()
            .filter(|path| path.ends_with(".html"))
            .map(String::as_str)
            .collect();
        ensure!(
            actual_pages == expected_pages,
            "real documentation/poem routes differ: expected {expected_pages:?}, got {actual_pages:?}"
        );
        for &(path, source) in CSS_SOURCES {
            let baseline_css = baseline
                .get(path)
                .with_context(|| format!("missing actual stylesheet {path}"))?;
            let authored_css =
                fs::read(source_path(Path::new(env!("CARGO_MANIFEST_DIR")), source)?)?;
            ensure!(
                baseline_css == &authored_css,
                "all-disabled build did not preserve authored CSS: {source}"
            );
            #[cfg(feature = "minify-css")]
            ensure!(
                baseline_css != &release[path],
                "release did not compact the actual stylesheet {source}"
            );
        }

        for (path, baseline_bytes) in &baseline {
            let release_bytes = &release[path];
            if path == "regen-manifest.json" || path == "guide/poem/regen-manifest.json" {
                // Digests and lengths describe changed CSS/HTML bytes, not public
                // authored content. Compare metadata and routed inventory here;
                // the repeated build above compares both complete manifests.
                let mut before: serde_json::Value = serde_json::from_slice(baseline_bytes)?;
                let mut after: serde_json::Value = serde_json::from_slice(release_bytes)?;
                let manifest_root = path
                    .strip_suffix("regen-manifest.json")
                    .context("unexpected manifest path")?;
                let inventory = |manifest: &serde_json::Value, namespaces: &[(String, String)]| {
                    Ok::<BTreeSet<String>, anyhow::Error>(
                        manifest["files"]
                            .as_object()
                            .with_context(|| format!("missing file inventory in {path}"))?
                            .keys()
                            .map(|path| {
                                normalize_path(&format!("{manifest_root}{path}"), namespaces)
                            })
                            .collect(),
                    )
                };
                ensure!(
                    inventory(&before, &baseline_namespaces)?
                        == inventory(&after, &release_namespaces)?,
                    "release changed manifest inventory: {path}"
                );
                before["files"] = serde_json::Value::Null;
                after["files"] = serde_json::Value::Null;
                ensure!(before == after, "release changed manifest metadata: {path}");
            } else if path.ends_with(".html") {
                // No parsing, whitespace folding, raw-text stripping, or script
                // execution: every HTML byte other than these asset URLs matters.
                ensure!(
                    normalize_html(baseline_bytes, &baseline_namespaces)?
                        == normalize_html(release_bytes, &release_namespaces)?,
                    "release changed HTML/raw text beyond versioned asset URLs: {path}"
                );
            } else if path.ends_with(".css")
                && release_namespaces
                    .iter()
                    .any(|(_, normalized)| path.starts_with(normalized))
            {
                #[cfg(feature = "minify-css")]
                compare_css_resources(baseline_bytes, release_bytes, path)?;
                #[cfg(not(feature = "minify-css"))]
                ensure!(
                    baseline_bytes == release_bytes,
                    "CSS must pass through exactly without minify-css: {path}"
                );
            } else {
                ensure!(
                    baseline_bytes == release_bytes,
                    "release changed non-CSS assets, public content, or metadata: {path}"
                );
            }
        }
        Ok(())
    }
}
