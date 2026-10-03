//! Highlight code at build time for guides and live-example source viewers.
use std::fmt::Write;
use syntect::{
    easy::HighlightLines,
    highlighting::{Color, ThemeSet},
    parsing::SyntaxSet,
    util::LinesWithEndings,
};

pub struct Highlighter {
    syntaxes: SyntaxSet,
    themes: ThemeSet,
}

impl Highlighter {
    pub(crate) fn new() -> Self {
        Self {
            syntaxes: two_face::syntax::extra_newlines(),
            themes: ThemeSet::load_defaults(),
        }
    }

    fn syntax(&self, label: &str) -> &syntect::parsing::SyntaxReference {
        let label = label.to_lowercase();
        let extension = if label.contains(".html") || label.contains("html") {
            "html"
        } else if label.contains(".rs") || label.starts_with("rust") {
            "rs"
        } else if label.contains("toml") {
            "toml"
        } else if label.contains(".js") || label.starts_with("javascript") {
            "js"
        } else if label.contains(".ts") || label.starts_with("typescript") {
            "ts"
        } else if label.contains("json") {
            "json"
        } else if label.starts_with("terminal") {
            "sh"
        } else {
            &label
        };
        self.syntaxes
            .find_syntax_by_extension(extension)
            .unwrap_or_else(|| self.syntaxes.find_syntax_plain_text())
    }

    fn highlight(
        &self,
        code: &str,
        label: &str,
    ) -> Result<Vec<(String, String)>, Box<dyn std::error::Error>> {
        let syntax = self.syntax(label);
        let mut light = HighlightLines::new(syntax, &self.themes.themes["InspiredGitHub"]);
        let mut dark = HighlightLines::new(syntax, &self.themes.themes["base16-ocean.dark"]);
        let mut tokens: Vec<(String, String)> = Vec::new();
        for line in LinesWithEndings::from(code) {
            let light = light.highlight_line(line, &self.syntaxes)?;
            let dark = dark.highlight_line(line, &self.syntaxes)?;
            let (mut li, mut di, mut lo, mut d_o) = (0, 0, 0, 0);
            // Themes can coalesce adjacent spans differently; intersect their ranges.
            while li < light.len() && di < dark.len() {
                let len = (light[li].1.len() - lo).min(dark[di].1.len() - d_o);
                let text = &light[li].1[lo..lo + len];
                let style = format!(
                    "--syntax-light:{};--syntax-dark:{}",
                    accessible_color(light[li].0.foreground, [247, 249, 248]),
                    accessible_color(dark[di].0.foreground, [21, 34, 27]),
                );
                if let Some((previous, _)) = tokens.last_mut().filter(|(_, s)| *s == style) {
                    previous.push_str(text);
                } else {
                    tokens.push((text.to_owned(), style));
                }
                lo += len;
                d_o += len;
                if lo == light[li].1.len() {
                    li += 1;
                    lo = 0;
                }
                if d_o == dark[di].1.len() {
                    di += 1;
                    d_o = 0;
                }
            }
        }
        assert_eq!(
            tokens
                .iter()
                .map(|(text, _)| text.as_str())
                .collect::<String>(),
            code
        );
        Ok(tokens)
    }

    pub fn tokens(&self, code: &str, label: &str) -> Result<String, Box<dyn std::error::Error>> {
        let tokens = self.highlight(code, label)?;
        let mut source = String::from("&[");
        for (id, (text, style)) in tokens.iter().enumerate() {
            write!(
                source,
                "CodeToken {{ id: {id}, text: {text:?}, style: {style:?} }},"
            )?;
        }
        source.push(']');
        Ok(source)
    }

    pub(crate) fn html(
        &self,
        code: &str,
        language: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let mut html = String::new();
        for (text, style) in self.highlight(code, language)? {
            write!(
                html,
                "<span class=\"syntax-token\" style=\"{style}\">{}</span>",
                super::html::escape(&text)
            )?;
        }
        Ok(html)
    }
}

// Keep comments and punctuation readable against the actual docs backgrounds.
fn accessible_color(color: Color, background: [u8; 3]) -> String {
    fn luminance(rgb: [u8; 3]) -> f64 {
        rgb.into_iter()
            .zip([0.2126, 0.7152, 0.0722])
            .map(|(channel, weight)| {
                let value = f64::from(channel) / 255.0;
                weight
                    * if value <= 0.04045 {
                        value / 12.92
                    } else {
                        ((value + 0.055) / 1.055).powf(2.4)
                    }
            })
            .sum()
    }
    let mut rgb = [color.r, color.g, color.b];
    let bg = luminance(background);
    loop {
        let fg = luminance(rgb);
        if (fg.max(bg) + 0.05) / (fg.min(bg) + 0.05) >= 4.5 {
            break;
        }
        for channel in &mut rgb {
            *channel = if bg > 0.5 {
                channel.saturating_sub(4)
            } else {
                channel.saturating_add(4)
            };
        }
    }
    format!("#{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2])
}
