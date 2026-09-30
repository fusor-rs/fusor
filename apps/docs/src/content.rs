use crate::code::{CodeData, CodeToken};

#[derive(Clone, Copy, PartialEq)]
pub struct InlineData {
    pub id: usize,
    pub code: bool,
    pub text: &'static str,
}
#[derive(Clone, Copy, PartialEq)]
pub struct ItemData {
    pub id: usize,
    pub spans: &'static [InlineData],
}
/// A paragraph, or a bullet list when `items` is not empty.
#[derive(Clone, Copy, PartialEq)]
pub struct ParagraphData {
    pub id: usize,
    pub spans: &'static [InlineData],
    pub items: &'static [ItemData],
}
#[derive(Clone, Copy, PartialEq)]
pub struct CalloutData {
    pub id: usize,
    pub kind: &'static str,
    pub label: &'static str,
    pub body: &'static [ParagraphData],
}
#[derive(Clone, Copy, PartialEq)]
pub struct TermData {
    pub id: usize,
    pub term: &'static [InlineData],
    pub text: &'static [ParagraphData],
}
#[derive(Clone, Copy, PartialEq)]
pub struct MapCardData {
    pub href: &'static str,
    pub name: &'static str,
    pub kind: &'static str,
    pub text: &'static str,
}
#[derive(Clone, Copy, PartialEq)]
pub struct MapGroupData {
    pub id: usize,
    pub title: &'static str,
    pub side: &'static str,
    pub cards: &'static [MapCardData],
}
#[derive(Clone, Copy, PartialEq)]
pub struct MemberData {
    pub anchor: &'static str,
    pub name: &'static str,
    pub returns: &'static str,
    pub text: &'static [ParagraphData],
    pub signature: &'static [CodeToken],
    pub details: &'static [ParagraphData],
    pub details_label: &'static str,
}
#[derive(Clone, Copy, PartialEq)]
pub struct MemberGroupData {
    pub id: usize,
    pub title: &'static str,
    pub members: &'static [MemberData],
}
/// One type or function on a reference page.
#[derive(Clone, Copy, PartialEq)]
pub struct ApiData {
    pub kind: &'static str,
    pub side: &'static str,
    pub side_label: &'static str,
    pub from: &'static [ParagraphData],
    pub params: &'static [TermData],
    pub groups: &'static [MemberGroupData],
    pub declaration: &'static [CodeToken],
}
#[derive(Clone, Copy, PartialEq)]
pub struct SectionData {
    pub id: &'static str,
    pub title: &'static str,
    pub body_prose: &'static [ParagraphData],
    pub body: &'static str,
    pub code: &'static str,
    pub tokens: &'static [CodeToken],
    pub language: &'static str,
    pub note_prose: &'static [ParagraphData],
    pub note: &'static str,
    pub references: &'static [LinkData],
    pub links: &'static [LinkData],
    /// Zero or one entry, so templates can render it with `ForEach`.
    pub api: &'static [ApiData],
    pub callouts: &'static [CalloutData],
    pub terms: &'static [TermData],
    pub map: &'static [MapGroupData],
    /// Text from the structured fields above, for search.
    pub search: &'static str,
}
pub struct PageData {
    pub parent: Option<&'static str>,
    pub reference: bool,
    pub slug: &'static str,
    pub title: &'static str,
    pub group: &'static str,
    pub lead_prose: &'static [ParagraphData],
    pub lead: &'static str,
    pub sections: &'static [SectionData],
}
include!(concat!(env!("OUT_DIR"), "/content.rs"));
#[derive(Clone, Copy, PartialEq)]
pub struct LinkData {
    pub href: &'static str,
    pub label: &'static str,
}

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

/// Ancestors are ordered from the topic root to the immediate parent.
pub fn ancestors(index: usize) -> Vec<usize> {
    let mut result = Vec::new();
    let mut current = PAGES.get(index).and_then(|page| page.parent);
    while let Some(slug) = current {
        let parent = PAGES
            .iter()
            .position(|page| page.slug == slug)
            .expect("validated parent");
        result.push(parent);
        current = PAGES[parent].parent;
    }
    result.reverse();
    result
}
pub fn children(parent: Option<&str>, group: &str) -> Vec<usize> {
    PAGES
        .iter()
        .enumerate()
        .filter(|(_, page)| page.parent == parent && page.group == group)
        .map(|(index, _)| index)
        .collect()
}
pub fn matches(index: usize, query: &str) -> bool {
    let needle = query.trim().to_lowercase();
    let page = &PAGES[index];
    [page.title, page.group, page.lead]
        .iter()
        .any(|text| text.replace('`', "").to_lowercase().contains(&needle))
        || page.sections.iter().any(|section| {
            [
                section.title,
                section.body,
                section.code,
                section.note,
                section.search,
            ]
            .iter()
            .any(|text| text.replace('`', "").to_lowercase().contains(&needle))
        })
}
