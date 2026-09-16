//! Profile precedence, executable-config validation, and compiled capabilities.

#[cfg(feature = "hooks")]
use super::HookWhen;
use super::{BuildOptions, Config};
use crate::test_support::tempdir;
use std::fs;

fn load(settings: &str) -> anyhow::Result<Config> {
    let root = tempdir();
    fs::write(
        root.path().join("regen.toml"),
        format!(
            r#"{settings}
[site]
title = "Profiles"
base_url = "https://example.com/project/"
default_language = "en"
[[languages]]
code = "en"
name = "English"
"#
        ),
    )?;
    Config::load(root.path())
}

#[test]
fn profile_ancestry_overlays_common_settings_then_leaf_then_invocation() {
    let config = load(
        r#"
[build]
profile = "leaf"
[build.minify]
html = true
css = true
js = true
[build.assets]
minify = true
[profiles.dev.minify]
html = false
[profiles.parent]
extends = "dev"
[profiles.parent.minify]
css = false
[profiles.parent.assets]
minify = false
[profiles.leaf]
extends = "parent"
review = false
[profiles.leaf.minify]
js = false
"#,
    )
    .unwrap();
    let resolved = config.resolve_build(&BuildOptions::default()).unwrap();
    assert_eq!(resolved.profile, "leaf");
    assert!(!resolved.review);
    assert!(!resolved.minify.html && !resolved.minify.css && !resolved.minify.js);
    assert!(!resolved.minify_assets);

    let overridden = config
        .resolve_build(&BuildOptions {
            minify_html: Some(cfg!(feature = "minify-html")),
            minify_css: Some(cfg!(feature = "minify-css")),
            minify_js: Some(cfg!(feature = "minify-js")),
            minify_assets: Some(true),
            ..BuildOptions::default()
        })
        .unwrap();
    assert_eq!(overridden.minify.html, cfg!(feature = "minify-html"));
    assert_eq!(overridden.minify.css, cfg!(feature = "minify-css"));
    assert_eq!(overridden.minify.js, cfg!(feature = "minify-js"));
    assert!(overridden.minify_assets);
}

#[test]
fn profile_selection_uses_the_ultimate_builtin_and_implicit_release_parent() {
    let config = load(
        r#"
[build]
profile = "preview"
[profiles.preview]
extends = "dev"
[profiles.shipping]
[profiles.release]
review = true
"#,
    )
    .unwrap();
    let preview = config.resolve_build(&BuildOptions::default()).unwrap();
    assert!(preview.review);
    assert!(!preview.minify_assets);
    let shipping = config
        .resolve_build(&BuildOptions {
            profile: Some("shipping"),
            ..BuildOptions::default()
        })
        .unwrap();
    assert_eq!(shipping.profile, "shipping");
    assert!(shipping.review);
    assert!(shipping.minify_assets);
    assert!(!shipping.minify.html && !shipping.minify.js);
    assert_eq!(shipping.minify.css, cfg!(feature = "minify-css"));
}

#[cfg(feature = "hooks")]
#[test]
fn each_hook_list_replaces_independently_and_empty_clears_inherited_hooks() {
    let config = load(
        r#"
[build.hooks]
pre = [{ command = ["common-pre"] }]
post = [{ command = ["common-post"] }]
[profiles.parent]
extends = "dev"
[profiles.parent.hooks]
pre = [{ command = ["parent-pre", ""], allow_failure = true }]
post = [{ command = ["parent-post"], when = "failure", error_details = true }]
[profiles.leaf]
extends = "parent"
[profiles.leaf.hooks]
pre = []
[profiles.clear]
extends = "parent"
[profiles.clear.hooks]
post = []
"#,
    )
    .unwrap();
    let leaf = config
        .resolve_build(&BuildOptions {
            profile: Some("leaf"),
            ..BuildOptions::default()
        })
        .unwrap();
    assert!(leaf.hooks.pre.is_empty());
    let [post] = leaf.hooks.post.as_slice() else {
        panic!("a replacement must not concatenate ancestor post hooks");
    };
    assert_eq!(post.command, ["parent-post"]);
    assert!(matches!(post.when, HookWhen::Failure));
    assert!(post.error_details);
    let clear = config
        .resolve_build(&BuildOptions {
            profile: Some("clear"),
            ..BuildOptions::default()
        })
        .unwrap();
    assert!(clear.hooks.post.is_empty());
    let [pre] = clear.hooks.pre.as_slice() else {
        panic!("clearing post must preserve the effective pre replacement");
    };
    assert_eq!(pre.command, ["parent-pre", ""]);
    assert!(pre.allow_failure);
}

