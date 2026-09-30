//! Small, deliberately limited authoring format: paragraphs, bullet lists and
//! inline code. Produces typed text nodes, never HTML. Malformed code
//! delimiters and half-bulleted paragraphs fail the build.
pub fn compile(source: &str) -> Result<String, String> {
    let mut output = String::from("&[");
    for (id, paragraph) in source
        .split("\n\n")
        .filter(|p| !p.trim().is_empty())
        .enumerate()
    {
        let paragraph = paragraph.trim();
        let lines: Vec<&str> = paragraph.lines().map(str::trim).collect();
        let bullets = lines.iter().filter(|line| line.starts_with("- ")).count();
        if bullets == 0 {
            output.push_str(&format!(
                "ParagraphData {{ id: {id}, spans: {}, items: &[] }},",
                inline(paragraph)?
            ));
            continue;
        }
        if bullets != lines.len() {
            return Err(format!(
                "Start every line of a list paragraph with \"- \": {paragraph}"
            ));
        }
        output.push_str(&format!("ParagraphData {{ id: {id}, spans: &[], items: &["));
        for (item, line) in lines.iter().enumerate() {
            output.push_str(&format!(
                "ItemData {{ id: {item}, spans: {} }},",
                inline(&line[2..])?
            ));
        }
        output.push_str("] },");
    }
    output.push(']');
    Ok(output)
}

/// One run of text and inline code, for terms and list items.
pub fn inline(text: &str) -> Result<String, String> {
    if text.matches('`').count() % 2 != 0 {
        return Err(format!("Unclosed inline code in documentation: {text}"));
    }
    let mut output = String::from("&[");
    for (index, part) in text.split('`').enumerate() {
        if part.is_empty() {
            if index % 2 == 1 {
                return Err(format!("Empty inline code in documentation: {text}"));
            }
            continue;
        }
        output.push_str(&format!(
            "InlineData {{ id: {index}, code: {}, text: {part:?} }},",
            index % 2 == 1
        ));
    }
    output.push(']');
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::compile;

    #[test]
    fn preserves_code_and_paragraph_boundaries_as_data() {
        let result =
            compile("Use `Signal<String>` with `\"Ada\"`.\n\nCall `update(|n| *n += 1)`.").unwrap();
        assert_eq!(result.matches("ParagraphData {").count(), 2);
        assert_eq!(result.matches("code: true").count(), 3);
        assert!(result.contains("text: \"Signal<String>\""));
        assert!(result.contains("text: \"update(|n| *n += 1)\""));
    }

    #[test]
    fn bullet_paragraphs_become_list_items() {
        let result = compile("Before.\n\n- One `a`\n- Two").unwrap();
        assert_eq!(result.matches("ParagraphData {").count(), 2);
        assert_eq!(result.matches("ItemData {").count(), 2);
        assert!(result.contains("text: \"One \""));
        assert!(result.contains("text: \"a\""));
        assert!(compile("- One\nnot a bullet").is_err());
    }

    #[test]
    fn rejects_incomplete_markup_and_ignores_empty_paragraphs() {
        assert!(compile("Use `owner").is_err());
        assert!(compile("Use `` here").is_err());
        assert!(compile("- Use `owner").is_err());
        assert_eq!(compile(" \n\n").unwrap(), "&[]");
        // HTML-looking input is still a text value, with no HTML injection API.
        assert!(
            compile("`<script>alert(1)</script>`")
                .unwrap()
                .contains("text: \"<script>alert(1)</script>\"")
        );
    }
}
