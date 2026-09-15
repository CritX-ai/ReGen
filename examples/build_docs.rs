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

/// Prepare independent temporary sites, then assemble them into a new static tree.
fn main() -> Result<()> {
    let arguments = Arguments::parse();
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
    let temporary = tempfile::Builder::new().prefix("regen-docs-").tempdir()?;
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
    let poem = regen::build(&poem_root).context("cannot build localized guide poem")?;

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
    for asset in [
        "regen-logo.svg",
        "regen-logo-static.svg",
        "regen-mark.svg",
        "version.svg",
    ] {
        copy_source(
            repository,
            &format!("site/assets/{asset}"),
            &docs_root.join("public/brand").join(asset),
        )?;
    }
    copy_generated(&poem.output, &docs_root.join("public/guide/poem"))?;
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
        (
            "site/assets/version.svg",
            format!("{base_path}/brand/version.svg"),
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
            "version": env!("CARGO_PKG_VERSION"),
            "repository_url": REPOSITORY_URL,
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
    let documentation = regen::build(&docs_root).context("cannot build prepared documentation")?;

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
    html::push_html(&mut output, events.into_iter());
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
