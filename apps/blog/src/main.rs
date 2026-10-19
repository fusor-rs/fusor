//! The Fusor blog. `fusor build` runs this program as `--fusor-render-pages
//! DIRECTORY` from the package directory. It reads `posts/*.md` and writes the
//! list of posts and one page per post.
mod post;

use fusor::prelude::*;
use fusor_server::{Context, Render};
use post::{Banner, Block, Post, Span};
use std::{env, error::Error, fs, path::Path};

const BASE: &str = env!("FUSOR_BASE_PATH");
/// Where the blog is published, for the absolute URLs link previews read.
const ORIGIN: &str = "https://fusor.build";
const NAME: &str = "Fusor Blog";
const DESCRIPTION: &str = "Notes from the Fusor lab on what we make, and how.";
const USAGE: &str = "run by fusor build as --fusor-render-pages DIRECTORY";

#[derive(Clone, PartialEq)]
enum View {
    Index(Vec<Post>),
    Post(Post),
}

/// Every page of the blog: the list of posts, or one post.
struct Document {
    title: String,
    description: String,
    /// The page's absolute URL.
    url: String,
    view: View,
}

impl Document {
    fn index(posts: &[Post]) -> Self {
        Self {
            title: NAME.to_owned(),
            description: DESCRIPTION.to_owned(),
            url: format!("{ORIGIN}{BASE}"),
            view: View::Index(posts.to_vec()),
        }
    }

    fn post(post: &Post) -> Self {
        Self {
            title: format!("{} · {NAME}", post.title),
            description: post.summary.clone(),
            url: format!("{ORIGIN}{}", post_url(&post.slug)),
            view: View::Post(post.clone()),
        }
    }

    /// The Open Graph type that link previews read.
    fn kind(&self) -> &'static str {
        match self.view {
            View::Index(_) => "website",
            View::Post(_) => "article",
        }
    }

    /// The image link previews show: a post's banner.
    fn banner(&self) -> Option<Banner> {
        match &self.view {
            View::Index(_) => None,
            View::Post(post) => post.banner.clone(),
        }
    }

    /// A large preview card when there is an image to fill it.
    fn card(&self) -> &'static str {
        if self.banner().is_some() {
            "summary_large_image"
        } else {
            "summary"
        }
    }
}

/// The buttons that share a post: the system share sheet, and X.
#[derive(FromInputs)]
struct Share {
    #[input]
    title: String,
    /// The post's absolute URL.
    #[input]
    url: String,
}

impl Share {
    /// X's compose page, with the post's title and URL filled in.
    fn on_x(&self) -> String {
        let query = fusor_router::encode_query([("text", self.title.as_str()), ("url", &self.url)]);
        format!("https://x.com/intent/post?{query}")
    }
}

/// A run of styled text inside a heading, paragraph, quote or list item.
#[derive(FromInputs)]
struct Inline {
    #[input]
    spans: Vec<Span>,
}

fn post_url(slug: &str) -> String {
    format!("{BASE}{slug}/")
}

/// Names an element so that it morphs into the element of the same name on the
/// next page: a post's banner and title on the list, and the same on its page.
fn morph(part: &str, slug: &str) -> String {
    format!("view-transition-name: {part}-{slug}")
}

/// Where a file under `public/` is published.
fn asset(path: &str) -> String {
    format!("{BASE}{path}")
}

/// The same, as an absolute URL for link previews.
fn absolute(path: &str) -> String {
    format!("{ORIGIN}{}", asset(path))
}

/// Each value with its position, the key a `ForEach` row needs.
fn numbered<T: Clone>(values: &[T]) -> Vec<(usize, T)> {
    values.iter().cloned().enumerate().collect()
}

fn write(path: &Path, document: &Document) -> Result<(), Box<dyn Error>> {
    let html = document.render(&mut Context::new())?;
    fs::write(path, format!("<!doctype html>\n{html}"))?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    let [flag, directory] = args.as_slice() else {
        return Err(USAGE.into());
    };
    if flag != "--fusor-render-pages" {
        return Err(USAGE.into());
    }
    let directory = Path::new(directory);
    // fusor build runs this program in the blog's package directory.
    let posts = post::read_all(Path::new(""))?;
    write(&directory.join("index.html"), &Document::index(&posts))?;
    for post in &posts {
        let page = directory.join(&post.slug);
        fs::create_dir(&page)?;
        write(&page.join("index.html"), &Document::post(post))?;
    }
    Ok(())
}

