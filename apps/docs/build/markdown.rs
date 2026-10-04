use super::{highlight::Highlighter, html::escape};
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd, html};
use std::{collections::BTreeSet, fs, iter::Peekable, path::Path};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;
type Events<'a> = Peekable<Parser<'a>>;

pub(crate) struct Document {
    pub(crate) title: String,
    pub(crate) lead: String,
    pub(crate) sections: Vec<Section>,
    pub(crate) search: String,
    pub(crate) anchors: BTreeSet<String>,
}

pub(crate) struct Section {
    pub(crate) id: String,
    pub(crate) title: String,
    pub(crate) body: Body,
}

#[derive(Default)]
pub(crate) struct Body {
    pub(crate) html: String,
    pub(crate) search: String,
    pub(crate) code: String,
}

struct Renderer<'a> {
    highlighter: &'a Highlighter,
    directory: &'a Path,
    anchors: BTreeSet<String>,
}

struct Heading {
    text: String,
    html: String,
}

pub(crate) fn parse(source: &str, directory: &Path, highlighter: &Highlighter) -> Result<Document> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES;
    let mut events = Parser::new_ext(source, options).peekable();
    Renderer {
        highlighter,
        directory,
        anchors: BTreeSet::new(),
    }
    .document(&mut events)
}

fn heading(events: &mut Events<'_>) -> Result<Heading> {
    let mut title = String::new();
    let mut output = Vec::new();
    for event in events.by_ref() {
        match &event {
            Event::End(TagEnd::Heading(_)) => break,
            Event::Text(text) | Event::Code(text) => title.push_str(text),
            Event::SoftBreak | Event::HardBreak => title.push(' '),
            _ => {}
        }
        output.push(render_event(event)?);
    }
    if title.trim().is_empty() {
        return Err("a documentation heading must have a title".into());
    }
    let mut html = String::new();
    html::push_html(&mut html, output.into_iter());
    Ok(Heading { text: title, html })
}

