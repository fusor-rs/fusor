//! A post: its front matter and its Markdown body, read into the blocks the
//! blog's templates render. Markdown the blog cannot render faithfully stops
//! the build at its line, so nothing an author writes silently disappears.
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use serde::Deserialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fs, io,
    ops::Range,
    path::{Component, Path},
};

const FENCE: &str = "+++\n";
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// What a post states between its `+++` lines.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrontMatter {
    title: String,
    date: toml::value::Datetime,
    summary: String,
    /// Ids from `authors.toml`.
    authors: Vec<String>,
    banner: Option<Banner>,
}

#[derive(Clone, PartialEq)]
pub struct Post {
    pub slug: String,
    pub title: String,
    pub date: Day,
    pub summary: String,
    pub authors: Vec<Author>,
    pub banner: Option<Banner>,
    pub blocks: Vec<Block>,
}

impl Post {
    /// The authors' names as a phrase: "A", "A and B", "A, B and C".
    pub fn byline(&self) -> String {
        let names: Vec<&str> = self
            .authors
            .iter()
            .map(|author| author.name.as_str())
            .collect();
        let (last, rest) = names
            .split_last()
            .expect("parse rejects a post without authors");
        if rest.is_empty() {
            (*last).to_owned()
        } else {
            format!("{} and {last}", rest.join(", "))
        }
    }

    /// Every image the post shows, as a path under `public/`.
    fn images(&self) -> impl Iterator<Item = &str> {
        self.banner
            .iter()
            .map(|banner| banner.image.as_str())
            .chain(
                self.authors
                    .iter()
                    .filter_map(|author| author.avatar.as_deref()),
            )
    }
}

/// Someone who writes posts, listed once in `authors.toml` under an id.
#[derive(Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Author {
    pub name: String,
    /// Where the author's name links, such as a profile.
    pub link: Option<String>,
    /// A square photo under `public/`; without one the byline shows initials.
    pub avatar: Option<String>,
}

impl Author {
    pub fn initials(&self) -> String {
        self.name
            .split_whitespace()
            .filter_map(|word| word.chars().next())
            .take(2)
            .collect()
    }
}

/// The image at the top of a post and in its link previews, under `public/`.
#[derive(Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Banner {
    pub image: String,
    /// What the image shows, for people who cannot see it.
    pub alt: String,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Day {
    year: u16,
    month: u8,
    day: u8,
}

impl Day {
    /// The `datetime` of its `<time>` element.
    pub fn datetime(self) -> String {
        format!("{}-{:02}-{:02}", self.year, self.month, self.day)
    }

    pub fn label(self) -> String {
        format!("{} {}, {}", self.month_name(), self.day, self.year)
    }

    /// The numeral on the list of posts, with the month and year beneath it.
    pub fn day_of_month(self) -> u8 {
        self.day
    }

    /// "Oct 2026".
    pub fn month_and_year(self) -> String {
        format!("{} {}", &self.month_name()[..3], self.year)
    }

    fn month_name(self) -> &'static str {
        MONTHS[usize::from(self.month) - 1]
    }
}

#[derive(Clone, PartialEq)]
pub enum Block {
    /// `##`, with the anchor its `id` links to.
    Heading {
        anchor: String,
        text: Vec<Span>,
    },
    /// `###`.
    Subheading {
        anchor: String,
        text: Vec<Span>,
    },
    Paragraph(Vec<Span>),
    Quote(Vec<Span>),
    BulletList(Vec<Vec<Span>>),
    NumberedList(Vec<Vec<Span>>),
    Code(String),
    Rule,
}

/// Inline text in one style; styles do not nest.
#[derive(Clone, PartialEq)]
pub enum Span {
    Text(String),
    Strong(String),
    Emphasis(String),
    Code(String),
    Link { href: String, text: String },
}

/// Every post in the blog's `posts/`, newest first, with the authors it names
/// from `authors.toml` and the images it shows found under `public/`.
pub fn read_all(blog: &Path) -> Result<Vec<Post>, Box<dyn Error>> {
    // Git keeps no empty directory, so posts/ is absent until the first post.
    let entries = match fs::read_dir(blog.join("posts")) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    let authors_path = blog.join("authors.toml");
    let authors: BTreeMap<String, Author> = toml::from_str(&fs::read_to_string(&authors_path)?)
        .map_err(|error| format!("{}: {}", authors_path.display(), error.message()))?;
    let mut posts = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.extension().is_none_or(|extension| extension != "md") {
            continue;
        }
        let source = fs::read_to_string(&path)?;
        let post = parse(slug(&path)?, &source, &authors)
            .map_err(|(line, message)| format!("{}:{line}: {message}", path.display()))?;
        for image in post.images() {
            if !inside_public(image) || !blog.join("public").join(image).is_file() {
                return Err(format!(
                    "{}: the image {image} is not a file under public/, such as banners/first-post.png",
                    path.display()
                )
                .into());
            }
        }
        posts.push(post);
    }
    posts.sort_by(|a, b| b.date.cmp(&a.date).then_with(|| a.slug.cmp(&b.slug)));
    Ok(posts)
}

