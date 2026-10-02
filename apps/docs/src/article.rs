use crate::{
    code::{CodeBlock, CodeData, Signature},
    content::{
        ApiData, CalloutData, InlineData, LinkData, MapGroupData, MemberData, PAGES, PageData,
        ParagraphData, SectionData, TermData, ancestors,
    },
    routes::href,
};
use fusor::prelude::*;
static MISSING: PageData = PageData {
    parent: None,
    reference: false,
    slug: "missing",
    title: "Page not found",
    group: "Documentation",
    lead: "That page is not in this edition of the docs. Choose a topic in the sidebar or return to the introduction.",
    lead_prose: &[ParagraphData {
        id: 0,
        spans: &[InlineData {
            id: 0,
            code: false,
            text: "That page is not in this edition of the docs. Choose a topic in the sidebar or return to the introduction.",
        }],
        items: &[],
    }],
    sections: &[],
};
pub struct Article {
    page: &'static PageData,
    index: usize,
    count: Signal<i32>,
}
impl Article {
    pub fn new(index: Option<usize>) -> Self {
        let page = index.and_then(|index| PAGES.get(index)).unwrap_or(&MISSING);
        if let Ok(document) = fusor::dom::document() {
            document.set_title(&format!("{} · fusor", page.title));
        }
        Self {
            page,
            index: index.unwrap_or(usize::MAX),
            count: signal(0),
        }
    }
    fn breadcrumbs(&self) -> Vec<LinkData> {
        ancestors(self.index)
            .into_iter()
            .map(|index| LinkData {
                href: PAGES[index].slug,
                label: PAGES[index].title,
            })
            .collect()
    }
    fn sections(&self) -> Vec<SectionData> {
        self.page.sections.to_vec()
    }
    fn previous(&self) -> usize {
        self.index.saturating_sub(1).min(PAGES.len() - 1)
    }
    fn next(&self) -> usize {
        self.index.saturating_add(1).min(PAGES.len() - 1)
    }
}
#[derive(FromInputs)]
struct Prose {
    #[input]
    paragraphs: &'static [ParagraphData],
}
#[derive(FromInputs)]
struct Spans {
    #[input]
    spans: &'static [InlineData],
}
#[derive(FromInputs)]
struct Terms {
    #[input]
    items: &'static [TermData],
}

struct Breadcrumb {
    item: fusor::Memo<LinkData>,
}
struct Section {
    item: fusor::Memo<SectionData>,
}
struct Toc {
    item: fusor::Memo<SectionData>,
}
struct RelatedLink {
    item: fusor::Memo<LinkData>,
}
struct Callout {
    item: fusor::Memo<CalloutData>,
}
struct MapGroup {
    item: fusor::Memo<MapGroupData>,
}
struct ApiBody {
    item: fusor::Memo<ApiData>,
}
struct Member {
    item: fusor::Memo<MemberData>,
}
fusor::bindings!(article);

// Keyed rows receive their item as the whole input.
macro_rules! row_inputs {
    ($($row:ty),*) => {$(
        impl fusor::dom::FromInputs for $row {
            type Error = fusor::dom::JsValue;
            type Inputs = Self;
            fn from_inputs(inputs: Self, _owner: fusor::OwnerHandle) -> Result<Self, fusor::dom::JsValue> {
                Ok(inputs)
            }
        }
    )*};
}
row_inputs!(Callout, MapGroup, ApiBody, Member);

impl fusor::dom::FromInputs for Breadcrumb {
    type Inputs = Self;
    type Error = fusor::dom::JsValue;
    fn from_inputs(inputs: Self, _owner: fusor::OwnerHandle) -> Result<Self, fusor::dom::JsValue> {
        Ok(inputs)
    }
}

impl fusor::dom::FromInputs for Section {
    type Inputs = Self;
    type Error = fusor::dom::JsValue;
    fn from_inputs(inputs: Self, _owner: fusor::OwnerHandle) -> Result<Self, fusor::dom::JsValue> {
        Ok(inputs)
    }
}

impl fusor::dom::FromInputs for Toc {
    type Inputs = Self;
    type Error = fusor::dom::JsValue;
    fn from_inputs(inputs: Self, _owner: fusor::OwnerHandle) -> Result<Self, fusor::dom::JsValue> {
        Ok(inputs)
    }
}

impl fusor::dom::FromInputs for RelatedLink {
    type Inputs = Self;
    type Error = fusor::dom::JsValue;
    fn from_inputs(inputs: Self, _owner: fusor::OwnerHandle) -> Result<Self, fusor::dom::JsValue> {
        Ok(inputs)
    }
}

pub struct ArticleInputs {
    pub index: Option<usize>,
}
impl fusor::dom::FromInputs for Article {
    type Inputs = ArticleInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(inputs: Self::Inputs, _: OwnerHandle) -> Result<Self, wasm_bindgen::JsValue> {
        Ok(Self::new(inputs.index))
    }
}
