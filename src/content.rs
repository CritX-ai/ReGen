//! Translation inventory, page schema, and deployment-independent routes.
//!
//! Validate the complete content layout before rendering: every locale must have
//! one homepage and the same page IDs, while slugs may differ by translation.
//! Ordered maps make template iteration stable; output installation belongs elsewhere.

use std::collections::{BTreeMap, BTreeSet, btree_map::Entry};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Config;
use crate::files::{PathCases, portable_path, read_text, visit_tree};

/// Complete translation set, validated for cross-locale IDs and route uniqueness.
pub(crate) struct Content {
    /// All configured locales, keyed by language code.
    pub localized: BTreeMap<String, Localized>,
    /// Authored output spellings, shared with public files before any path can alias.
    pub output_cases: PathCases,
}

/// Authored content for one language.
pub(crate) struct Localized {
    /// Free-form localized values exposed to templates as `site`.
    pub site: BTreeMap<String, Value>,
    /// Pages keyed by relative YAML path without its extension, not by slug.
    pub pages: BTreeMap<String, Page>,
}

/// Resolved page metadata with an explicit extension point for template-specific data.
#[derive(Serialize)]
pub(crate) struct Page {
    /// Localized page title, also used in navigation.
    pub title: String,
    /// Localized description made available to the selected template.
    pub description: String,
    /// Portable `.html` path relative to `templates/`.
    pub template: String,
    /// Resolved route suffix; only an explicitly empty slug denotes the locale homepage.
    pub slug: String,
    /// Free-form template values without weakening the page metadata schema.
    pub data: BTreeMap<String, Value>,
}

impl Content {
    /// Load required locale content using an already validated configuration.
    ///
    /// Returns errors for invalid layout or YAML, unsafe page paths, reserved or
    /// duplicate routes, missing homepages, and mismatched translation IDs.
    pub(crate) fn load(root: &Path, config: &Config) -> Result<Self> {
        let content_root = root.join("content");
        let language_codes: BTreeSet<&str> = config
            .languages
            .iter()
            .map(|language| language.code.as_str())
            .collect();
        let paths = content_files(&content_root, &language_codes)?;

        let mut localized: BTreeMap<String, Localized> = config
            .languages
            .iter()
            .map(|language| {
                (
                    language.code.clone(),
                    Localized {
                        site: BTreeMap::new(),
                        pages: BTreeMap::new(),
                    },
                )
            })
            .collect();
        let mut routes: BTreeMap<String, &Path> = BTreeMap::new();
        let mut output_cases = PathCases::default();
        output_cases
            .insert("sitemap.xml")
            .expect("fixed sitemap name is portable and unique");
        output_cases
            .insert("regen-manifest.json")
            .expect("fixed manifest name is portable and distinct");
        for path in &paths {
            // The inventory guarantees portable UTF-8 paths in configured locales:
            // site.yaml or files beneath pages/. Page extensions are checked below.
            let relative = path
                .strip_prefix(&content_root)
                .expect("content_files returns descendants of content/");
            let mut components = relative.components();
            let code = components
                .next()
                .expect("content_files excludes the content root")
                .as_os_str()
                .to_str()
                .expect("visit_tree validates UTF-8 paths");
            let locale = localized
                .get_mut(code)
                .expect("content_files validates configured locales");
            let locale_path = components.as_path();
            if locale_path == Path::new("site.yaml") {
                locale.site = read_yaml(path)?;
                continue;
            }
            let page_path = locale_path
                .strip_prefix("pages")
                .expect("content_files validates the locale layout");
            let id = page_id(page_path)
                .with_context(|| format!("invalid content file {}", path.display()))?;
            let page = read_page(path, &id)?;
            validate_page(&page).with_context(|| format!("invalid page {}", path.display()))?;

            if code == config.site.default_language
                && let Some(first) = page.slug.split('/').next()
            {
                ensure!(
                    !["assets", "sitemap.xml", "regen-manifest.json"]
                        .iter()
                        .any(|reserved| first.eq_ignore_ascii_case(reserved))
                        && !language_codes
                            .iter()
                            .any(|code| first.eq_ignore_ascii_case(code)),
                    "{}: slug {:?} conflicts with a reserved root namespace",
                    path.display(),
                    page.slug
                );
            }
            let page_route = route(code, &config.site.default_language, &page.slug);
            output_cases
                .insert(&format!("{}index.html", page_route.trim_start_matches('/')))
                .with_context(|| format!("invalid output path for {}", path.display()))?;
            match routes.entry(page_route) {
                Entry::Vacant(entry) => {
                    entry.insert(path);
                }
                Entry::Occupied(entry) => {
                    bail!(
                        "{} and {} have the same route {:?}",
                        entry.get().display(),
                        path.display(),
                        entry.key()
                    );
                }
            }
            locale.pages.insert(id, page);
        }

        // Config::load validates the default against the same language list
        // used to populate localized, whose entries are never removed.
        let default_locale = &localized[&config.site.default_language];
        for (code, locale) in &localized {
            let homepages = locale
                .pages
                .values()
                .filter(|page| page.slug.is_empty())
                .count();
            ensure!(
                homepages == 1,
                "content/{code}/pages must contain exactly one empty-slug homepage; found {homepages}"
            );
            if locale.pages.keys().ne(default_locale.pages.keys()) {
                let missing: Vec<&str> = default_locale
                    .pages
                    .keys()
                    .filter(|id| !locale.pages.contains_key(*id))
                    .map(String::as_str)
                    .collect();
                let additional: Vec<&str> = locale
                    .pages
                    .keys()
                    .filter(|id| !default_locale.pages.contains_key(*id))
                    .map(String::as_str)
                    .collect();
                bail!(
                    "content/{code}/pages IDs do not match default language {:?}; missing: {missing:?}; additional: {additional:?}",
                    config.site.default_language
                );
            }
        }
        Ok(Self {
            localized,
            output_cases,
        })
    }
}