/// Whether an image path stays inside `public/`, which is published beside the pages.
fn inside_public(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// The file name is the post's URL path, so it is limited to what reads well there.
fn slug(path: &Path) -> Result<String, String> {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("");
    let valid = !stem.is_empty()
        && stem
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if valid {
        Ok(stem.to_owned())
    } else {
        Err(format!(
            "{}: name posts with lowercase letters, digits and hyphens, such as first-post.md",
            path.display()
        ))
    }
}

/// A problem in a post, at its 1-based line.
type Problem = (usize, String);

fn parse(slug: String, source: &str, authors: &BTreeMap<String, Author>) -> Result<Post, Problem> {
    let unopened = || {
        (
            1,
            "a post begins with front matter between +++ lines".to_owned(),
        )
    };
    let rest = source.strip_prefix(FENCE).ok_or_else(unopened)?;
    let (front, body) = rest
        .split_once(&format!("\n{FENCE}"))
        .ok_or_else(|| (1, "the front matter needs a closing +++ line".to_owned()))?;
    let first_line = 1 + front.lines().count() + 2;
    let front: FrontMatter =
        toml::from_str(front).map_err(|error| (2, error.message().to_owned()))?;
    let date = match (front.date.date, front.date.time, front.date.offset) {
        (Some(date), None, None) => Day {
            year: date.year,
            month: date.month,
            day: date.day,
        },
        _ => {
            return Err((
                2,
                "date must be a day, such as date = 2026-10-20".to_owned(),
            ));
        }
    };
    if front.authors.is_empty() {
        return Err((
            2,
            "name at least one author, such as authors = [\"andres\"]".to_owned(),
        ));
    }
    let authors = front
        .authors
        .iter()
        .map(|id| {
            authors
                .get(id)
                .cloned()
                .ok_or_else(|| (2, format!("{id} is not an author; add it to authors.toml")))
        })
        .collect::<Result<_, _>>()?;
    let blocks = Reader::new(body, first_line).blocks()?;
    Ok(Post {
        slug,
        title: front.title,
        date,
        summary: front.summary,
        authors,
        banner: front.banner,
        blocks,
    })
}

struct Reader<'a> {
    body: &'a str,
    first_line: usize,
    events: std::iter::Peekable<
        pulldown_cmark::OffsetIter<'a, pulldown_cmark::DefaultBrokenLinkCallback>,
    >,
    anchors: BTreeSet<String>,
}

impl<'a> Reader<'a> {
    fn new(body: &'a str, first_line: usize) -> Self {
        Self {
            body,
            first_line,
            events: Parser::new(body).into_offset_iter().peekable(),
            anchors: BTreeSet::new(),
        }
    }

    fn problem(&self, range: &Range<usize>, message: &str) -> Problem {
        let line = self.first_line + self.body[..range.start].matches('\n').count();
        (line, message.to_owned())
    }

    fn next(&mut self) -> Result<(Event<'a>, Range<usize>), Problem> {
        self.events.next().ok_or_else(|| {
            let end = self.body.len()..self.body.len();
            self.problem(&end, "the post ends inside an unfinished block")
        })
    }

    fn blocks(mut self) -> Result<Vec<Block>, Problem> {
        let mut blocks = Vec::new();
        while let Some((event, range)) = self.events.next() {
            blocks.push(match event {
                Event::Start(Tag::Paragraph) => Block::Paragraph(self.inline(TagEnd::Paragraph)?),
                Event::Start(Tag::Heading { level, .. }) => {
                    let text = self.inline(TagEnd::Heading(level))?;
                    let anchor = self.anchor(&text);
                    match level {
                        HeadingLevel::H2 => Block::Heading { anchor, text },
                        HeadingLevel::H3 => Block::Subheading { anchor, text },
                        _ => {
                            return Err(self.problem(
                                &range,
                                "use ## and ### headings; the front matter title is the post's only top heading",
                            ));
                        }
                    }
                }
                Event::Start(Tag::BlockQuote(_)) => self.quote(&range)?,
                Event::Start(Tag::List(first)) => {
                    let items = self.items(&range)?;
                    if first.is_some() {
                        Block::NumberedList(items)
                    } else {
                        Block::BulletList(items)
                    }
                }
                Event::Start(Tag::CodeBlock(_)) => self.code()?,
                Event::Rule => Block::Rule,
                Event::Start(Tag::HtmlBlock) | Event::Html(_) | Event::InlineHtml(_) => {
                    return Err(self.problem(&range, "posts cannot contain raw HTML"));
                }
                _ => return Err(self.problem(&range, "this Markdown is not supported in posts")),
            });
        }
        Ok(blocks)
    }

