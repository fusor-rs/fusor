use crate::{
    code::{CodeBlock, CodeData},
    content::{DEMOS, DemoData},
    demos::{
        chartjs::ChartJs, coherent::Coherent, comparison::Comparison, context::Context,
        fractal::Fractal, game::Game, keyed::Keyed, lifecycle::Lifecycle, loading::Loading,
        reactive::Reactive, search::Search, threejs::ThreeJs,
    },
};
use fusor::{Signal, dom::Content, signal};

pub struct Gallery;

impl Gallery {
    pub fn new() -> Self {
        set_title("Showcase");
        Self
    }
}

struct DemoCard {
    item: fusor::Memo<DemoData>,
}

pub struct DemoPage {
    demo: &'static DemoData,
    content: Content,
    generation: Signal<u32>,
    language: Signal<u32>,
}

impl DemoPage {
    pub fn new(index: usize) -> Self {
        let demo = &DEMOS[index];
        set_title(demo.title);
        let content = match demo.slug {
            "reactive" => Content::new(|_| Reactive::new()),
            "keyed" => Content::new(|_| Keyed::new()),
            "lifecycle" => Content::new(|_| Lifecycle::new()),
            "loading" => Content::new(Loading::new),
            "coherent" => Content::new(|_| Coherent::new()),
            "comparison" => Content::new(Comparison::new),
            "context" => Content::try_new(Context::new),
            "chartjs" => Content::new(|_| ChartJs::new()),
            "threejs" => Content::new(|_| ThreeJs::new()),
            "search" => Content::new(Search::new),
            "game" => Content::new(Game::new),
            "fractal" => Content::new(Fractal::new),
            _ => unreachable!("showcase metadata must name a registered demo"),
        };
        Self {
            demo,
            content,
            generation: signal(0),
            language: signal(0),
        }
    }

    fn source(&self) -> CodeData {
        match self.language.get() {
            1 => self.demo.html,
            2 => self.demo.javascript.unwrap_or(self.demo.rust),
            _ => self.demo.rust,
        }
    }

    fn source_url(&self) -> String {
        let extension = match self.language.get() {
            1 => "html",
            2 => "js",
            _ => "rs",
        };
        format!("/docs/source/showcase/{}.{extension}.txt", self.demo.slug)
    }
}

fn set_title(title: &str) {
    if let Ok(document) = fusor::dom::document() {
        document.set_title(&format!("{title} · fusor"));
    }
}

fusor::bindings!(showcase);

impl fusor::dom::FromInputs for DemoCard {
    type Error = fusor::dom::JsValue;
    type Inputs = Self;
    fn from_inputs(inputs: Self, _owner: fusor::OwnerHandle) -> Result<Self, fusor::dom::JsValue> {
        Ok(inputs)
    }
}

pub struct GalleryInputs {}
impl fusor::dom::FromInputs for Gallery {
    type Inputs = GalleryInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(_: Self::Inputs, _: fusor::OwnerHandle) -> Result<Self, wasm_bindgen::JsValue> {
        Ok(Self::new())
    }
}
pub struct DemoPageInputs {
    pub index: usize,
}
impl fusor::dom::FromInputs for DemoPage {
    type Inputs = DemoPageInputs;
    type Error = fusor::dom::JsValue;
    fn from_inputs(
        inputs: Self::Inputs,
        _: fusor::OwnerHandle,
    ) -> Result<Self, wasm_bindgen::JsValue> {
        Ok(Self::new(inputs.index))
    }
}