impl Renderer<'_> {
    fn document(mut self, events: &mut Events<'_>) -> Result<Document> {
        if !matches!(
            events.next(),
            Some(Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            }))
        ) {
            return Err("start each guide with a level-one heading (# Title)".into());
        }
        let title = heading(events)?.text;
        let lead = self.body(events)?;
        let mut document = Document {
            title,
            lead: lead.html,
            sections: Vec::new(),
            search: lead.search,
            anchors: BTreeSet::new(),
        };
        while let Some(Event::Start(Tag::Heading { id, .. })) = events.next() {
            let heading = heading(events)?;
            let title = heading.text;
            let id = self.anchor(id.as_deref(), &title)?;
            let mut body = self.body(events)?;
            body.html.insert_str(
                0,
                &format!(
                    "<h2><a href=\"#{id}\">{}<span aria-hidden=\"true\">#</span></a></h2>",
                    heading.html
                ),
            );
            document.search.push_str(&title);
            document.search.push('\n');
            document.search.push_str(&body.search);
            document.sections.push(Section { id, title, body });
        }
        document.anchors = self.anchors;
        Ok(document)
    }

    fn body(&mut self, events: &mut Events<'_>) -> Result<Body> {
        let mut body = Body::default();
        let mut output = Vec::new();
        let mut depth = 0;
        while let Some(event) = events.peek() {
            if depth == 0
                && matches!(
                    event,
                    Event::Start(Tag::Heading {
                        level: HeadingLevel::H2,
                        ..
                    })
                )
            {
                break;
            }
            let event = events.next().expect("peeked Markdown event exists");
            match &event {
                Event::Start(Tag::CodeBlock(kind)) => {
                    output.push(Event::Html(self.code(kind, events, &mut body)?.into()));
                    continue;
                }
                Event::Start(Tag::Heading { level, id, .. }) => {
                    let heading = self.heading(*level, id.as_deref(), events, &mut body)?;
                    output.push(Event::Html(heading.into()));
                    continue;
                }
                Event::Start(_) => depth += 1,
                Event::End(_) => depth -= 1,
                Event::Text(text) | Event::Code(text) => body.search.push_str(text),
                Event::SoftBreak | Event::HardBreak => body.search.push(' '),
                _ => {}
            }
            if matches!(
                event,
                Event::End(TagEnd::Paragraph | TagEnd::Item | TagEnd::TableCell)
            ) {
                body.search.push('\n');
            }
            output.push(render_event(event)?);
        }
        html::push_html(&mut body.html, output.into_iter());
        Ok(body)
    }

    fn heading(
        &mut self,
        level: HeadingLevel,
        id: Option<&str>,
        events: &mut Events<'_>,
        body: &mut Body,
    ) -> Result<String> {
        if level == HeadingLevel::H1 {
            return Err("a guide has one # Title; use ## for sections".into());
        }
        let heading = heading(events)?;
        let id = self.anchor(id, &heading.text)?;
        body.search.push_str(&heading.text);
        body.search.push('\n');
        Ok(format!("<{level} id=\"{id}\">{}</{level}>", heading.html))
    }

    fn anchor(&mut self, explicit: Option<&str>, title: &str) -> Result<String> {
        let base = explicit.map(str::to_owned).unwrap_or_else(|| {
            title
                .to_lowercase()
                .split(|c: char| !c.is_alphanumeric())
                .filter(|word| !word.is_empty())
                .collect::<Vec<_>>()
                .join("-")
        });
        if base.is_empty()
            || !base
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_'))
        {
            return Err(format!("invalid heading id: {base:?}").into());
        }
        let mut id = base.clone();
        let mut suffix = 1;
        while !self.anchors.insert(id.clone()) {
            if explicit.is_some() {
                return Err(format!("duplicate heading id: {id}").into());
            }
            suffix += 1;
            id = format!("{base}-{suffix}");
        }
        Ok(id)
    }

    fn code(
        &self,
        kind: &CodeBlockKind<'_>,
        events: &mut Events<'_>,
        body: &mut Body,
    ) -> Result<String> {
        let info = match kind {
            CodeBlockKind::Fenced(info) => info.as_ref(),
            CodeBlockKind::Indented => "text",
        };
        let (options, title) = info.split_once(" title=").unwrap_or((info, info));
        let mut options = options.split_whitespace();
        let language = options.next().unwrap_or("text");
        let mut code = String::new();
        for event in events.by_ref() {
            match event {
                Event::End(TagEnd::CodeBlock) => break,
                Event::Text(text) => code.push_str(&text),
                _ => {}
            }
        }
        for option in options {
            let path = option
                .strip_prefix("source=")
                .ok_or_else(|| format!("unknown code option: {option}"))?;
            if !code.is_empty() {
                return Err("a source= code fence must be empty".into());
            }
            let path = self.directory.join(path);
            println!("cargo:rerun-if-changed={}", path.display());
            code = fs::read_to_string(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
        }
        body.search.push_str(&code);
        body.code.push_str(&code);
        body.code.push('\n');
        Ok(format!(
            r#"<div class="code-block">
<div class="code-label"><span>{}</span><span aria-hidden="true">• • •</span></div>
<pre tabindex="0" aria-label="{}"><code>{}</code></pre>
</div>"#,
            escape(title),
            escape(title),
            self.highlighter.html(&code, language)?
        ))
    }
}

fn render_event(event: Event<'_>) -> Result<Event<'_>> {
    match event {
        Event::Html(text) | Event::InlineHtml(text) => Ok(if disclosure(&text) {
            Event::Html(text)
        } else {
            Event::Text(text)
        }),
        Event::Start(Tag::Link {
            dest_url, title, ..
        }) => {
            validate_url(&dest_url)?;
            let attributes = if dest_url.starts_with("/docs/source/") || dest_url.ends_with(".md") {
                " target=\"_blank\" rel=\"noopener\""
            } else if dest_url.starts_with("/docs/") {
                " data-fusor-link"
            } else {
                ""
            };
            Ok(Event::Html(
                format!(
                    "<a href=\"{}\" title=\"{}\"{attributes}>",
                    escape(&dest_url),
                    escape(&title)
                )
                .into(),
            ))
        }
        Event::Start(Tag::Image { ref dest_url, .. }) => {
            validate_url(dest_url)?;
            Ok(event)
        }
        Event::Code(text) => Ok(Event::Html(
            format!("<code class=\"inline-code\">{}</code>", escape(&text)).into(),
        )),
        _ => Ok(event),
    }
}

fn disclosure(text: &str) -> bool {
    text.lines().all(|line| {
        let line = line.trim();
        matches!(line, "" | "<details>" | "</details>")
            || line
                .strip_prefix("<summary>")
                .and_then(|line| line.strip_suffix("</summary>"))
                .is_some_and(|label| !label.contains(['<', '>']))
    })
}

fn validate_url(url: &str) -> Result<()> {
    if let Some((scheme, _)) = url.split_once(':') {
        if !scheme.contains(['/', '#', '?'])
            && !matches!(
                scheme.to_ascii_lowercase().as_str(),
                "https" | "http" | "mailto"
            )
        {
            return Err(format!("unsupported documentation link scheme: {scheme}").into());
        }
    }
    Ok(())
}
