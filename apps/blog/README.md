# Fusor blog

The blog at fusor.build/blog/: static pages written by fusor's native renderer,
with no WebAssembly or scripts. It is mounted into the site beside the landing
page, docs and benchmarks, and uses [static delivery](../docs/public/content/static-pages.md).

## Write a post

Add a Markdown file to `posts/`. The file name is the post's address:
`posts/first-post.md` is published at `/blog/first-post/`. Use lowercase
letters, digits and hyphens.

Start the file with its front matter between `+++` lines:

```markdown
+++
title = "What we make"
date = 2026-10-20
summary = "One sentence, shown on the list of posts and in link previews."
authors = ["andres"]
banner = { image = "banners/what-we-make.png", alt = "What the image shows" }
+++

The post's text starts here.
```

`authors` names one or more people from `authors.toml`, where each author is
listed once:

```toml
[andres]
name = "Andres Pirela"
link = "https://github.com/andreespirela"   # optional: where the name links
avatar = "authors/andres.jpg"              # optional: a square image under public/
```

Without an avatar, the byline shows the author's initials.

`banner` is optional. Its image is shown wide above the post, as a thumbnail on
the list, and as the picture in link previews, so make it 1200 by 630 pixels.
The `alt` text describes it for people who cannot see it. Images live under
`public/` and are published beside the pages, so `banners/what-we-make.png` is
`public/banners/what-we-make.png`. A missing image stops the build.

The title is the post's only top heading, so the body uses `##` and `###`.
The list shows posts newest first.

The body may contain paragraphs, `##` and `###` headings, bold, italics,
inline code, links, bulleted and numbered lists, quotes, fenced code and
horizontal rules. A list item and a quote each hold one paragraph, and
formatting does not nest. Markdown the blog cannot render faithfully, such as
raw HTML, images, nested lists or a link inside bold text, stops the build with
the file and line, so nothing you write silently disappears.

## Preview it

```sh
just dev
```

Open http://127.0.0.1:4173/blog/. Every edit to a post rebuilds the blog. The
blog uses the site's fusor mark and wordmark from `/brand/`, which the landing
page publishes, so preview it as part of the site.

## How it is built

- `src/main.rs`: the renderer. `fusor build` runs it as
  `--fusor-render-pages DIRECTORY`; it writes `index.html`, the list of posts,
  and `<slug>/index.html` for each post.
- `src/post.rs`: reads each post's front matter and Markdown into the blocks
  the templates render, and rejects what they cannot.
- `web/index.html`: the one document every page renders, with its title,
  description and link-preview tags, and the `Inline` component for text.
- `web/app.css`: Tailwind with the typography plugin. Colors come from the
  landing page's `web/theme.css`, so both share one palette.

`cargo test -p fusor-blog` checks the Markdown and front matter rules, and
renders a post page through the templates.
