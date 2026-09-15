//! Project-wide configuration and URL/language policy.
//!
//! Normalize deployment URLs and validate locale identities before content loading,
//! so downstream routing can rely on one canonical prefix and a known default.
//! This module validates syntax, not language-registry membership or URL reachability.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::files::{portable_path, read_text};

/// Validated project settings; construct through `load` before generating routes.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Config {
    /// Shared template metadata and canonical deployment origin/prefix.
    pub site: Site,
    /// Unique locales in authored order, also used for alternate links.
    pub languages: Vec<Language>,
    /// URL path prefix without a trailing slash; empty for origin-root deployment.
    #[serde(skip)]
    pub base_path: String,
}

/// Project metadata exposed to templates as `project`.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Site {
    /// Shared project title, distinct from each locale's site content.
    pub title: String,
    /// Normalized absolute HTTP(S) base URL without a trailing slash.
    pub base_url: String,
    /// Configured locale whose routes omit the language prefix.
    pub default_language: String,
}

/// Locale metadata shared by routing and template language selectors.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Language {
    /// Portable lowercase language tag used as a content directory and URL prefix.
    pub code: String,
    /// Authored display name; no translation or registry lookup is performed.
    pub name: String,
    /// HTML text direction, restricted to `ltr` or `rtl`.
    #[serde(default = "default_direction")]
    pub direction: String,
}

impl Config {
    /// Read `regen.toml`, reject ambiguous settings, and establish routing invariants.
    ///
    /// Returns errors for unreadable or malformed TOML, invalid or duplicate locale
    /// codes, an unknown default locale, or an unsafe deployment URL.
    pub(crate) fn load(root: &Path) -> Result<Self> {
        let path = root.join("regen.toml");
        let text = read_text(&path)?;
        let mut config: Self = toml::from_str(&text)
            .with_context(|| format!("invalid project configuration {}", path.display()))?;

        ensure!(
            !config.languages.is_empty(),
            "{}: at least one language is required",
            path.display()
        );
        let mut codes = BTreeSet::new();
        for language in &config.languages {
            validate_language_code(&language.code).with_context(|| {
                format!("{}: invalid language {:?}", path.display(), language.code)
            })?;
            ensure!(
                language.code != "assets",
                "{}: language code assets conflicts with the asset namespace",
                path.display()
            );
            ensure!(
                codes.insert(language.code.as_str()),
                "{}: duplicate language code {:?}",
                path.display(),
                language.code
            );
            ensure!(
                matches!(language.direction.as_str(), "ltr" | "rtl"),
                "{}: language {:?} direction must be ltr or rtl",
                path.display(),
                language.code
            );
        }
        ensure!(
            codes.contains(config.site.default_language.as_str()),
            "{}: default_language {:?} is not a configured language",
            path.display(),
            config.site.default_language
        );
        let base_url = normalize_base_url(&config.site.base_url)
            .with_context(|| format!("{}: invalid site.base_url", path.display()))?;
        config.base_path = base_url.path().trim_end_matches('/').to_owned();
        config.site.base_url = base_url.into();
        if config.site.base_url.ends_with('/') {
            config.site.base_url.pop();
        }
        Ok(config)
    }
}

fn default_direction() -> String {
    "ltr".to_owned()
}

/// Recognize the supported ordered BCP47 subset without resolving a language registry.
fn validate_language_code(code: &str) -> Result<()> {
    portable_path(code)?;
    let mut subtags = code.split('-').peekable();
    // str::split always yields a first subtag, including for an empty string.
    let primary = subtags.next().expect("split yields a primary subtag");
    ensure!(
        (2..=8).contains(&primary.len()) && primary.bytes().all(|byte| byte.is_ascii_lowercase()),
        "language code must start with 2–8 lowercase ASCII letters"
    );

    // Accept the ordinary BCP47 language grammar, not singleton extensions,
    // private-use or grandfathered tags. Registry membership is not inferred.
    if primary.len() <= 3 {
        for _ in 0..3 {
            if subtags.peek().is_some_and(|subtag| {
                subtag.len() == 3 && subtag.bytes().all(|byte| byte.is_ascii_lowercase())
            }) {
                subtags.next();
            } else {
                break;
            }
        }
    }
    if subtags.peek().is_some_and(|subtag| {
        subtag.len() == 4 && subtag.bytes().all(|byte| byte.is_ascii_lowercase())
    }) {
        subtags.next();
    }
    if subtags.peek().is_some_and(|subtag| {
        (subtag.len() == 2 && subtag.bytes().all(|byte| byte.is_ascii_lowercase()))
            || (subtag.len() == 3 && subtag.bytes().all(|byte| byte.is_ascii_digit()))
    }) {
        subtags.next();
    }
    let mut variants = BTreeSet::new();
    for variant in subtags {
        ensure!(
            variant
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
                && ((5..=8).contains(&variant.len())
                    || (variant.len() == 4
                        && variant.as_bytes().first().is_some_and(u8::is_ascii_digit))),
            "unsupported language subtag {variant:?}; use lowercase BCP47 language, script, region and variant subtags"
        );
        ensure!(
            variants.insert(variant),
            "repeated language variant {variant:?}"
        );
    }
    Ok(())
}

/// Reject ambiguous raw URL syntax before parsing can normalize it away.
fn normalize_base_url(input: &str) -> Result<Url> {
    ensure!(
        !input
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte.is_ascii_whitespace()),
        "base_url must not contain whitespace or control characters"
    );
    let (scheme, authority_and_path) = input
        .split_once("://")
        .context("base_url must be an absolute HTTP(S) site URL")?;
    ensure!(
        scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"),
        "base_url must use HTTP or HTTPS"
    );
    ensure!(
        !authority_and_path
            .chars()
            .any(|character| matches!(character, '@' | '?' | '#' | '\\')),
        "base_url must not contain credentials, a query, a fragment or backslashes"
    );
    if let Some((authority, path)) = authority_and_path.split_once('/') {
        ensure!(!authority.is_empty(), "base_url must have an authority");
        if !path.is_empty() {
            let prefix = path.strip_suffix('/').unwrap_or(path);
            ensure!(
                !prefix.is_empty()
                    && prefix.split('/').all(|part| {
                        !part.is_empty()
                            && !matches!(part, "." | "..")
                            && part
                                .bytes()
                                .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
                    }),
                "base_url path must use normal unencoded ASCII URL segments"
            );
        }
    }

    // Check raw syntax first: URL normalization must not hide /../ or userinfo.
    Url::parse(input).context("base_url is not a valid URL")
}