#[test]
fn invalid_inactive_ancestry_and_invalid_default_cannot_hide_behind_cli_selection() {
    for settings in [
        "[profiles.unused]\nextends = 'missing'",
        "[profiles.a]\nextends = 'b'\n[profiles.b]\nextends = 'a'",
        "[profiles.loop]\nextends = 'loop'",
        "[profiles.release]\nextends = 'dev'",
        "[profiles.dev]\nextends = 'release'",
        "[build]\nprofile = 'missing'",
    ] {
        let config = load(settings).unwrap();
        assert!(
            config
                .resolve_build(&BuildOptions {
                    profile: Some("dev"),
                    ..BuildOptions::default()
                })
                .is_err(),
            "accepted invalid inactive configuration: {settings}"
        );
    }
    let config = load("").unwrap();
    assert!(
        config
            .resolve_build(&BuildOptions {
                profile: Some("missing"),
                ..BuildOptions::default()
            })
            .is_err()
    );
}

#[test]
fn profile_names_are_single_portable_components_even_when_unused() {
    for name in ["", "a/b", "../outside", "Preview", "nul", "trailing.", "é"] {
        let config = load(&format!("[profiles.{name:?}]")).unwrap();
        assert!(config.resolve_build(&BuildOptions::default()).is_err());
    }
}

#[test]
fn unknown_fields_and_post_only_pre_options_are_rejected_at_every_new_boundary() {
    for settings in [
        "unknown = true",
        "[build]\nreview = true",
        "[build.minify]\nunknown = true",
        "[build.assets]\nunknown = true",
        "[build.hooks]\nunknown = []",
        "[profiles.dev]\nunknown = true",
        "[profiles.dev.minify]\nunknown = true",
        "[profiles.dev.assets]\nunknown = true",
        "[profiles.dev.hooks]\nunknown = []",
        "[build.hooks]\npre = [{ command = ['tool'], when = 'always' }]",
        "[build.hooks]\npre = [{ command = ['tool'], error_details = true }]",
        "[build.hooks]\npost = [{ command = ['tool'], unknown = true }]",
        "[build.hooks]\npost = [{ command = ['tool'], when = 'sometimes' }]",
        "[build.hooks]\npre = [{ command = 'tool argument' }]",
        "[build.hooks]\npost = [{}]",
    ] {
        assert!(
            load(settings).is_err(),
            "accepted invalid schema: {settings}"
        );
    }
}

#[test]
fn invalid_hook_argv_is_rejected_even_in_an_unselected_profile() {
    for command in ["[]", "['']", r#"["tool", "bad\u0000argument"]"#] {
        let config = load(&format!(
            "[profiles.unused.hooks]\npost = [{{ command = {command} }}]"
        ))
        .unwrap();
        assert!(config.resolve_build(&BuildOptions::default()).is_err());
    }
}

#[test]
fn unavailable_minifiers_are_checked_only_after_all_overrides() {
    for (name, available) in [
        ("html", cfg!(feature = "minify-html")),
        ("css", cfg!(feature = "minify-css")),
        ("js", cfg!(feature = "minify-js")),
    ] {
        let config = load(&format!(
            "[build]\nprofile = 'dev'\n[build.minify]\n{name} = true"
        ))
        .unwrap();
        let result = config.resolve_build(&BuildOptions::default());
        assert_eq!(result.is_ok(), available);
        if let Err(error) = result {
            assert!(format!("{error:#}").contains(&format!("minify-{name}")));
        }
        config
            .resolve_build(&BuildOptions {
                minify_html: Some(false),
                minify_css: Some(false),
                minify_js: Some(false),
                ..BuildOptions::default()
            })
            .unwrap();
    }
}

