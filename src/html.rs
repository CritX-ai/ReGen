//! Thin configuration adapter for minify-html; no ReGen-owned HTML transforms.
//!
//! HTML minification is opt-in because the native library always rewrites
//! whitespace and some attributes. These switches cannot make it lossless.

use crate::config::HtmlOptions;

pub(crate) fn optimize(source: &str, options: &HtmlOptions) -> Vec<u8> {
    minify_html::minify(
        source.as_bytes(),
        &minify_html::Cfg {
            keep_comments: !options.remove_comments,
            keep_ssi_comments: !options.remove_ssi_comments,
            keep_closing_tags: !options.omit_closing_tags,
            keep_html_and_head_opening_tags: !options.omit_html_head_opening_tags,
            keep_input_type_text_attr: !options.remove_input_type_text,
            minify_doctype: options.minify_doctype,
            allow_noncompliant_unquoted_attribute_values: options
                .allow_noncompliant_unquoted_attribute_values,
            allow_optimal_entities: options.allow_optimal_entities,
            allow_removing_spaces_between_attributes: options
                .allow_removing_spaces_between_attributes,
            remove_bangs: options.remove_bangs,
            remove_processing_instructions: options.remove_processing_instructions,
            // Inline languages are deliberately not passed to separate optimizers.
            minify_css: false,
            minify_js: false,
            ..minify_html::Cfg::default()
        },
    )
}

/// Compare HTML5 trees, retaining text (including whitespace), comments and attributes.
/// Parser recovery is not a browser execution or layout equivalence oracle.
pub(crate) fn check_regression(source: &str, output: &[u8]) -> anyhow::Result<bool> {
    if source.as_bytes() == output {
        return Ok(true);
    }
    let output = std::str::from_utf8(output)?;
    let before = scraper::Html::parse_document(source);
    let after = scraper::Html::parse_document(output);
    // Preorder node values plus child counts describe the rooted tree without
    // comparing allocator IDs or copying serialized DOMs.
    let before_nodes = before
        .tree
        .root()
        .descendants()
        .map(|node| (node.value(), node.children().count()));
    let after_nodes = after
        .tree
        .root()
        .descendants()
        .map(|node| (node.value(), node.children().count()));
    Ok(before.quirks_mode == after.quirks_mode && before_nodes.eq(after_nodes))
}

#[cfg(test)]
#[path = "../tests/unit/html.rs"]
mod tests;
