//! Project-wide configuration and URL/language policy.
//!
//! Normalize deployment URLs and validate locale identities before content loading,
//! so downstream routing can rely on one canonical prefix and a known default.
//! This module validates syntax, not language-registry membership or URL reachability.

use std::collections::{BTreeMap, BTreeSet};
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
    #[serde(default)]
    build: BuildSettings,
    #[serde(default)]
    profiles: BTreeMap<String, Profile>,
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

/// Policy for static comparisons of minified output.
///
/// TOML accepts `false` (off), `"warn"` (report and continue), or `true` (enforce).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegressionCheckMode {
    /// Skip static regression comparisons.
    Off,
    /// Report differences to stderr without blocking output installation.
    Warn,
    /// Reject differences before installing output.
    Enforce,
}

impl std::str::FromStr for RegressionCheckMode {
    type Err = &'static str;

    fn from_str(value: &str) -> std::result::Result<Self, Self::Err> {
        match value {
            "false" => Ok(Self::Off),
            "warn" => Ok(Self::Warn),
            "true" => Ok(Self::Enforce),
            _ => Err("expected true, false, or warn"),
        }
    }
}

impl<'de> Deserialize<'de> for RegressionCheckMode {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ModeVisitor;

        impl serde::de::Visitor<'_> for ModeVisitor {
            type Value = RegressionCheckMode;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("true, false, or the string \"warn\"")
            }

            fn visit_bool<E: serde::de::Error>(self, value: bool) -> Result<Self::Value, E> {
                Ok(if value {
                    RegressionCheckMode::Enforce
                } else {
                    RegressionCheckMode::Off
                })
            }

            fn visit_str<E: serde::de::Error>(self, value: &str) -> Result<Self::Value, E> {
                if value == "warn" {
                    Ok(RegressionCheckMode::Warn)
                } else {
                    Err(E::invalid_value(serde::de::Unexpected::Str(value), &self))
                }
            }
        }

        deserializer.deserialize_any(ModeVisitor)
    }
}

impl RegressionCheckMode {
    /// Report completed comparisons; validation and processing errors stay fatal.
    #[cfg(any(feature = "minify-html", feature = "minify-css", feature = "minify-js"))]
    pub(crate) fn check(
        self,
        comparison: Result<bool>,
        language: &str,
        target: std::fmt::Arguments<'_>,
        difference: &str,
    ) -> Result<()> {
        if comparison.with_context(|| format!("cannot check {language} for {target}"))? {
            return Ok(());
        }
        if self == Self::Warn {
            eprintln!(
                "warning: {language} regression check failed for {target}: {difference}; continuing because regression_checks = \"warn\"; review the optimized output before publishing"
            );
            Ok(())
        } else {
            anyhow::bail!(
                "{language} regression check failed for {target}: {difference}; review the change before relaxing regression_checks"
            )
        }
    }
}

