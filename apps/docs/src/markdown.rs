use fusor::dom::{Component, Content, JsValue, Scope, TemplateComponent, element};

pub(crate) fn markdown(html: &'static str) -> Content {
    Content::new(move |_| Markdown(html))
}

struct Markdown(&'static str);

impl Component for Markdown {
    fn mount(self) -> Result<Scope, JsValue> {
        let root = element("div")?;
        root.set_class_name("markdown");
        // Only the build-time Markdown renderer supplies HTML to this component.
        root.set_inner_html(self.0);
        Ok(Scope::new(root))
    }
}

impl TemplateComponent for Markdown {}
