use crate::{
    article::extras,
    content::{DEMOS, SITE},
    routes::Page,
    showcase::{DemoPage, Gallery},
};
use docs_base::{Article, ArticleExtras, Location, Navigation, Shell};
use fusor::{dom::Content, prelude::*};
use fusor_router::{Route, browser::declarative::Navigation as RouterNavigation};

pub struct App {
    navigation: Navigation,
    header: Content,
    sidebar: Content,
}

impl App {
    fn new() -> Self {
        let navigation = Navigation::new(&SITE);
        let header_navigation = navigation.clone();
        let sidebar_navigation = navigation.clone();
        Self {
            navigation,
            header: Content::new(move |_| TopLinks {
                navigation: header_navigation.clone(),
            }),
            sidebar: Content::new(move |_| Explore {
                navigation: sidebar_navigation.clone(),
            }),
        }
    }
}

struct TopLinks {
    navigation: Navigation,
}

struct Explore {
    navigation: Navigation,
}

impl Explore {
    fn matches(&self) -> bool {
        let needle = self.navigation.query.get().trim().to_lowercase();
        "showcase examples demos".contains(&needle)
            || DEMOS.iter().any(|demo| {
                [demo.title, demo.category, demo.description]
                    .iter()
                    .any(|text| text.to_lowercase().contains(&needle))
            })
    }
}

struct DocRoute {
    page: Option<Page>,
    extras: ArticleExtras,
}

pub struct DocRouteInputs {
    pub navigation: Navigation,
}

impl fusor::dom::FromInputs for DocRoute {
    type Inputs = DocRouteInputs;
    type Error = fusor::dom::JsValue;

    fn from_inputs(inputs: Self::Inputs, owner: OwnerHandle) -> Result<Self, Self::Error> {
        let router = RouterNavigation::from_owner(&owner)
            .ok_or_else(|| wasm_bindgen::JsValue::from_str("missing docs navigation"))?;
        let page = Page::parse(&router.location().get());
        let location = match page {
            Some(Page::Guide(index)) => Location::Guide(index),
            Some(_) => Location::Custom,
            None => Location::Missing,
        };
        inputs.navigation.active.set(location);
        let index = match location {
            Location::Guide(index) => Some(index),
            _ => None,
        };
        Ok(Self {
            page,
            extras: extras(index),
        })
    }
}

impl DocRoute {
    fn guide(&self) -> Option<usize> {
        match self.page {
            Some(Page::Guide(index)) => Some(index),
            _ => None,
        }
    }

    fn demo(&self) -> usize {
        match self.page {
            Some(Page::Demo(index)) => index,
            _ => 0,
        }
    }
}

fusor::bindings!(app);
