use fusor_router::{AppUrl, BasePath, Route};
pub const BASE: &str = env!("FUSOR_BASE_PATH");
#[derive(Clone, PartialEq, Debug)]
pub enum Page {
    Home,
    Article(u32),
}
impl Route for Page {
    fn parse(url: &AppUrl) -> Option<Self> {
        match url.segments().ok()?.as_slice() {
            [root] if root.is_empty() => Some(Self::Home),
            [articles, id] if articles == "articles" => id.parse().ok().map(Self::Article),
            _ => None,
        }
    }
    fn path(&self) -> String {
        match self {
            Self::Home => "/".into(),
            Self::Article(id) => format!("/articles/{id}"),
        }
    }
}
pub fn href(page: Page) -> String {
    BasePath::new(BASE)
        .expect("the build validates FUSOR_BASE_PATH")
        .href(&page)
        .expect("Page paths contain only static segments and numeric identifiers")
}
