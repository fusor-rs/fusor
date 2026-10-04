use crate::content::{DEMOS, PAGES};
use fusor_router::{AppUrl, Route};
#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    Guide(usize),
    Showcase,
    Demo(usize),
}
impl Route for Page {
    fn parse(url: &AppUrl) -> Option<Self> {
        let segments = url.segments().ok()?;
        match segments.as_slice() {
            [slug] if slug == "showcase" => Some(Self::Showcase),
            [root, slug] if root == "showcase" => DEMOS
                .iter()
                .position(|demo| demo.slug == slug)
                .map(Self::Demo),
            _ => PAGES
                .iter()
                .position(|page| page.slug == segments.join("/"))
                .map(Self::Guide),
        }
    }
    fn path(&self) -> String {
        match self {
            Self::Guide(index) => format!("/{}", PAGES[*index].slug),
            Self::Showcase => "/showcase".into(),
            Self::Demo(index) => format!("/showcase/{}", DEMOS[*index].slug),
        }
    }
}
