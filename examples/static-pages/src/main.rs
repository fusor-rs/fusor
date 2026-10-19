//! A static site: this program writes every page, and nothing runs in the
//! browser. `fusor build` runs it as `--fusor-render-pages DIRECTORY`.
use fusor_server::{Context, Render};
use std::{env, error::Error, fs, path::Path};

const BASE: &str = env!("FUSOR_BASE_PATH");
const USAGE: &str = "run by fusor build as --fusor-render-pages DIRECTORY";

#[derive(Clone, Copy, PartialEq)]
struct Note {
    slug: &'static str,
    title: &'static str,
    text: &'static str,
}

const NOTES: &[Note] = &[
    Note {
        slug: "html-first",
        title: "HTML first",
        text: "Each page is an HTML template, rendered by Rust before anyone visits.",
    },
    Note {
        slug: "nothing-to-load",
        title: "Nothing to load",
        text: "Static delivery publishes documents and a stylesheet: no WebAssembly, no scripts.",
    },
];

/// Every page: the index lists the notes, and a note's page lists none.
struct Document {
    title: String,
    heading: &'static str,
    text: &'static str,
    notes: Vec<Note>,
}

impl Document {
    fn index() -> Self {
        Self {
            title: "Field notes".to_owned(),
            heading: "Field notes",
            text: "Short notes on writing a site with fusor's static delivery.",
            notes: NOTES.to_vec(),
        }
    }

    fn note(note: &Note) -> Self {
        Self {
            title: format!("{} · Field notes", note.title),
            heading: note.title,
            text: note.text,
            notes: Vec::new(),
        }
    }
}

fn note_url(slug: &str) -> String {
    format!("{BASE}notes/{slug}/")
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
    write(&directory.join("index.html"), &Document::index())?;
    for note in NOTES {
        let page = directory.join("notes").join(note.slug);
        fs::create_dir_all(&page)?;
        write(&page.join("index.html"), &Document::note(note))?;
    }
    fs::write(directory.join("robots.txt"), "User-agent: *\nAllow: /\n")?;
    Ok(())
}

fusor::bindings!(app);
include!(env!("FUSOR_MODULE"));
