use crate::Result;
use fusor::template::escape_into;
use std::fmt::Write as _;
const KEY_ATTRIBUTE: &str = " data-fusor-key=\"";

/// HTML created by a generated server component. It cannot be constructed from
/// arbitrary runtime strings; dynamic values go through escaped writer methods.
#[derive(Clone, Debug)]
pub struct Html {
    source: String,
    editable: bool,
    first_open: Option<usize>,
}
impl Html {
    pub(super) fn is_editable(&self) -> bool {
        self.editable
    }

    pub fn as_str(&self) -> &str {
        &self.source
    }
    #[doc(hidden)]
    pub fn with_key(mut self, key: &impl serde::Serialize) -> Result<Self> {
        let offset = self
            .first_open
            .ok_or("keyed rows require an element root")?;
        let mut attribute = String::from(KEY_ATTRIBUTE);
        escape_into(&mut attribute, &fusor_islands::encode(key)?, true);
        attribute.push('"');
        self.source.insert_str(offset, &attribute);
        Ok(self)
    }
    pub fn into_string(self) -> String {
        self.source
    }
}
impl std::fmt::Display for Html {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.source.fmt(f)
    }
}

/// Generated server code writes static syntax separately from escaped values.
#[doc(hidden)]
#[derive(Default)]
pub struct Writer {
    source: String,
    editable: bool,
    first_open: Option<usize>,
    // Scratch belongs to this render, not a global cache or the returned Html.
    // Nested keyed children finish using it before their parent's key is encoded.
    key_json: Vec<u8>,
    key_attribute: String,
}