    fn quote(&mut self, range: &Range<usize>) -> Result<Block, Problem> {
        let one_paragraph = "a quote holds one paragraph";
        if !matches!(self.next()?.0, Event::Start(Tag::Paragraph)) {
            return Err(self.problem(range, one_paragraph));
        }
        let text = self.inline(TagEnd::Paragraph)?;
        match self.next()?.0 {
            Event::End(TagEnd::BlockQuote(_)) => Ok(Block::Quote(text)),
            _ => Err(self.problem(range, one_paragraph)),
        }
    }

    /// A list's items, each one line of text; loose items hold one paragraph.
    fn items(&mut self, range: &Range<usize>) -> Result<Vec<Vec<Span>>, Problem> {
        let mut items = Vec::new();
        loop {
            match self.next()? {
                (Event::End(TagEnd::List(_)), _) => return Ok(items),
                (Event::Start(Tag::Item), _) => {}
                (_, at) => {
                    return Err(self.problem(&at, "this Markdown is not supported in a list"));
                }
            }
            if matches!(self.events.peek(), Some((Event::Start(Tag::Paragraph), _))) {
                self.next()?;
                items.push(self.inline(TagEnd::Paragraph)?);
                if !matches!(self.next()?.0, Event::End(TagEnd::Item)) {
                    return Err(self.problem(
                        range,
                        "a list item holds one paragraph, without nested lists",
                    ));
                }
            } else {
                items.push(self.inline(TagEnd::Item)?);
            }
        }
    }

    fn code(&mut self) -> Result<Block, Problem> {
        let mut text = String::new();
        loop {
            match self.next()? {
                (Event::Text(part), _) => text.push_str(&part),
                (Event::End(TagEnd::CodeBlock), _) => return Ok(Block::Code(text)),
                (_, at) => return Err(self.problem(&at, "this Markdown is not supported in code")),
            }
        }
    }

    /// Text until `end`, in spans of one style each.
    fn inline(&mut self, end: TagEnd) -> Result<Vec<Span>, Problem> {
        let mut spans: Vec<Span> = Vec::new();
        loop {
            let (event, range) = self.next()?;
            let span = match event {
                Event::End(tag) if tag == end => return Ok(spans),
                Event::Text(text) => Span::Text(text.into_string()),
                Event::SoftBreak | Event::HardBreak => Span::Text(" ".to_owned()),
                Event::Code(code) => Span::Code(code.into_string()),
                Event::Start(Tag::Strong) => Span::Strong(self.plain(TagEnd::Strong)?),
                Event::Start(Tag::Emphasis) => Span::Emphasis(self.plain(TagEnd::Emphasis)?),
                Event::Start(Tag::Link { dest_url, .. }) => Span::Link {
                    href: dest_url.into_string(),
                    text: self.plain(TagEnd::Link)?,
                },
                Event::Start(Tag::Image { .. }) => {
                    return Err(self.problem(&range, "posts cannot contain images yet"));
                }
                Event::Start(Tag::List(_)) => {
                    return Err(self.problem(&range, "lists cannot be nested"));
                }
                Event::Html(_) | Event::InlineHtml(_) => {
                    return Err(self.problem(&range, "posts cannot contain raw HTML"));
                }
                _ => return Err(self.problem(&range, "this Markdown is not supported in text")),
            };
            match (spans.last_mut(), span) {
                (Some(Span::Text(previous)), Span::Text(text)) => previous.push_str(&text),
                (_, span) => spans.push(span),
            }
        }
    }

    /// The text inside bold, italic or a link, which holds no other formatting.
    fn plain(&mut self, end: TagEnd) -> Result<String, Problem> {
        let mut text = String::new();
        loop {
            match self.next()? {
                (Event::End(tag), _) if tag == end => return Ok(text),
                (Event::Text(part), _) => text.push_str(&part),
                (Event::SoftBreak | Event::HardBreak, _) => text.push(' '),
                (_, at) => {
                    return Err(self.problem(
                        &at,
                        "formatting cannot be nested, such as bold inside a link",
                    ));
                }
            }
        }
    }