fusor::bindings!(app);
include!(env!("FUSOR_MODULE"));

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn render(document: &Document) -> String {
        document.render(&mut Context::new()).unwrap().into_string()
    }

    /// A blog of one post by two authors, one with an avatar and a link. The
    /// post's front matter ends with `banner`.
    fn blog(name: &str, banner: &str) -> PathBuf {
        let blog = env::temp_dir().join(format!("fusor-blog-{name}-{}", std::process::id()));
        for directory in ["posts", "public/banners", "public/authors"] {
            fs::create_dir_all(blog.join(directory)).unwrap();
        }
        fs::write(blog.join("public/banners/first-post.png"), "image").unwrap();
        fs::write(blog.join("public/authors/ana.jpg"), "image").unwrap();
        fs::write(
            blog.join("authors.toml"),
            "[ana]\nname = \"Ana Lima\"\nlink = \"https://example.com/ana\"\navatar = \"authors/ana.jpg\"\n\
             [ben]\nname = \"Ben Wu\"\n",
        )
        .unwrap();
        fs::write(
            blog.join("posts/first-post.md"),
            format!(
                "+++\ntitle = \"Tools & <things>\"\ndate = 2026-10-20\nsummary = \"Why a < b.\"\n\
                 authors = [\"ana\", \"ben\"]\n{banner}\n+++\n\
                 ## Why HTML\n\nRead [the docs](/docs/) and keep `x < y`.\n"
            ),
        )
        .unwrap();
        blog
    }

    #[test]
    fn each_post_is_its_own_page_with_its_own_preview_and_escaped_text() {
        let blog = blog(
            "pages",
            "banner = { image = \"banners/first-post.png\", alt = \"A <title> tag\" }",
        );
        let posts = post::read_all(&blog).unwrap();
        fs::remove_dir_all(&blog).unwrap();

        let page = render(&Document::post(&posts[0]));
        for expected in [
            "<title data-fusor-text=\"0\">Tools &amp; &lt;things&gt; · Fusor Blog</title>",
            "property=\"og:type\" content=\"article\"",
            "property=\"og:url\" content=\"https://fusor.build/blog/first-post/\"",
            "rel=\"canonical\" href=\"https://fusor.build/blog/first-post/\"",
            "name=\"description\" content=\"Why a &lt; b.\"",
            "datetime=\"2026-10-20\"",
            ">October 20, 2026</time>",
            "id=\"why-html\"",
            ">the docs</a>",
            "x &lt; y</code>",
            "name=\"twitter:card\" content=\"summary_large_image\"",
            "property=\"og:image\" content=\"https://fusor.build/blog/banners/first-post.png\"",
            "property=\"og:image:alt\" content=\"A &lt;title&gt; tag\"",
            "src=\"/blog/banners/first-post.png\"",
            "alt=\"A &lt;title&gt; tag\"",
            "src=\"/blog/authors/ana.jpg\"",
            "href=\"https://example.com/ana\"",
            ">Ana Lima</a>",
            ">BW</span>",
            ">Ben Wu</span>",
            "href=\"https://x.com/intent/post?text=Tools+%26+%3Cthings%3E&amp;url=https%3A%2F%2Ffusor.build%2Fblog%2Ffirst-post%2F\"",
            "hidden=\"\" type=\"button\" data-title=\"Tools &amp; &lt;things&gt;\" data-url=\"https://fusor.build/blog/first-post/\"",
        ] {
            assert!(page.contains(expected), "{expected} in {page}");
        }
        assert!(!page.contains("<things>"));

        // The list and the post name the same banner and title, so one morphs
        // into the other.
        let index = render(&Document::index(&posts));
        for pair in [
            "style=\"view-transition-name: banner-first-post\"",
            "style=\"view-transition-name: title-first-post\"",
        ] {
            assert_eq!(
                (index.matches(pair).count(), page.matches(pair).count()),
                (1, 1),
                "{pair}"
            );
        }
        assert!(index.contains("href=\"/blog/first-post/\""), "{index}");
        assert!(index.contains("property=\"og:type\" content=\"website\""));
        assert!(index.contains(">Ana Lima and Ben Wu<"), "{index}");
        assert!(index.contains("name=\"twitter:card\" content=\"summary\""));
        assert!(!index.contains("og:image"));
        assert!(!index.contains("The first post is on its way."));
        assert!(render(&Document::index(&[])).contains("The first post is on its way."));
    }

    #[test]
    fn an_image_missing_from_public_stops_the_build() {
        let blog = blog(
            "missing",
            "banner = { image = \"banners/missing.png\", alt = \"x\" }",
        );
        let error = post::read_all(&blog).map(drop).unwrap_err().to_string();
        fs::remove_dir_all(&blog).unwrap();
        assert!(
            error.contains("the image banners/missing.png is not a file under public/"),
            "{error}"
        );
    }
}