// Format directly into the escaped destination, including Display implementations
// that emit several fragments. No intermediate formatted String is necessary.
struct Escaped<'a> {
    output: &'a mut String,
    attribute: bool,
}
impl std::fmt::Write for Escaped<'_> {
    fn write_str(&mut self, value: &str) -> std::fmt::Result {
        escape_into(self.output, value, self.attribute);
        Ok(())
    }
}
impl Writer {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn literal(&mut self, value: &'static str) {
        self.source.push_str(value);
    }
    /// Compiler-escaped adjacent markup and its structural root metadata.
    /// Dynamic expressions are evaluated only after their preceding chunk.
    #[doc(hidden)]
    pub fn static_markup(
        &mut self,
        value: &'static str,
        first_open: Option<usize>,
        editable: bool,
    ) {
        if self.first_open.is_none() {
            self.first_open = first_open.map(|offset| self.source.len() + offset);
        }
        self.editable |= editable;
        self.source.push_str(value);
    }
    pub fn open(&mut self, tag: &'static str) {
        self.editable |= matches!(tag, "input" | "textarea" | "select");
        self.source.push('<');
        self.source.push_str(tag);
        if self.first_open.is_none() {
            self.first_open = Some(self.source.len());
        }
    }
    pub fn attr(&mut self, name: &'static str, value: impl std::fmt::Display) {
        self.source.push(' ');
        self.source.push_str(name);
        self.source.push_str("=\"");
        let start = self.source.len();
        write!(
            Escaped {
                output: &mut self.source,
                attribute: true
            },
            "{value}"
        )
        .expect("Display returned an error while writing HTML");
        self.editable |= name == "contenteditable" && &self.source[start..] != "false";
        self.source.push('"');
    }
    /// Compiler-escaped static attributes; dynamic values still use `attr`.
    pub fn static_attributes(&mut self, attributes: &'static str, editable: bool) {
        self.editable |= editable;
        self.source.push_str(attributes);
    }
    pub fn boolean(&mut self, name: &'static str, value: bool) {
        if value {
            self.attr(name, "");
        }
    }
    pub fn end_open(&mut self) {
        self.source.push('>');
    }
    pub fn close(&mut self, tag: &'static str) {
        self.source.push_str("</");
        self.source.push_str(tag);
        self.source.push('>');
    }
    pub fn text(&mut self, value: impl std::fmt::Display) {
        write!(
            Escaped {
                output: &mut self.source,
                attribute: false
            },
            "{value}"
        )
        .expect("Display returned an error while writing HTML");
    }
    pub fn child(&mut self, value: &Html) {
        self.editable |= value.editable;
        self.source.push_str(value.as_str());
    }
    pub(super) fn rendered_root(&mut self, value: &Html) {
        if self.first_open.is_none() {
            self.first_open = value.first_open.map(|offset| self.source.len() + offset);
        }
        self.child(value);
    }
    /// Render a nested component without allocating a separate output buffer.
    pub fn child_into(&mut self, render: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        self.region(render, |_| Ok(()))
    }
    pub fn keyed_child(
        &mut self,
        key: &impl serde::Serialize,
        render: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<()> {
        self.region(render, |writer| {
            let offset = writer
                .first_open
                .ok_or("keyed rows require an element root")?;
            writer.key_json.clear();
            serde_json::to_writer(&mut writer.key_json, key)?;
            let json = std::str::from_utf8(&writer.key_json).expect("serde_json writes UTF-8");
            writer.key_attribute.clear();
            writer.key_attribute.push_str(KEY_ATTRIBUTE);
            escape_into(&mut writer.key_attribute, json, true);
            writer.key_attribute.push('"');
            writer.source.insert_str(offset, &writer.key_attribute);
            Ok(())
        })
    }
    fn region(
        &mut self,
        render: impl FnOnce(&mut Self) -> Result<()>,
        finish: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<()> {
        // Restore the parent's root metadata on success; roll back bytes and
        // editability too on error or unwind, including a failing key encoder.
        struct Region<'a> {
            writer: &'a mut Writer,
            start: usize,
            first_open: Option<usize>,
            editable: bool,
            committed: bool,
        }
        impl Drop for Region<'_> {
            fn drop(&mut self) {
                self.writer.first_open = self.first_open;
                if !self.committed {
                    self.writer.source.truncate(self.start);
                    self.writer.editable = self.editable;
                }
            }
        }
        let mut region = Region {
            start: self.source.len(),
            first_open: self.first_open.take(),
            editable: self.editable,
            writer: self,
            committed: false,
        };
        render(region.writer)?;
        finish(region.writer)?;
        region.committed = true;
        Ok(())
    }
    pub fn inert_json(&mut self, json: &str) {
        for ch in json.chars() {
            match ch {
                '<' => self.source.push_str("\\u003c"),
                '>' => self.source.push_str("\\u003e"),
                '&' => self.source.push_str("\\u0026"),
                '\u{2028}' => self.source.push_str("\\u2028"),
                '\u{2029}' => self.source.push_str("\\u2029"),
                ch => self.source.push(ch),
            }
        }
    }
    pub fn finish(self) -> Html {
        Html {
            source: self.source,
            editable: self.editable,
            first_open: self.first_open,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Writer;

    fn section_pair(editable: bool) -> (Writer, Writer) {
        let mut ordinary = Writer::new();
        ordinary.literal("<!--日本語😀-->");
        ordinary.open("section");
        ordinary.static_attributes(
            if editable {
                " contenteditable=\"FALSE\""
            } else {
                " contenteditable=\"false\""
            },
            editable,
        );
        ordinary.end_open();
        let mut packed = Writer::new();
        packed.static_markup(
            if editable {
                "<!--日本語😀--><section contenteditable=\"FALSE\">"
            } else {
                "<!--日本語😀--><section contenteditable=\"false\">"
            },
            Some("<!--日本語😀--><section".len()),
            editable,
        );
        (ordinary, packed)
    }

    #[test]
    fn packed_markup_preserves_utf8_root_editability_and_escaping() {
        for editable in [false, true] {
            let (mut ordinary, mut packed) = section_pair(editable);
            assert_eq!(packed.source, ordinary.source);
            assert_eq!(packed.first_open, ordinary.first_open);
            assert_eq!(packed.editable, ordinary.editable);
            packed.text("<&\"日本語");
            ordinary.text("<&\"日本語");
            packed.static_markup("</section>", None, false);
            ordinary.close("section");
            assert_eq!(
                packed.finish().with_key(&"<&日本語").unwrap().as_str(),
                ordinary.finish().with_key(&"<&日本語").unwrap().as_str()
            );
        }
    }

    #[test]
    fn packed_and_streamed_regions_rollback_errors_and_panics() {
        use std::panic::{AssertUnwindSafe, catch_unwind};
        for editable in [false, true] {
            let (_, mut writer) = section_pair(editable);
            let before = writer.source.clone();
            let root = writer.first_open;
            let error = writer.child_into(|writer| {
                writer.static_markup(
                    "<!--nested--><input>",
                    Some("<!--nested--><input".len()),
                    true,
                );
                Err("renderer failed".into())
            });
            assert!(
                matches!(error.unwrap_err(), crate::Error::Render(message) if message == "renderer failed")
            );
            let panic = catch_unwind(AssertUnwindSafe(|| {
                writer.child_into(|writer| {
                    writer.open("input");
                    writer.end_open();
                    panic!("renderer failed");
                })
            }));
            assert_eq!(
                panic.unwrap_err().downcast_ref::<&str>(),
                Some(&"renderer failed")
            );
            assert_eq!(writer.source, before);
            assert_eq!(writer.first_open, root);
            assert_eq!(writer.editable, editable);
            let error = writer
                .keyed_child(&1, |child| {
                    child.text("no root");
                    Ok(())
                })
                .unwrap_err();
            assert!(
                matches!(error, crate::Error::Render(message) if message == "keyed rows require an element root")
            );
            assert_eq!(writer.source, before);
        }
    }

    #[test]
    fn streamed_children_preserve_nested_root_keys() {
        let mut writer = Writer::new();
        writer.open("ul");
        writer.end_open();
        let parent_root = writer.first_open;
        writer
            .keyed_child(&"日本語<&", |child| {
                child.open("li");
                child.end_open();
                child.child_into(|nested| {
                    nested.open("input");
                    nested.end_open();
                    Ok(())
                })?;
                child.close("li");
                Ok(())
            })
            .unwrap();
        assert_eq!(writer.first_open, parent_root);
        assert!(writer.editable);
        writer.close("ul");
        assert_eq!(
            writer.finish().with_key(&9).unwrap().as_str(),
            "<ul data-fusor-key=\"9\"><li data-fusor-key=\"&quot;日本語&lt;&amp;&quot;\"><input></li></ul>"
        );
    }

    #[test]
    fn streaming_display_escapes_fragments_and_preserves_editability() {
        struct Fragments<'a>(&'a [&'a str]);
        impl std::fmt::Display for Fragments<'_> {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                for fragment in self.0 {
                    formatter.write_str(fragment)?;
                }
                Ok(())
            }
        }
        let fragments = Fragments(&["日本語", "<&", "\"'", ">😀"]);
        let mut writer = Writer::new();
        writer.open("div");
        writer.attr("title", format_args!("prefix {{{fragments}}} {:04}", 7));
        writer.attr("contenteditable", Fragments(&["f", "al", "se"]));
        writer.end_open();
        writer.text(format_args!("{fragments} {:04}", 7));
        writer.close("div");
        let html = writer.finish();
        assert_eq!(
            html.as_str(),
            "<div title=\"prefix {日本語&lt;&amp;&quot;&#39;&gt;😀} 0007\" contenteditable=\"false\">日本語&lt;&amp;\"'&gt;😀 0007</div>"
        );
        assert!(!html.editable);
        for value in ["", "FALSE", "false ", "<false>"] {
            let mut writer = Writer::new();
            writer.attr("contenteditable", Fragments(&[value]));
            assert!(writer.finish().editable);
        }
    }

