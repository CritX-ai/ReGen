//! Observable opt-in HTML behavior delegated to the native minifier.

use super::optimize;
use crate::config::HtmlOptions;

fn output(source: &str, options: &HtmlOptions) -> String {
    String::from_utf8(optimize(source, options)).unwrap()
}

#[test]
fn regression_check_compares_dom_content_not_quote_or_entity_spelling() {
    let source = "<!doctype html><html><head><title>T</title></head><body><p class=\"x\">&amp;</p><template><input type=\"text\"></template></body></html>";
    let equivalent = b"<!doctype html><title>T</title><p class=x>&#38;</p><template><input type=text></template>";
    assert!(super::check_regression(source, source.as_bytes()).unwrap());
    assert!(super::check_regression(source, equivalent).unwrap());
    assert!(
        !super::check_regression(
            source,
            b"<!doctype html><title>T</title><p class=x>&#38;</p><template><input></template>"
        )
        .unwrap()
    );
    assert!(!super::check_regression("<p>a <b>b</b></p>", b"<p>a<b>b</b></p>").unwrap());
    assert!(super::check_regression("<p>valid</p>", b"<p>\xff</p>").is_err());
}

#[test]
fn ordinary_and_ssi_comments_require_their_removal_options() {
    let source = "<!-- ordinary --><!--#include file=\"part.html\" --><p>content</p>";
    let preserved = output(source, &HtmlOptions::default());
    assert!(preserved.contains("<!-- ordinary -->"));
    assert!(preserved.contains("<!--#include file=\"part.html\" -->"));

    let ordinary_removed = output(
        source,
        &HtmlOptions {
            remove_comments: true,
            ..HtmlOptions::default()
        },
    );
    assert!(!ordinary_removed.contains("<!-- ordinary -->"));
    assert!(ordinary_removed.contains("<!--#include file=\"part.html\" -->"));

    // Keeping all comments takes precedence over the SSI-specific switch.
    let all_kept = output(
        source,
        &HtmlOptions {
            remove_ssi_comments: true,
            ..HtmlOptions::default()
        },
    );
    assert!(all_kept.contains("<!-- ordinary -->"));
    assert!(all_kept.contains("<!--#include file=\"part.html\" -->"));

    let all_removed = output(
        source,
        &HtmlOptions {
            remove_comments: true,
            remove_ssi_comments: true,
            ..HtmlOptions::default()
        },
    );
    assert!(!all_removed.contains("<!--"));
    assert!(all_removed.contains("content"));
}

#[test]
fn opening_and_closing_tag_omission_are_separate_opt_ins() {
    let source = "<html><head><title>Title</title></head><body><ul><li>one</li><li>two</li></ul></body></html>";
    let preserved = output(source, &HtmlOptions::default());
    for tag in ["<html>", "<head>", "</head>", "</li>", "</body>", "</html>"] {
        assert!(preserved.contains(tag), "missing {tag} in {preserved}");
    }

    let no_closing = output(
        source,
        &HtmlOptions {
            omit_closing_tags: true,
            ..HtmlOptions::default()
        },
    );
    assert!(!no_closing.contains("</li>"));
    assert!(no_closing.contains("<html>"));
    assert!(no_closing.contains("<head>"));

    let no_opening = output(
        source,
        &HtmlOptions {
            omit_html_head_opening_tags: true,
            ..HtmlOptions::default()
        },
    );
    assert!(!no_opening.contains("<html>"));
    assert!(!no_opening.contains("<head>"));
    assert!(no_opening.contains("</li>"));
    assert!(no_opening.contains("</html>"));
}

#[test]
fn input_text_type_is_retained_unless_explicitly_removed() {
    let source = "<input type=\"text\"><input type=\"password\">";
    let preserved = output(source, &HtmlOptions::default());
    assert_eq!(preserved.matches("type=").count(), 2);
    assert!(preserved.contains("text"));
    assert!(preserved.contains("password"));

    let removed = output(
        source,
        &HtmlOptions {
            remove_input_type_text: true,
            ..HtmlOptions::default()
        },
    );
    assert_eq!(removed.matches("type=").count(), 1);
    assert!(!removed.contains("text"));
    assert!(removed.contains("password"));
}

#[test]
fn inline_languages_keep_their_code_without_language_minification() {
    let javascript = "const value = 1 + 2;\nconsole.log(value);";
    let css = ".a { margin: 0px 0px; color: red; }";
    let data = " { \"value\": 1 } ";
    let source = format!(
        "<style>\n{css}\n</style><script>\n{javascript}\n</script><script type=\"application/json\">{data}</script>"
    );
    let optimized = output(&source, &HtmlOptions::default());
    // Native HTML processing can trim raw-text boundaries, but must not fold JS
    // expressions or optimize CSS declarations. Attribute quoting is immaterial.
    assert!(optimized.contains(javascript));
    assert!(optimized.contains(css));
    assert!(optimized.contains(data));
}
