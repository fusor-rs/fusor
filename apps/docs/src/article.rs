use crate::content::PAGES;
use docs_base::ArticleExtras;
use fusor::{dom::Content, prelude::*};

pub(crate) fn extras(index: Option<usize>) -> ArticleExtras {
    ArticleExtras {
        before: (index == Some(0)).then(|| Content::new(|_| Welcome)),
        after: index
            .filter(|index| *index == 0 || PAGES[*index].slug == "reactivity")
            .map(|_| Content::new(|_| Counter { count: signal(0) })),
        footer: Some(Content::new(|_| ContributorLink)),
        aside: Some(Content::new(|_| Motto)),
    }
}

struct Welcome;
struct Counter {
    count: Signal<i32>,
}
struct ContributorLink;
struct Motto;

fusor::bindings!(article);