    #[test]
    fn compiled_literals_preserve_dynamic_escaping_metadata_and_root_keys() {
        for editable in [false, true] {
            let mut dynamic = Writer::new();
            dynamic.literal("<!--before root-->");
            dynamic.open("section");
            dynamic.attr("title", "\"<&>'日本語😀");
            dynamic.attr("contenteditable", if editable { "true" } else { "false" });
            dynamic.attr("data-dynamic", "<&\"");
            dynamic.end_open();
            dynamic.text("\"<&>'日本語😀");
            dynamic.close("section");
            let dynamic = dynamic.finish();

            let mut compiled = Writer::new();
            compiled.literal("<!--before root-->");
            compiled.open("section");
            compiled.static_attributes(" title=\"&quot;&lt;&amp;&gt;&#39;日本語😀\"", false);
            compiled.static_attributes(
                if editable {
                    " contenteditable=\"true\""
                } else {
                    " contenteditable=\"false\""
                },
                editable,
            );
            compiled.attr("data-dynamic", "<&\"");
            compiled.end_open();
            compiled.literal("\"&lt;&amp;&gt;'日本語😀");
            compiled.close("section");
            let compiled = compiled.finish();
            assert_eq!(compiled.as_str(), dynamic.as_str());
            assert_eq!(compiled.editable, editable);
            assert_eq!(compiled.editable, dynamic.editable);
            assert_eq!(compiled.first_open, dynamic.first_open);
            let key = "<&\"日本語";
            let compiled = compiled.with_key(&key).unwrap();
            assert_eq!(compiled.as_str(), dynamic.with_key(&key).unwrap().as_str());
            assert!(
                compiled
                    .as_str()
                    .starts_with("<!--before root--><section data-fusor-key=\"")
            );

            let mut parent = Writer::new();
            parent.child(&compiled);
            assert_eq!(parent.finish().editable, editable);
        }
    }

