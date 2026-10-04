#[path = "../build/highlight.rs"]
pub mod highlight;
#[path = "../build/html.rs"]
pub mod html;
#[path = "../build/markdown.rs"]
pub mod markdown;

use highlight::Highlighter;
use std::path::Path;

#[test]
fn renders_markdown_and_preserves_explicit_and_repeated_heading_ids() {
    let document = markdown::parse(
        "# Documentation\n\nA **bold** and *emphasized* [guide](/docs/events).\n\n\
         ## First {#existing-link}\n\n1. One\n2. Two\n\n\
         | Name | Value |\n| --- | --- |\n| State | `Signal<T>` |\n\n\
         > A quote.\n\n- [x] Complete\n\n### Details\n\nText.\n\n### Details\n\nMore.\n\n\
         ### Read `Signal<T>`\n",
        Path::new(env!("CARGO_MANIFEST_DIR")),
        &Highlighter::new(),
    )
    .unwrap();
    assert_eq!(document.title, "Documentation");
    assert_eq!(
        document.lead,
        "<p>A <strong>bold</strong> and <em>emphasized</em> <a href=\"/docs/events\" title=\"\" data-fusor-link>guide</a>.</p>\n"
    );
    assert_eq!(document.sections[0].id, "existing-link");
    assert_eq!(document.sections[0].title, "First");
    let body = &document.sections[0].body.html;
    for expected in [
        "<ol>\n<li>One</li>\n<li>Two</li>\n</ol>",
        "<td><code class=\"inline-code\">Signal&lt;T&gt;</code></td>",
        "<blockquote>\n<p>A quote.</p>\n</blockquote>",
        "<input disabled=\"\" type=\"checkbox\" checked=\"\"/>",
        "<h3 id=\"details\">Details</h3>",
        "<h3 id=\"details-2\">Details</h3>",
        "<h3 id=\"read-signal-t\">Read <code class=\"inline-code\">Signal&lt;T&gt;</code></h3>",
    ] {
        assert!(body.contains(expected), "missing {expected}: {body}");
    }
    assert_eq!(
        document.anchors.into_iter().collect::<Vec<_>>(),
        ["details", "details-2", "existing-link", "read-signal-t"]
    );
    assert!(document.search.contains("Signal<T>"));
    assert!(
        document
            .search
            .starts_with("A bold and emphasized guide.\n")
    );
}

#[test]
fn renders_disclosures_but_escapes_authored_html_and_code() {
    let document = markdown::parse(
        "# Escaping\n\n## Example\n\n<details>\n<summary>Source</summary>\n\n\
         ```text title=Literal HTML\n<script>alert('hello')</script>\n```\n\n</details>\n\n\
         <img src=x onerror=alert(1)>\n\n`<App>` and ![Icon](/docs/favicon.svg).\n",
        Path::new(env!("CARGO_MANIFEST_DIR")),
        &Highlighter::new(),
    )
    .unwrap();
    let section = &document.sections[0];
    assert_eq!(section.body.code, "<script>alert('hello')</script>\n\n");
    assert!(
        section
            .body
            .html
            .contains("<details>\n<summary>Source</summary>")
    );
    assert!(
        section
            .body
            .html
            .contains("&lt;script&gt;alert(&#39;hello&#39;)&lt;/script&gt;")
    );
    assert!(
        section
            .body
            .html
            .contains("&lt;img src=x onerror=alert(1)&gt;")
    );
    assert!(
        section
            .body
            .html
            .contains("<code class=\"inline-code\">&lt;App&gt;</code>")
    );
    assert!(
        section
            .body
            .html
            .contains("<img src=\"/docs/favicon.svg\" alt=\"Icon\" />")
    );
}

#[test]
fn includes_the_compiled_tutorial_source_in_rendering_and_search() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"));
    let document = markdown::parse(
        "# Source\n\n## Counter\n\n```rust source=tutorial/src/watch.rs title=src/watch.rs\n```\n",
        directory,
        &Highlighter::new(),
    )
    .unwrap();
    let authored = std::fs::read_to_string(directory.join("tutorial/src/watch.rs")).unwrap();
    assert_eq!(document.sections[0].body.code, format!("{authored}\n"));
    assert!(document.search.contains(&authored));
    assert!(
        document.sections[0]
            .body
            .html
            .contains("aria-label=\"src/watch.rs\"")
    );
}

#[test]
fn rejects_broken_document_contracts() {
    let highlighter = Highlighter::new();
    for (source, message) in [
        ("No heading", "start each guide"),
        ("# Title\n\n# Another", "one # Title"),
        (
            "# Title\n\n## A {#same}\n\n### B {#same}",
            "duplicate heading id: same",
        ),
        ("# Title\n\n## A {#bad/id}", "invalid heading id"),
        (
            "# Title\n\n[Click](javascript:alert%281%29)",
            "unsupported documentation link scheme",
        ),
        (
            "# Title\n\n```rust source=missing.rs\nlet value = 1;\n```",
            "must be empty",
        ),
        ("# Title\n\n```rust source=missing.rs\n```", "missing.rs:"),
    ] {
        let error = markdown::parse(source, Path::new(env!("CARGO_MANIFEST_DIR")), &highlighter)
            .err()
            .expect("invalid guide must fail");
        assert!(error.to_string().contains(message), "{source}: {error}");
    }
}