#[test]
fn minifier_options_inherit_individually_and_empty_symbol_list_clears() {
    let config = load(
        r#"
[build]
profile = "child"
[build.minify.html_options]
omit_closing_tags = true
remove_comments = true
remove_input_type_text = true
[build.minify.css_options]
optimize = true
unused_symbols = ["obsolete"]
[build.minify.js_options]
compress = true
drop_console = true
keep_names = false
source_type = "module"
[profiles.parent]
extends = "release"
[profiles.parent.minify.html_options]
omit_closing_tags = false
remove_comments = false
[profiles.parent.minify.js_options]
drop_console = false
keep_names = true
[profiles.child]
extends = "parent"
[profiles.child.minify.css_options]
unused_symbols = []
[profiles.child.minify.js_options]
source_type = "script"
"#,
    )
    .unwrap();
    let resolved = config.resolve_build(&BuildOptions::default()).unwrap();
    assert!(!resolved.minify.html_options.omit_closing_tags);
    assert!(!resolved.minify.html_options.remove_comments);
    assert!(resolved.minify.html_options.remove_input_type_text);
    assert!(resolved.minify.css_options.optimize);
    assert!(resolved.minify.css_options.unused_symbols.is_empty());
    assert!(resolved.minify.js_options.compress);
    assert!(!resolved.minify.js_options.drop_console);
    assert!(resolved.minify.js_options.keep_names);
    assert!(matches!(
        resolved.minify.js_options.source_type,
        super::JsSourceType::Script
    ));
    let parent = config
        .resolve_build(&BuildOptions {
            profile: Some("parent"),
            ..BuildOptions::default()
        })
        .unwrap();
    assert!(matches!(
        parent.minify.js_options.source_type,
        super::JsSourceType::Module
    ));
}

#[test]
fn dependent_transform_options_cannot_silently_do_nothing() {
    for option in [
        "drop_console = true",
        "drop_debugger = true",
        "join_vars = true",
        "sequences = true",
        "remove_unused = true",
        "keep_names = false",
        "pure_annotations = true",
    ] {
        let config = load(&format!("[build.minify.js_options]\n{option}")).unwrap();
        // Inactive options neither activate processing nor require its compiled capability.
        config.resolve_build(&BuildOptions::default()).unwrap();
        if cfg!(feature = "minify-js") {
            assert!(
                config
                    .resolve_build(&BuildOptions {
                        minify_js: Some(true),
                        ..Default::default()
                    })
                    .is_err()
            );
            config
                .resolve_build(&BuildOptions {
                    minify_js: Some(true),
                    minify_assets: Some(false),
                    ..Default::default()
                })
                .unwrap();
        }
    }
    let config = load("[build.minify.css_options]\nunused_symbols = ['obsolete']").unwrap();
    config
        .resolve_build(&BuildOptions {
            minify_css: Some(false),
            ..Default::default()
        })
        .unwrap();
    if cfg!(feature = "minify-css") {
        assert!(
            config
                .resolve_build(&BuildOptions {
                    minify_css: Some(true),
                    ..Default::default()
                })
                .is_err()
        );
        config
            .resolve_build(&BuildOptions {
                minify_css: Some(true),
                minify_assets: Some(false),
                ..Default::default()
            })
            .unwrap();
    }
}

#[test]
fn unknown_transform_names_and_modes_are_not_ignored() {
    for settings in [
        "[build.minify.html_options]\nremove_comments = 'yes'",
        "[profiles.unused.minify.html_options]\nminify_whitespace = true",
        "[build.minify.css_options]\nunused_symbols = false",
        "[profiles.unused.minify.js_options]\nmangle_properties = true",
        "[build.minify.js_options]\nsource_type = 'guess'",
        "[profiles.unused.minify.js_options]\nsource_type = 'guess'",
    ] {
        assert!(
            load(settings).is_err(),
            "accepted invalid options: {settings}"
        );
    }
}