    #[test]
    fn reused_key_scratch_preserves_escaping_and_nested_root_identity() {
        fn leaf(writer: &mut Writer) -> super::Result<()> {
            writer.open("li");
            writer.end_open();
            writer.text("payload");
            writer.close("li");
            Ok(())
        }
        let mut writer = Writer::new();
        writer.open("ul");
        writer.end_open();
        let parent_root = writer.first_open;
        let mut expected = String::from("<ul>");
        for (key, row) in [
            ("日本語\"'&<>😀".to_owned(), r#"<li data-fusor-key="&quot;日本語\&quot;&#39;&amp;&lt;&gt;😀&quot;">payload</li>"#.to_owned()),
            ("x".repeat(2048), format!("<li data-fusor-key=\"&quot;{}&quot;\">payload</li>", "x".repeat(2048))),
            (String::new(), "<li data-fusor-key=\"&quot;&quot;\">payload</li>".into()),
            ("short".to_owned(), "<li data-fusor-key=\"&quot;short&quot;\">payload</li>".into()),
        ] {
            expected.push_str(&row);
            writer.keyed_child(&key, leaf).unwrap();
            assert_eq!(writer.first_open, parent_root);
        }
        writer
            .keyed_child(&("outer<&", 7), |writer| {
                writer.open("li");
                writer.end_open();
                writer.keyed_child(&["a", "日本語\"&😀"], |writer| {
                    writer.open("span");
                    writer.end_open();
                    writer.text("nested");
                    writer.close("span");
                    Ok(())
                })?;
                writer.close("li");
                Ok(())
            })
            .unwrap();
        expected.push_str(r#"<li data-fusor-key="[&quot;outer&lt;&amp;&quot;,7]"><span data-fusor-key="[&quot;a&quot;,&quot;日本語\&quot;&amp;😀&quot;]">nested</span></li>"#);
        writer.close("ul");
        expected.push_str("</ul>");
        assert_eq!(writer.first_open, parent_root);
        assert_eq!(writer.finish().as_str(), expected);
    }

    struct Broken<'a> {
        panic: bool,
        rendered: &'a std::cell::Cell<bool>,
    }
    impl serde::Serialize for Broken<'_> {
        fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
            assert!(
                self.rendered.get(),
                "key serialization must follow child rendering"
            );
            use serde::ser::SerializeSeq;
            let mut sequence = serializer.serialize_seq(Some(2))?;
            sequence.serialize_element("partial日本語<&")?;
            if self.panic {
                panic!("expected key serialization panic");
            }
            Err(serde::ser::Error::custom(
                "expected key serialization error",
            ))
        }
    }

    #[test]
    fn partial_key_serialization_errors_and_panics_allow_subsequent_keys() {
        use std::{
            cell::Cell,
            panic::{AssertUnwindSafe, catch_unwind},
        };
        let mut writer = Writer::new();
        writer.open("section");
        writer.end_open();
        let parent_root = writer.first_open;
        for panic in [false, true, false] {
            let before = writer.source.clone();
            let rendered = Cell::new(false);
            let key = Broken {
                panic,
                rendered: &rendered,
            };
            let result = catch_unwind(AssertUnwindSafe(|| {
                writer.keyed_child(&key, |writer| {
                    writer.open("input");
                    writer.end_open();
                    rendered.set(true);
                    Ok(())
                })
            }));
            if panic {
                assert_eq!(
                    result.unwrap_err().downcast_ref::<&str>(),
                    Some(&"expected key serialization panic")
                );
            } else {
                let crate::Error::Json(error) = result.unwrap().unwrap_err() else {
                    panic!("expected key serialization error");
                };
                assert_eq!(error.to_string(), "expected key serialization error");
            }
            assert!(rendered.get());
            assert_eq!(writer.source, before);
            assert_eq!(writer.first_open, parent_root);
            assert!(!writer.editable);
            let key = "recovered日本語<&";
            writer
                .keyed_child(&key, |writer| {
                    writer.open("div");
                    writer.end_open();
                    writer.close("div");
                    Ok(())
                })
                .unwrap();
            assert_eq!(
                writer.source,
                format!(
                    "{before}<div data-fusor-key=\"&quot;recovered日本語&lt;&amp;&quot;\"></div>"
                )
            );
            assert_eq!(writer.first_open, parent_root);
        }
    }
}