    /// A heading's `id`: its words, lowercased and joined by hyphens, and
    /// numbered when an earlier heading has the same words.
    fn anchor(&mut self, text: &[Span]) -> String {
        let words: String = text
            .iter()
            .map(|span| match span {
                Span::Text(text)
                | Span::Strong(text)
                | Span::Emphasis(text)
                | Span::Code(text)
                | Span::Link { text, .. } => text.as_str(),
            })
            .collect::<String>()
            .to_lowercase();
        let base: Vec<&str> = words
            .split(|character: char| !character.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect();
        let base = base.join("-");
        let mut anchor = base.clone();
        let mut number = 2;
        while !self.anchors.insert(anchor.clone()) {
            anchor = format!("{base}-{number}");
            number += 1;
        }
        anchor
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn authors() -> BTreeMap<String, Author> {
        BTreeMap::from([(
            "andres".to_owned(),
            Author {
                name: "Andres Pirela".to_owned(),
                link: None,
                avatar: None,
            },
        )])
    }

    /// A post whose body starts on line 7, after five lines of front matter.
    fn post(body: &str) -> Result<Post, Problem> {
        parse(
            "first-post".to_owned(),
            &format!(
                "+++\ntitle = \"First\"\ndate = 2026-10-20\nsummary = \"Hello.\"\nauthors = [\"andres\"]\n+++\n{body}"
            ),
            &authors(),
        )
    }

    fn text(value: &str) -> Span {
        Span::Text(value.to_owned())
    }

    #[test]
    fn front_matter_and_markdown_become_the_post() {
        let post = post(
            "Write the **interface** in *HTML* and [keep](https://fusor.build) `Rust`\nnearby.\n\n\
             ## Why HTML\n\n### Why HTML\n\n> Less to learn.\n\n- one\n- *two*\n\n1. first\n\n\
             ```rust\nlet x = 1;\n```\n\n---\n",
        )
        .unwrap();
        assert_eq!(post.title, "First");
        assert_eq!(post.summary, "Hello.");
        assert_eq!(post.date.datetime(), "2026-10-20");
        assert_eq!(post.date.label(), "October 20, 2026");
        assert!(
            post.blocks
                == vec![
                    Block::Paragraph(vec![
                        text("Write the "),
                        Span::Strong("interface".to_owned()),
                        text(" in "),
                        Span::Emphasis("HTML".to_owned()),
                        text(" and "),
                        Span::Link {
                            href: "https://fusor.build".to_owned(),
                            text: "keep".to_owned()
                        },
                        text(" "),
                        Span::Code("Rust".to_owned()),
                        text(" nearby."),
                    ]),
                    Block::Heading {
                        anchor: "why-html".to_owned(),
                        text: vec![text("Why HTML")]
                    },
                    Block::Subheading {
                        anchor: "why-html-2".to_owned(),
                        text: vec![text("Why HTML")]
                    },
                    Block::Quote(vec![text("Less to learn.")]),
                    Block::BulletList(vec![
                        vec![text("one")],
                        vec![Span::Emphasis("two".to_owned())]
                    ]),
                    Block::NumberedList(vec![vec![text("first")]]),
                    Block::Code("let x = 1;\n".to_owned()),
                    Block::Rule,
                ]
        );
    }

    #[test]
    fn markdown_the_blog_cannot_render_stops_at_its_line() {
        for (body, line, message) in [
            ("# Title\n", 7, "use ## and ### headings"),
            (
                "Text\n\n<div>raw</div>\n",
                9,
                "posts cannot contain raw HTML",
            ),
            (
                "A ![logo](logo.png)\n",
                7,
                "posts cannot contain images yet",
            ),
            ("**bold [link](/)**\n", 7, "formatting cannot be nested"),
            ("- one\n  - nested\n", 8, "lists cannot be nested"),
            ("> one\n>\n> two\n", 7, "a quote holds one paragraph"),
            (
                "1. first\n\n   second\n",
                7,
                "a list item holds one paragraph",
            ),
        ] {
            let (found_line, found) = post(body).map(drop).unwrap_err();
            assert!(found.starts_with(message), "{body}: {found}");
            assert_eq!(found_line, line, "{body}");
        }
        let front = |extra: &str| {
            format!("+++\ntitle = \"x\"\ndate = 2026-10-20\nsummary = \"s\"\n{extra}+++\n")
        };
        for (source, message) in [
            (
                "title = \"x\"\n".to_owned(),
                "a post begins with front matter",
            ),
            (
                "+++\ntitle = \"x\"\n".to_owned(),
                "the front matter needs a closing +++ line",
            ),
            (
                front("authors = [\"andres\"]\nauthor = \"a\"\n"),
                "unknown field",
            ),
            (
                front("authors = [\"andres\"]\n").replace("2026-10-20", "2026-10-20T09:00:00"),
                "date must be a day",
            ),
            (front(""), "missing field `authors`"),
            (front("authors = []\n"), "name at least one author"),
            (
                front("authors = [\"ana\"]\n"),
                "ana is not an author; add it to authors.toml",
            ),
            (
                front("authors = [\"andres\"]\nbanner = { image = \"banners/x.png\" }\n"),
                "missing field `alt`",
            ),
        ] {
            let (_, found) = parse("x".to_owned(), &source, &authors())
                .map(drop)
                .unwrap_err();
            assert!(found.contains(message), "{source}: {found}");
        }
    }
}
