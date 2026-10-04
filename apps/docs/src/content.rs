use crate::code::{CodeData, CodeToken};
use docs_base::Site;

include!(concat!(env!("OUT_DIR"), "/content.rs"));

pub static SITE: Site = Site {
    name: "fusor",
    base_path: "/docs/",
    logo: "/docs/favicon.svg",
    version: "v0.1 dev",
    pages: PAGES,
};

#[derive(Clone, Copy, PartialEq)]
pub struct DemoData {
    pub slug: &'static str,
    pub title: &'static str,
    pub category: &'static str,
    pub description: &'static str,
    pub try_it: &'static str,
    pub explanation: &'static str,
    pub guide: &'static str,
    pub guide_label: &'static str,
    pub preview_title: &'static str,
    pub preview_value: &'static str,
    pub preview_detail: &'static str,
    /// Runs background workers; the demo gets the full width, as JavaScript demos do.
    pub workers: bool,
    pub rust: CodeData,
    pub html: CodeData,
    pub javascript: Option<CodeData>,
}
