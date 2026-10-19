//! Everything the lab makes, as the index lists it and the stage shows it.
use crate::code::{CodeFile, GREETING_HTML, GREETING_RS, HYPERCMD_BROWSER_HTML, HYPERPANE_RUN};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProjectId {
    Fusor,
    HyperCmd,
    Hyperpane,
    Hughie,
    SqlBomb,
    SpotifyTui,
    SupabaseTui,
}

/// A heading in the index and the projects under it.
#[derive(Clone, Copy, PartialEq)]
pub struct Group {
    pub name: &'static str,
    pub projects: &'static [ProjectId],
}

/// The lab's tools, which turn over to show how they are made, then the things
/// it makes with them.
pub const GROUPS: [Group; 2] = [
    Group {
        name: "Tools",
        projects: &[ProjectId::Fusor, ProjectId::HyperCmd, ProjectId::Hyperpane],
    },
    Group {
        name: "Things",
        projects: &[ProjectId::Hughie, ProjectId::SqlBomb],
    },
];

/// Hobby projects, listed on the "Made with fusor" page rather than the index.
pub const MADE_WITH: [ProjectId; 2] = [ProjectId::SpotifyTui, ProjectId::SupabaseTui];

/// The page's colors while a project is on stage, set in `app.css`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum World {
    Day,
    Night,
}

/// What the frame on stage becomes, set in `app.css`.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    Browser,
    Terminal,
    Phone,
    Face,
}

#[derive(Clone, Copy, PartialEq)]
pub struct Link {
    pub label: &'static str,
    pub href: &'static str,
}

#[derive(PartialEq)]
pub struct Screenshot {
    pub src: &'static str,
    pub description: &'static str,
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

#[derive(Clone, Copy, PartialEq)]
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
        format!("{} {}", MONTHS[usize::from(self.month) - 1], self.day)
    }
}

#[derive(PartialEq)]
pub struct Project {
    pub key: &'static str,
    pub name: &'static str,
    pub kind: &'static str,
    pub status: &'static str,
    pub summary: &'static str,
    /// The last day it shipped.
    pub shipped: Day,
    pub world: World,
    pub shape: Shape,
    pub screenshot: Option<Screenshot>,
    pub links: &'static [Link],
    /// The files that make what the front of the frame shows; empty for things.
    pub sources: &'static [&'static CodeFile],
    /// The tool's mark, for badges on what is built with it.
    pub mark: Option<&'static str>,
    /// The lab's tools this project is built with, shown as badges.
    pub built_with: &'static [ProjectId],
}

static FUSOR_SOURCES: [&CodeFile; 2] = [&GREETING_HTML, &GREETING_RS];
static HYPERCMD_SOURCES: [&CodeFile; 1] = [&HYPERCMD_BROWSER_HTML];
static HYPERPANE_SOURCES: [&CodeFile; 1] = [&HYPERPANE_RUN];

static FUSOR: Project = Project {
    key: "fusor",
    name: "fusor",
    kind: "Web framework",
    status: concat!("v", env!("CARGO_PKG_VERSION")),
    summary: "Reactive web apps. HTML templates and Rust modules, compiled to WebAssembly.",
    shipped: Day {
        year: 2026,
        month: 10,
        day: 18,
    },
    world: World::Day,
    shape: Shape::Browser,
    screenshot: None,
    links: &[
        Link {
            label: "Documentation",
            href: "/docs/",
        },
        Link {
            label: "Examples",
            href: "/docs/showcase",
        },
        Link {
            label: "This page’s source ↗",
            href: "https://github.com/fusor-rs/fusor/tree/main/apps/landing",
        },
    ],
    sources: &FUSOR_SOURCES,
    mark: Some("./brand/fusor.svg"),
    built_with: &[],
};

static HYPERCMD: Project = Project {
    key: "hypercmd",
    name: "HyperCMD",
    kind: "Terminal renderer",
    status: "Released",
    summary: "The same HTML and Rust, laid out in terminal cells, with the keyboard built in.",
    shipped: Day {
        year: 2026,
        month: 10,
        day: 5,
    },
    world: World::Night,
    shape: Shape::Terminal,
    screenshot: Some(Screenshot {
        src: "./lab/hypercmd-file-browser.png",
        description: "HyperCMD’s file browser example: a filter field and a list of Markdown files",
    }),
    links: &[
        Link {
            label: "cmd.fusor.build ↗",
            href: "https://cmd.fusor.build/",
        },
        Link {
            label: "Source ↗",
            href: "https://github.com/fusor-rs/hypercmd",
        },
    ],
    sources: &HYPERCMD_SOURCES,
    mark: Some("./lab/hypercmd-mark.svg"),
    built_with: &[ProjectId::Fusor],
};