/// Per-invocation overrides for a configured build profile.
///
/// `None` inherits the selected profile's effective value. Overrides apply after
/// common build settings and the complete profile inheritance chain.
#[derive(Default)]
pub struct BuildOptions<'a> {
    /// Select a built-in or configured profile instead of `build.profile`.
    pub profile: Option<&'a str>,
    /// Force review output without changing the selected profile's configuration.
    pub review: bool,
    /// Enable or disable rendered HTML minification (off by default).
    pub minify_html: Option<bool>,
    /// Enable or disable CSS compact printing (enabled for release when compiled).
    pub minify_css: Option<bool>,
    /// Enable or disable JavaScript asset minification (off by default).
    pub minify_js: Option<bool>,
    /// Permit enabled CSS/JS minifiers to process assets; does not enable either.
    pub minify_assets: Option<bool>,
    /// Check minified output against conservative representations; None selects by risk.
    pub regression_checks: Option<RegressionCheckMode>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct BuildSettings {
    profile: Option<String>,
    regression_checks: Option<RegressionCheckMode>,
    #[serde(default)]
    minify: MinifySettings,
    #[serde(default)]
    assets: AssetSettings,
    #[serde(default)]
    hooks: HookSettings,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Profile {
    extends: Option<String>,
    review: Option<bool>,
    regression_checks: Option<RegressionCheckMode>,
    #[serde(default)]
    minify: MinifySettings,
    #[serde(default)]
    assets: AssetSettings,
    #[serde(default)]
    hooks: HookSettings,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct MinifySettings {
    html: Option<bool>,
    css: Option<bool>,
    js: Option<bool>,
    #[serde(default)]
    html_options: HtmlSettings,
    #[serde(default)]
    css_options: CssSettings,
    #[serde(default)]
    js_options: JsSettings,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct HtmlSettings {
    remove_comments: Option<bool>,
    remove_ssi_comments: Option<bool>,
    omit_closing_tags: Option<bool>,
    omit_html_head_opening_tags: Option<bool>,
    remove_input_type_text: Option<bool>,
    minify_doctype: Option<bool>,
    allow_noncompliant_unquoted_attribute_values: Option<bool>,
    allow_optimal_entities: Option<bool>,
    allow_removing_spaces_between_attributes: Option<bool>,
    remove_bangs: Option<bool>,
    remove_processing_instructions: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct CssSettings {
    optimize: Option<bool>,
    unused_symbols: Option<Vec<String>>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct JsSettings {
    compress: Option<bool>,
    drop_debugger: Option<bool>,
    drop_console: Option<bool>,
    join_vars: Option<bool>,
    sequences: Option<bool>,
    remove_unused: Option<bool>,
    keep_names: Option<bool>,
    pure_annotations: Option<bool>,
    source_type: Option<JsSourceType>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetSettings {
    minify: Option<bool>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct HookSettings {
    pre: Option<Vec<PreHook>>,
    post: Option<Vec<PostHook>>,
}

/// Validated effective settings shared by the build pipeline and hook runner.
#[derive(Debug)]
pub(crate) struct ResolvedBuild {
    pub profile: String,
    pub review: bool,
    pub minify: Minify,
    pub minify_assets: bool,
    pub regression_checks: RegressionCheckMode,
    pub hooks: Hooks,
}

#[derive(Debug, Serialize)]
pub(crate) struct Minify {
    pub html: bool,
    pub css: bool,
    pub js: bool,
    pub html_options: HtmlOptions,
    pub css_options: CssOptions,
    pub js_options: JsOptions,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum JsSourceType {
    #[default]
    Script,
    Module,
}

#[derive(Debug, Serialize)]
pub(crate) struct HtmlOptions {
    pub remove_comments: bool,
    pub remove_ssi_comments: bool,
    pub omit_closing_tags: bool,
    pub omit_html_head_opening_tags: bool,
    pub remove_input_type_text: bool,
    pub minify_doctype: bool,
    pub allow_noncompliant_unquoted_attribute_values: bool,
    pub allow_optimal_entities: bool,
    pub allow_removing_spaces_between_attributes: bool,
    pub remove_bangs: bool,
    pub remove_processing_instructions: bool,
}

impl HtmlOptions {
    pub const fn conservative() -> Self {
        Self {
            remove_comments: false,
            remove_ssi_comments: false,
            omit_closing_tags: false,
            omit_html_head_opening_tags: false,
            remove_input_type_text: false,
            minify_doctype: false,
            allow_noncompliant_unquoted_attribute_values: false,
            allow_optimal_entities: false,
            allow_removing_spaces_between_attributes: false,
            remove_bangs: false,
            remove_processing_instructions: false,
        }
    }
}

impl Default for HtmlOptions {
    fn default() -> Self {
        Self::conservative()
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct CssOptions {
    pub optimize: bool,
    pub unused_symbols: Vec<String>,
}

impl CssOptions {
    pub const fn conservative() -> Self {
        Self {
            optimize: false,
            unused_symbols: Vec::new(),
        }
    }
}

impl Default for CssOptions {
    fn default() -> Self {
        Self::conservative()
    }
}

#[derive(Clone, Debug, Serialize)]
pub(crate) struct JsOptions {
    pub compress: bool,
    pub drop_debugger: bool,
    pub drop_console: bool,
    pub join_vars: bool,
    pub sequences: bool,
    pub remove_unused: bool,
    pub keep_names: bool,
    pub pure_annotations: bool,
    pub source_type: JsSourceType,
}

impl JsOptions {
    pub const fn conservative() -> Self {
        Self {
            compress: false,
            drop_debugger: false,
            drop_console: false,
            join_vars: false,
            sequences: false,
            remove_unused: false,
            keep_names: true,
            pure_annotations: false,
            source_type: JsSourceType::Script,
        }
    }
}

impl Default for JsOptions {
    fn default() -> Self {
        Self::conservative()
    }
}

#[derive(Debug, Default)]
pub(crate) struct Hooks {
    pub pre: Vec<PreHook>,
    pub post: Vec<PostHook>,
}

// Inactive profiles retain strict schema validation without compiled execution.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreHook {
    pub command: Vec<String>,
    #[cfg(feature = "hooks")]
    #[serde(default)]
    pub allow_failure: bool,
    #[cfg(not(feature = "hooks"))]
    #[serde(default, rename = "allow_failure")]
    _allow_failure: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PostHook {
    pub command: Vec<String>,
    #[cfg(feature = "hooks")]
    #[serde(default)]
    pub when: HookWhen,
    #[cfg(not(feature = "hooks"))]
    #[serde(default, rename = "when")]
    _when: HookWhen,
    #[cfg(feature = "hooks")]
    #[serde(default)]
    pub error_details: bool,
    #[cfg(not(feature = "hooks"))]
    #[serde(default, rename = "error_details")]
    _error_details: bool,
    #[cfg(feature = "hooks")]
    #[serde(default)]
    pub allow_failure: bool,
    #[cfg(not(feature = "hooks"))]
    #[serde(default, rename = "allow_failure")]
    _allow_failure: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum HookWhen {
    Success,
    Failure,
    #[default]
    Always,
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

    /// Resolve and validate the complete build policy before any hook can run.
    pub(crate) fn resolve_build(&self, options: &BuildOptions<'_>) -> Result<ResolvedBuild> {
        self.validate_build()?;
        let selected = options
            .profile
            .or(self.build.profile.as_deref())
            .unwrap_or("release");
        self.validate_profile_reference(selected)?;

        // Borrow layers and hook lists until precedence has selected the winners.
        // Only the effective lists are cloned into the owned build configuration.
        let mut lineage = Vec::new();
        let mut current = selected;
        let builtin = loop {
            let profile = self.profiles.get(current);
            if let Some(profile) = profile {
                lineage.push(profile);
            }
            if is_builtin(current) {
                break current;
            }
            current = profile
                .expect("validated custom profile exists")
                .extends
                .as_deref()
                .unwrap_or("release");
        };
        let release = builtin == "release";
        let mut resolved = ResolvedBuild {
            profile: selected.to_owned(),
            review: !release,
            minify: Minify {
                // Only CSS compact printing is enabled without a site-specific opt-in.
                html: false,
                css: release && cfg!(feature = "minify-css"),
                js: false,
                html_options: HtmlOptions::default(),
                css_options: CssOptions::default(),
                js_options: JsOptions::default(),
            },
            minify_assets: release,
            regression_checks: RegressionCheckMode::Off,
            hooks: Hooks::default(),
        };
        let mut pre = &[][..];
        let mut post = &[][..];
        let mut unused_symbols = &[][..];
        let mut regression_checks = self.build.regression_checks;
        let common = (
            None,
            self.build.regression_checks,
            &self.build.minify,
            &self.build.assets,
            &self.build.hooks,
        );
        for (review, checks, minify, assets, hooks) in
            std::iter::once(common).chain(lineage.into_iter().rev().map(|profile| {
                (
                    profile.review,
                    profile.regression_checks,
                    &profile.minify,
                    &profile.assets,
                    &profile.hooks,
                )
            }))
        {
            resolved.review = review.unwrap_or(resolved.review);
            regression_checks = checks.or(regression_checks);
            resolved.minify.html = minify.html.unwrap_or(resolved.minify.html);
            resolved.minify.css = minify.css.unwrap_or(resolved.minify.css);
            resolved.minify.js = minify.js.unwrap_or(resolved.minify.js);
            // Every scalar inherits independently, including explicit false.
            macro_rules! inherit {
                ($destination:expr, $source:expr, $($field:ident),+ $(,)?) => {
                    $($destination.$field = $source.$field.unwrap_or($destination.$field);)+
                };
            }
            inherit!(
                resolved.minify.html_options,
                minify.html_options,
                remove_comments,
                remove_ssi_comments,
                omit_closing_tags,
                omit_html_head_opening_tags,
                remove_input_type_text,
                minify_doctype,
                allow_noncompliant_unquoted_attribute_values,
                allow_optimal_entities,
                allow_removing_spaces_between_attributes,
                remove_bangs,
                remove_processing_instructions,
            );
            inherit!(
                resolved.minify.js_options,
                minify.js_options,
                compress,
                drop_debugger,
                drop_console,
                join_vars,
                sequences,
                remove_unused,
                keep_names,
                pure_annotations,
                source_type,
            );
            inherit!(resolved.minify.css_options, minify.css_options, optimize);
            if let Some(symbols) = &minify.css_options.unused_symbols {
                unused_symbols = symbols;
            }
            resolved.minify_assets = assets.minify.unwrap_or(resolved.minify_assets);
            if let Some(list) = &hooks.pre {
                pre = list;
            }
            if let Some(list) = &hooks.post {
                post = list;
            }
        }
        resolved.minify.html = options.minify_html.unwrap_or(resolved.minify.html);
        resolved.minify.css = options.minify_css.unwrap_or(resolved.minify.css);
        resolved.minify.js = options.minify_js.unwrap_or(resolved.minify.js);
        resolved.minify_assets = options.minify_assets.unwrap_or(resolved.minify_assets);
        resolved.review |= options.review;
        resolved.regression_checks = options.regression_checks.or(regression_checks).unwrap_or({
            if resolved.minify.html
                || (resolved.minify_assets
                    && ((resolved.minify.css && resolved.minify.css_options.optimize)
                        || (resolved.minify.js && resolved.minify.js_options.compress)))
            {
                RegressionCheckMode::Enforce
            } else {
                RegressionCheckMode::Off
            }
        });
        for (enabled, available, feature) in [
            (
                resolved.minify.html,
                cfg!(feature = "minify-html"),
                "minify-html",
            ),
            (
                resolved.minify.css,
                cfg!(feature = "minify-css"),
                "minify-css",
            ),
            (resolved.minify.js, cfg!(feature = "minify-js"), "minify-js"),
        ] {
            ensure!(
                !enabled || available,
                "profile {selected:?} requests unavailable minifier {feature}; rebuild ReGen with Cargo feature {feature} or disable it"
            );
        }
        ensure!(
            !resolved.minify_assets
                || !resolved.minify.css
                || resolved.minify.css_options.optimize
                || unused_symbols.is_empty(),
            "CSS unused_symbols requires css_options.optimize = true"
        );
        let js = &resolved.minify.js_options;
        ensure!(
            !resolved.minify_assets
                || !resolved.minify.js
                || js.compress
                || !(js.drop_debugger
                    || js.drop_console
                    || js.join_vars
                    || js.sequences
                    || js.remove_unused
                    || !js.keep_names
                    || js.pure_annotations),
            "JavaScript transformation options require js_options.compress = true"
        );
        resolved.minify.css_options.unused_symbols = unused_symbols.to_vec();
        ensure!(
            cfg!(feature = "hooks") || (pre.is_empty() && post.is_empty()),
            "profile {selected:?} requests hooks; rebuild ReGen with --features hooks or clear the effective hook lists"
        );
        resolved.hooks.pre = pre.to_vec();
        resolved.hooks.post = post.to_vec();
        Ok(resolved)
    }

    fn validate_profile_reference(&self, name: &str) -> Result<()> {
        validate_profile_name(name)?;
        ensure!(
            is_builtin(name) || self.profiles.contains_key(name),
            "unknown build profile {name:?}"
        );
        Ok(())
    }

    fn validate_build(&self) -> Result<()> {
        self.validate_profile_reference(self.build.profile.as_deref().unwrap_or("release"))
            .context("invalid build.profile")?;
        self.build.hooks.validate().context("invalid build.hooks")?;
        for (name, profile) in &self.profiles {
            validate_profile_name(name)?;
            ensure!(
                !is_builtin(name) || profile.extends.is_none(),
                "built-in profile {name:?} cannot declare extends"
            );
            if !is_builtin(name) {
                self.validate_profile_reference(profile.extends.as_deref().unwrap_or("release"))
                    .with_context(|| format!("invalid parent of profile {name:?}"))?;
            }
            profile
                .hooks
                .validate()
                .with_context(|| format!("invalid hooks in profile {name:?}"))?;
        }

        // Iterative traversal avoids stack growth for deeply inherited profiles.
        // A false entry is on the current path; true means its ancestry is valid.
        let mut visited = BTreeMap::new();
        let mut path = Vec::new();
        for name in self.profiles.keys() {
            let mut current = name.as_str();
            while !is_builtin(current) {
                match visited.get(current) {
                    Some(true) => break,
                    Some(false) => anyhow::bail!("profile inheritance cycle at {current:?}"),
                    None => {}
                }
                visited.insert(current, false);
                path.push(current);
                current = self.profiles[current]
                    .extends
                    .as_deref()
                    .unwrap_or("release");
            }
            for ancestor in path.drain(..) {
                visited.insert(ancestor, true);
            }
        }
        Ok(())
    }
}

fn is_builtin(name: &str) -> bool {
    matches!(name, "release" | "dev")
}

fn validate_profile_name(name: &str) -> Result<()> {
    ensure!(
        !name.contains('/'),
        "profile name must be one path segment: {name:?}"
    );
    portable_path(name).with_context(|| format!("invalid profile name {name:?}"))
}

impl HookSettings {
    fn validate(&self) -> Result<()> {
        for (index, hook) in self.pre.iter().flatten().enumerate() {
            validate_command(&hook.command)
                .with_context(|| format!("invalid pre hook {}", index + 1))?;
        }
        for (index, hook) in self.post.iter().flatten().enumerate() {
            validate_command(&hook.command)
                .with_context(|| format!("invalid post hook {}", index + 1))?;
        }
        Ok(())
    }
}

fn validate_command(command: &[String]) -> Result<()> {
    ensure!(
        command.first().is_some_and(|program| !program.is_empty()),
        "hook command must be a nonempty argv array with a nonempty program"
    );
    ensure!(
        command.iter().all(|argument| !argument.contains('\0')),
        "hook command arguments must not contain NUL"
    );
    Ok(())
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

#[cfg(test)]
#[path = "../tests/unit/build_config.rs"]
mod tests;
