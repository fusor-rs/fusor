use crate::{
    article::Article,
    content::{DEMOS, PAGES, ancestors, children, matches},
    routes::{Page, href},
    showcase::{DemoPage, Gallery},
};
use fusor::{dom::document, prelude::*};
use fusor_router::{Route, browser::declarative::Navigation};
use wasm_bindgen::JsCast;

pub struct App {
    query: Signal<String>,
    menu: Signal<bool>,
    dark: Signal<bool>,
    active: Signal<usize>,
}
impl App {
    fn new() -> Self {
        let dark = web_sys::window()
            .and_then(|window| window.local_storage().ok().flatten())
            .and_then(|storage| storage.get_item("fusor-docs-theme").ok().flatten())
            .is_some_and(|value| value == "dark");
        Self {
            query: signal(String::new()),
            menu: signal(false),
            dark: signal(dark),
            active: signal(0),
        }
    }
    fn theme(&self) {
        self.dark.update(|value| *value = !*value);
        if let Some(storage) =
            web_sys::window().and_then(|window| window.local_storage().ok().flatten())
        {
            let _ = storage.set_item(
                "fusor-docs-theme",
                if self.dark.get() { "dark" } else { "light" },
            );
        }
    }
    fn keyboard(&self, event: web_sys::Event) {
        if let Some(event) = event.dyn_ref::<web_sys::KeyboardEvent>() {
            if (event.meta_key() || event.ctrl_key()) && event.key() == "k" {
                event.prevent_default();
                self.menu.set(true);
                if let Ok(input) = document().and_then(|doc| {
                    doc.get_element_by_id("search")
                        .ok_or_else(|| "search".into())
                }) {
                    if let Some(input) = input.dyn_ref::<web_sys::HtmlElement>() {
                        let _ = input.focus();
                    }
                }
            }
            if event.key() == "Escape" {
                self.query.set(String::new());
                self.menu.set(false);
            }
        }
    }
    fn matches(&self, index: usize) -> bool {
        matches(index, &self.query.get())
    }
    fn groups(&self) -> Vec<&'static str> {
        let mut groups = Vec::new();
        for page in PAGES {
            if !groups.contains(&page.group) {
                groups.push(page.group);
            }
        }
        groups
    }
    fn is_reference(&self) -> bool {
        PAGES
            .get(self.active.get())
            .is_some_and(|page| page.reference || page.group == "Reference")
    }
    fn matches_showcase(&self) -> bool {
        let needle = self.query.get().trim().to_lowercase();
        "showcase examples demos".contains(&needle)
            || DEMOS.iter().any(|demo| {
                [demo.title, demo.category, demo.description]
                    .iter()
                    .any(|text| text.to_lowercase().contains(&needle))
            })
    }
    fn open(&self) {
        self.menu.set(false);
        self.query.set(String::new());
    }
}
struct DocRoute {
    page: Option<Page>,
}
pub struct DocRouteInputs {
    pub active: Signal<usize>,
}
impl fusor::dom::FromInputs for DocRoute {
    type Inputs = DocRouteInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        inputs: Self::Inputs,
        owner: OwnerHandle,
    ) -> Result<Self, wasm_bindgen::JsValue> {
        let navigation = Navigation::from_owner(&owner)
            .ok_or_else(|| wasm_bindgen::JsValue::from_str("missing docs navigation"))?;
        let page = Page::parse(&navigation.location().get());
        inputs.active.set(match page {
            Some(Page::Guide(index)) => index,
            Some(_) => PAGES.len(),
            None => usize::MAX,
        });
        Ok(Self { page })
    }
}
impl DocRoute {
    fn guide(&self) -> Option<usize> {
        if let Some(Page::Guide(index)) = self.page {
            Some(index)
        } else {
            None
        }
    }
    fn demo(&self) -> usize {
        if let Some(Page::Demo(index)) = self.page {
            index
        } else {
            0
        }
    }
}

struct NavGroup {
    item: fusor::Memo<&'static str>,
    query: Signal<String>,
    menu: Signal<bool>,
    active: Signal<usize>,
}
impl NavGroup {
    fn entries(&self) -> Vec<usize> {
        children(None, self.item.get())
    }
    fn visible(&self) -> bool {
        PAGES
            .iter()
            .enumerate()
            .any(|(i, page)| page.group == self.item.get() && matches(i, &self.query.get()))
    }
}
struct NavEntry {
    item: fusor::Memo<usize>,
    query: Signal<String>,
    menu: Signal<bool>,
    active: Signal<usize>,
    // A manual choice applies until navigation changes the active page.
    expanded: Signal<Option<(usize, bool)>>,
}
impl NavEntry {
    fn new(
        item: fusor::Memo<usize>,
        query: Signal<String>,
        menu: Signal<bool>,
        active: Signal<usize>,
    ) -> Self {
        Self {
            item,
            query,
            menu,
            active,
            expanded: signal(None),
        }
    }
    fn title(&self) -> &'static str {
        if self.item.get() == 0 {
            "Introduction"
        } else {
            PAGES[self.item.get()].title
        }
    }
    fn children(&self) -> Vec<usize> {
        let page = &PAGES[self.item.get()];
        children(Some(page.slug), page.group)
    }
    fn visible(&self) -> bool {
        matches(self.item.get(), &self.query.get())
            || PAGES.iter().enumerate().any(|(i, _)| {
                ancestors(i).contains(&self.item.get()) && matches(i, &self.query.get())
            })
    }
    fn is_open(&self) -> bool {
        if !self.query.get().trim().is_empty() {
            return true;
        }
        let active = self.active.get();
        if let Some((page, open)) = self.expanded.get() {
            if page == active {
                return open;
            }
        }
        active == self.item.get() || ancestors(active).contains(&self.item.get())
    }
    fn toggle(&self) {
        self.expanded
            .set(Some((self.active.get(), !self.is_open())));
    }
    fn open(&self) {
        self.menu.set(false);
        self.query.set(String::new());
    }
}
fusor::bindings!(app);

impl fusor::dom::FromInputs for NavGroup {
    type Error = fusor::dom::JsValue;
    type Inputs = Self;
    fn from_inputs(inputs: Self, _owner: fusor::OwnerHandle) -> Result<Self, fusor::dom::JsValue> {
        Ok(inputs)
    }
}

struct NavEntryInputs {
    pub item: fusor::Memo<usize>,
    pub query: Signal<String>,
    pub menu: Signal<bool>,
    pub active: Signal<usize>,
}
impl fusor::dom::FromInputs for NavEntry {
    type Inputs = NavEntryInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        inputs: Self::Inputs,
        _owner: fusor::OwnerHandle,
    ) -> Result<Self, fusor::dom::JsValue> {
        Ok(Self::new(
            inputs.item,
            inputs.query,
            inputs.menu,
            inputs.active,
        ))
    }
}