/// Form a trailing-slash route from validated codes and slugs, without the base path.
pub(crate) fn route(code: &str, default: &str, slug: &str) -> String {
    match (code == default, slug.is_empty()) {
        (true, true) => "/".to_owned(),
        (true, false) => format!("/{slug}/"),
        (false, true) => format!("/{code}/"),
        (false, false) => format!("/{code}/{slug}/"),
    }
}

/// Validate directories as well as files so empty or misspelled locales cannot vanish.
fn content_files(content_root: &Path, expected: &BTreeSet<&str>) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    let mut found = BTreeMap::new();
    // Parent-before-child traversal establishes each locale before recording its
    // required entries; empty directories remain visible to layout validation.
    visit_tree(content_root, false, |entry| {
        let path = entry.path();
        match entry.depth() {
            1 => {
                ensure!(
                    entry.file_type().is_dir(),
                    "{}: content/ may contain only configured language directories",
                    path.display()
                );
                let name = entry
                    .file_name()
                    .to_str()
                    .expect("visit_tree validates UTF-8 names");
                let code = expected.get(name).with_context(|| {
                    format!("{}: language {name:?} is not configured", path.display())
                })?;
                found.insert(*code, (false, false));
            }
            2 => {
                let code = path
                    .parent()
                    .expect("depth-two entries have a locale parent")
                    .file_name()
                    .expect("locale parents have a name")
                    .to_str()
                    .expect("visit_tree validates UTF-8 names");
                let (has_site, has_pages) = found
                    .get_mut(code)
                    .expect("visit_tree visits validated locales before their children");
                match entry
                    .file_name()
                    .to_str()
                    .expect("visit_tree validates UTF-8 names")
                {
                    "site.yaml" if entry.file_type().is_file() => *has_site = true,
                    "pages" if entry.file_type().is_dir() => *has_pages = true,
                    _ => bail!(
                        "{}: each language directory accepts only site.yaml and pages/",
                        path.display()
                    ),
                }
            }
            _ => {}
        }
        if entry.file_type().is_file() {
            paths.push(entry.into_path());
        }
        Ok(())
    })?;
    for code in expected {
        let (has_site, has_pages) = found.get(code).with_context(|| {
            format!(
                "{}: missing configured language directory {code:?}",
                content_root.display()
            )
        })?;
        ensure!(
            *has_site,
            "{}: missing required site.yaml",
            content_root.join(code).display()
        );
        ensure!(
            *has_pages,
            "{}: missing required pages/ directory",
            content_root.join(code).display()
        );
    }
    Ok(paths)
}

/// Derive translation identity from the source path, independently of localized slugs.
fn page_id(path: &Path) -> Result<String> {
    let mut id = String::new();
    for component in path.components() {
        let segment = component
            .as_os_str()
            .to_str()
            .expect("page paths come from visit_tree's validated UTF-8 paths");
        if !id.is_empty() {
            id.push('/');
        }
        id.push_str(segment);
    }
    ensure!(
        path.extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("yaml")),
        "page files must have the .yaml extension"
    );
    id.truncate(id.len() - ".yaml".len());
    portable_path(&id).context("page ID must be a portable relative path")?;
    Ok(id)
}

/// Deserialize authored metadata separately so only an omitted slug uses the filename.
fn read_page(path: &Path, id: &str) -> Result<Page> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct PageSource {
        title: String,
        description: String,
        template: String,
        #[serde(default, deserialize_with = "deserialize_slug")]
        slug: Option<String>,
        #[serde(default)]
        data: BTreeMap<String, Value>,
    }

    // Deserializing a present value as String keeps YAML null invalid; Option's
    // usual deserializer would incorrectly treat null as an omitted field.
    fn deserialize_slug<'de, D>(deserializer: D) -> std::result::Result<Option<String>, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Some)
    }

    let source: PageSource = read_yaml(path)?;
    Ok(Page {
        title: source.title,
        description: source.description,
        template: source.template,
        slug: source.slug.unwrap_or_else(|| {
            id.rsplit('/')
                .next()
                .expect("validated page IDs contain a basename")
                .to_owned()
        }),
        data: source.data,
    })
}

fn validate_page(page: &Page) -> Result<()> {
    portable_path(&page.template).context("template must be a portable relative path")?;
    ensure!(
        Path::new(&page.template)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("html")),
        "template must have the .html extension"
    );
    if !page.slug.is_empty() {
        portable_path(&page.slug).context("slug must be empty or a portable relative path")?;
    }
    Ok(())
}

fn read_yaml<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = read_text(path)?;
    // Keep the parser's default document and alias resource limits. Includes
    // and other unsupported tags are errors, never filesystem or network reads.
    let options = serde_saphyr::options! {
        duplicate_keys: serde_saphyr::DuplicateKeyPolicy::Error,
        reject_unsupported_tags: true,
    };
    serde_saphyr::from_str_with_options(&text, options)
        .with_context(|| format!("invalid YAML in {}", path.display()))
}