static HYPERPANE: Project = Project {
    key: "hyperpane",
    name: "hyperpane",
    kind: "iPhone and Mac apps",
    status: "In development",
    summary: "Your fusor app as an iPhone, iPad and Mac app, from the same build.",
    shipped: Day {
        year: 2026,
        month: 10,
        day: 17,
    },
    world: World::Day,
    shape: Shape::Phone,
    screenshot: None,
    links: &[],
    sources: &HYPERPANE_SOURCES,
    mark: None,
    built_with: &[],
};

static HUGHIE: Project = Project {
    key: "hughie",
    name: "Hughie",
    kind: "AI at home",
    status: "Waitlist open",
    summary: "A private cloud for your family, with AI that runs at home. Everything stays in \
              your house. Coming soon for the web and iPhone.",
    shipped: Day {
        year: 2026,
        month: 10,
        day: 16,
    },
    world: World::Night,
    shape: Shape::Face,
    screenshot: None,
    links: &[Link {
        label: "Join the waitlist at hughie.si ↗",
        href: "https://hughie.si/",
    }],
    sources: &[],
    mark: None,
    built_with: &[ProjectId::Fusor],
};

static SQL_BOMB: Project = Project {
    key: "sql-bomb",
    name: "SQL Bomb",
    kind: "SQL client",
    status: "Released",
    summary: "Query SQL. Watch the rows arrive. A terminal client that streams results into a \
              table.",
    shipped: Day {
        year: 2026,
        month: 10,
        day: 13,
    },
    world: World::Night,
    shape: Shape::Terminal,
    screenshot: Some(Screenshot {
        src: "./lab/sql-bomb-results.svg",
        description: "SQL Bomb showing a query’s results: the SQL editor and a table of cities",
    }),
    links: &[
        Link {
            label: "boom.fusor.build ↗",
            href: "https://boom.fusor.build/",
        },
        Link {
            label: "Source ↗",
            href: "https://github.com/fusor-rs/sql-bomb",
        },
    ],
    sources: &[],
    mark: None,
    built_with: &[ProjectId::HyperCmd],
};

static SPOTIFY_TUI: Project = Project {
    key: "spotify-tui",
    name: "spotify-tui",
    kind: "Music player",
    status: "Hobby project",
    summary: "Play Spotify. Stay in your terminal. Browse your library and play music on macOS \
              and Linux.",
    shipped: Day {
        year: 2026,
        month: 10,
        day: 11,
    },
    world: World::Night,
    shape: Shape::Terminal,
    screenshot: Some(Screenshot {
        src: "./lab/spotify-tui.svg",
        description: "spotify-tui playing a search result, with its track list",
    }),
    links: &[Link {
        label: "Source ↗",
        href: "https://github.com/fusor-rs/spotify-tui",
    }],
    sources: &[],
    mark: None,
    built_with: &[ProjectId::HyperCmd],
};

static SUPABASE_TUI: Project = Project {
    key: "supabase-tui",
    name: "supabase-tui",
    kind: "Supabase console",
    status: "Hobby project",
    summary: "Run Supabase. Stay in your terminal. Health, tables, SQL, users and storage for \
              every project you can access.",
    shipped: Day {
        year: 2026,
        month: 10,
        day: 8,
    },
    world: World::Night,
    shape: Shape::Terminal,
    screenshot: Some(Screenshot {
        src: "./lab/supabase-tui.svg",
        description: "supabase-tui’s Users view, with a list of a project’s users",
    }),
    links: &[Link {
        label: "Source ↗",
        href: "https://github.com/fusor-rs/supabase-tui",
    }],
    sources: &[],
    mark: None,
    built_with: &[ProjectId::HyperCmd],
};

impl ProjectId {
    pub fn project(self) -> &'static Project {
        match self {
            Self::Fusor => &FUSOR,
            Self::HyperCmd => &HYPERCMD,
            Self::Hyperpane => &HYPERPANE,
            Self::Hughie => &HUGHIE,
            Self::SqlBomb => &SQL_BOMB,
            Self::SpotifyTui => &SPOTIFY_TUI,
            Self::SupabaseTui => &SUPABASE_TUI,
        }
    }
}

impl World {
    /// The `data-world` that selects its colors.
    pub fn key(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Night => "night",
        }
    }
}

impl Shape {
    /// The frame's `data-shape`.
    pub fn key(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::Terminal => "terminal",
            Self::Phone => "phone",
            Self::Face => "face",
        }
    }
}
