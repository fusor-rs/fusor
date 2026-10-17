use fusor::template::{ComponentId, ElementId, TextId, TextMarker};

#[test]
fn html_identifiers_have_one_canonical_encoding() {
    for number in [0, 1, 19, usize::MAX] {
        let id = ElementId::new(number);
        assert_eq!(id.to_string().parse::<ElementId>().unwrap(), id);
        assert_eq!(id.index(), number);
        assert_eq!(ComponentId::new(number).to_string(), number.to_string());
    }
    assert_eq!(ComponentId::in_library(7, 3).to_string(), "7-3");
    for invalid in ["", "+1", "-1", "01", " 1", "1 ", "1.0", "١"] {
        assert!(invalid.parse::<ElementId>().is_err(), "{invalid}");
    }
}

#[test]
fn text_anchors_roundtrip_and_malformed_reserved_comments_fail() {
    for marker in [
        TextMarker::Start(TextId::new(0)),
        TextMarker::End(TextId::new(42)),
    ] {
        assert_eq!(
            TextMarker::parse(&marker.to_string()).unwrap(),
            Some(marker)
        );
    }
    assert_eq!(TextMarker::parse("ordinary comment").unwrap(), None);
    for invalid in ["fusor:", "fusor:no", "/fusor:01", "fusor:1 trailing"] {
        assert!(TextMarker::parse(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn shared_escaping_preserves_unicode_and_distinguishes_text_and_attributes() {
    let value = "\"<&>'日本語😀";
    let mut text = String::new();
    fusor::template::escape_into(&mut text, value, false);
    assert_eq!(text, "\"&lt;&amp;&gt;'日本語😀");
    let mut attribute = String::new();
    fusor::template::escape_into(&mut attribute, value, true);
    assert_eq!(attribute, "&quot;&lt;&amp;&gt;&#39;日本語😀");
}

#[test]
fn html_escape_spans_match_replacement_semantics_across_utf8_boundaries() {
    let alphabet: Vec<char> = (0..=127)
        .map(char::from)
        .chain("日本語😀é\u{2028}\u{10ffff}".chars())
        .collect();
    let mut seed = 91_u32;
    for length in 0..256 {
        let value: String = (0..length)
            .map(|_| {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                alphabet[seed as usize % alphabet.len()]
            })
            .collect();
        for attribute in [false, true] {
            let mut expected = value
                .replace('&', "&amp;")
                .replace('<', "&lt;")
                .replace('>', "&gt;");
            if attribute {
                expected = expected.replace('"', "&quot;").replace('\'', "&#39;");
            }
            let mut actual = String::from("prefix:");
            fusor::template::escape_into(&mut actual, &value, attribute);
            assert_eq!(actual, format!("prefix:{expected}"));
        }
    }
}
