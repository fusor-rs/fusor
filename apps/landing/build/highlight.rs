//! Produce escaped text tokens at build time; no browser highlighter or raw HTML.
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
    pub fn new() -> Self {
        Self {
            syntaxes: two_face::syntax::extra_newlines(),
            themes: ThemeSet::load_defaults(),
        }
    }

    pub fn tokens(
        &self,
        code: &str,
        extension: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let syntax = self
            .syntaxes
            .find_syntax_by_extension(extension)
            .ok_or_else(|| format!("Missing syntax for {extension}"))?;
        let mut highlighter =
            HighlightLines::new(syntax, &self.themes.themes["base16-eighties.dark"]);
        let mut tokens: Vec<(String, String)> = Vec::new();
        for line in LinesWithEndings::from(code) {
            for (style, text) in highlighter.highlight_line(line, &self.syntaxes)? {
                // Match .source-panel's lightest background, --color-night (#272a2c),
                // and keep all text at 4.5:1.
                let style = format!("color:{}", accessible_color(style.foreground, [39, 42, 44]));
                if let Some((previous, _)) = tokens.last_mut().filter(|(_, color)| *color == style)
                {
                    previous.push_str(text);
                } else {
                    tokens.push((text.to_owned(), style));
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
}

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
