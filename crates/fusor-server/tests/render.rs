use fusor_islands::{Activation, DeliveryManifest, Entry, Island, Prefetch, RenderMode, Unit};
use fusor_server::{Context, Html, Registry, Render, Writer};
use serde::{Deserialize, Serialize};
#[derive(Serialize, Deserialize)]
struct Props {
    id: u64,
    text: String,
}
struct Descriptor;
impl Island for Descriptor {
    type Props = Props;
    const NAME: &'static str = "test.card";
    const UNIT: &'static str = "cards";
    const SCHEMA: &'static str = "v1";
}
struct Card(Props);
impl Render for Card {
    const TEMPLATE_HASH: &'static str = "hash-v1";
    fn render(&self, _: &mut Context<'_>) -> fusor_server::Result<Html> {
        let mut writer = Writer::new();
        writer.open("article");
        writer.attr("title", &self.0.text);
        writer.end_open();
        writer.text(&self.0.text);
        writer.close("article");
        Ok(writer.finish())
    }
}
fn manifest() -> DeliveryManifest {
    DeliveryManifest {
        version: 1,
        generation: "test-v1".into(),
        units: [(
            "cards".into(),
            Unit {
                javascript: "/test-v1/cards.js".into(),
                wasm: "/test-v1/cards.wasm".into(),
                dependencies: vec![],
                entries: vec![Entry {
                    unit: "cards".into(),
                    descriptor: "test.card".into(),
                    props_schema: "v1".into(),
                    template_hash: "hash-v1".into(),
                    mode: RenderMode::Attach,
                }],
            },
        )]
        .into(),
    }
}
#[test]
fn escaping_opaque_props_and_duplicate_instances() {
    let mut registry = Registry::new();
    registry.register::<Descriptor, Card>(Card).unwrap();
    let manifest = manifest();
    let mut context = Context::with_islands(&manifest, &registry).unwrap();
    let props = Props {
        id: 9_007_199_254_740_993,
        text: "</script><img src=x onerror=alert(1)>\" & 日本語\u{2028}".into(),
    };
    let html = context
        .island::<Descriptor>(Some("card"), &props, Activation::Manual, Prefetch::None)
        .unwrap()
        .into_string();
    assert!(!html.contains("<img"));
    assert_eq!(html.matches("</script>").count(), 1);
    assert!(html.contains("&lt;/script&gt;"));
    assert!(html.contains("&quot;"));
    assert!(html.contains("\\u003c/script\\u003e"));
    assert!(html.contains("\\u2028"));
    assert!(html.contains("9007199254740993"));
    let roundtrip: Props = fusor_islands::decode(&fusor_islands::encode(&props).unwrap()).unwrap();
    assert_eq!(roundtrip.id, props.id);
    assert_eq!(roundtrip.text, props.text);
    assert!(
        context
            .island::<Descriptor>(Some("card"), &props, Activation::Manual, Prefetch::None)
            .is_err()
    );
}
#[test]
fn registration_pairs_and_protocol_versions_fail_closed() {
    let mut registry = Registry::new();
    registry.register::<Descriptor, Card>(Card).unwrap();
    for mutate in [0, 1, 2, 3] {
        let mut manifest = manifest();
        let entry = &mut manifest.units.get_mut("cards").unwrap().entries[0];
        match mutate {
            0 => entry.template_hash = "wrong".into(),
            1 => entry.props_schema = "wrong".into(),
            2 => entry.unit = "wrong".into(),
            _ => manifest.version = 2,
        };
        assert!(Context::with_islands(&manifest, &registry).is_err());
    }
    let mut manifest = manifest();
    manifest.units.get_mut("cards").unwrap().javascript = "//outside.example/module.js".into();
    assert!(manifest.validate().is_err());
}
#[test]
fn row_keys_are_inserted_at_the_structural_root_and_escaped() {
    let mut writer = Writer::new();
    writer.open("li");
    writer.attr("class", "row");
    writer.end_open();
    writer.text("value");
    writer.close("li");
    let html = writer.finish().with_key(&"\"<&").unwrap();
    assert!(html.as_str().starts_with("<li data-fusor-key=\""));
    assert!(html.as_str().ends_with(" class=\"row\">value</li>"));
    assert!(html.as_str().contains("&lt;&amp;"));
}

#[test]
fn children_render_with_receiving_context_and_restore_the_caller_on_error() {
    struct Locale;
    impl fusor::ContextKey for Locale {
        type Value = &'static str;
    }
    struct Panel;
    impl Render for Panel {
        const TEMPLATE_HASH: &'static str = "panel";
        fn render(&self, _: &mut Context<'_>) -> fusor_server::Result<Html> {
            unreachable!()
        }
        fn render_with_children(
            &self,
            context: &mut Context<'_>,
            children: Option<&fusor_server::Children<'_>>,
        ) -> fusor_server::Result<Html> {
            assert_eq!(*context.owner().context::<Locale>().unwrap(), "child");
            children.unwrap()(context, Some("footer"))
        }
    }
    let mut context = Context::new();
    context.owner().provide::<Locale>("parent").unwrap();
    let result = context.try_child_with_children(
        |owner| {
            owner.provide::<Locale>("child").unwrap();
            Ok(Panel)
        },
        Some(&|context, slot| {
            assert_eq!(slot, Some("footer"));
            assert_eq!(*context.owner().context::<Locale>().unwrap(), "child");
            Err("failed child render".into())
        }),
    );
    assert!(
        matches!(result.unwrap_err(), fusor_server::Error::Render(message) if message == "failed child render")
    );
    assert_eq!(*context.owner().context::<Locale>().unwrap(), "parent");
}

#[test]
fn streamed_custom_renderers_keep_children_owner_and_key_metadata() {
    struct Locale;
    impl fusor::ContextKey for Locale {
        type Value = &'static str;
    }
    struct Panel;
    impl Render for Panel {
        const TEMPLATE_HASH: &'static str = "panel";
        fn render(&self, _: &mut Context<'_>) -> fusor_server::Result<Html> {
            unreachable!()
        }
        fn render_with_children(
            &self,
            context: &mut Context<'_>,
            children: Option<&fusor_server::Children<'_>>,
        ) -> fusor_server::Result<Html> {
            assert_eq!(*context.owner().context::<Locale>().unwrap(), "child");
            children.unwrap()(context, None)
        }
    }
    let mut context = Context::new();
    context.owner().provide::<Locale>("parent").unwrap();
    let mut writer = Writer::new();
    writer.open("div");
    writer.end_open();
    for fail in [true, false] {
        let result = writer.keyed_child(&7, |writer| {
            context.try_child_into_with_children(
                |owner| {
                    owner.provide::<Locale>("child").unwrap();
                    Ok(Panel)
                },
                Some(&|context, slot| {
                    assert_eq!(slot, None);
                    assert_eq!(*context.owner().context::<Locale>().unwrap(), "child");
                    if fail {
                        return Err("failure".into());
                    }
                    let mut content = Writer::new();
                    content.open("b");
                    content.end_open();
                    content.text("<&");
                    content.close("b");
                    Ok(content.finish())
                }),
                writer,
            )
        });
        assert_eq!(result.is_err(), fail);
        assert_eq!(*context.owner().context::<Locale>().unwrap(), "parent");
    }
    writer.close("div");
    assert_eq!(
        writer.finish().as_str(),
        "<div><b data-fusor-key=\"7\">&lt;&amp;</b></div>"
    );
}

#[derive(Clone, Copy)]
enum RenderFailure {
    Error,
    Panic,
    None,
}
struct RecoveringCard(RenderFailure);
impl Render for RecoveringCard {
    const TEMPLATE_HASH: &'static str = "hash-v1";
    fn render(&self, _: &mut Context<'_>) -> fusor_server::Result<Html> {
        match self.0 {
            RenderFailure::Error => Err("card rendering failed".into()),
            RenderFailure::Panic => panic!("card rendering panicked"),
            RenderFailure::None => Ok(Writer::new().finish()),
        }
    }
}

#[test]
fn failed_islands_release_instance_ids_and_restore_render_depth() {
    use std::{
        cell::Cell,
        panic::{AssertUnwindSafe, catch_unwind},
        rc::Rc,
    };
    for failure in [RenderFailure::Error, RenderFailure::Panic] {
        let state = Rc::new(Cell::new(failure));
        let selected = state.clone();
        let mut registry = Registry::new();
        registry
            .register::<Descriptor, RecoveringCard>(move |_| RecoveringCard(selected.get()))
            .unwrap();
        let manifest = manifest();
        let mut context = Context::with_islands(&manifest, &registry).unwrap();
        let props = Props {
            id: 7,
            text: "card".into(),
        };
        let result = catch_unwind(AssertUnwindSafe(|| {
            context.island::<Descriptor>(Some("card"), &props, Activation::Manual, Prefetch::None)
        }));
        match failure {
            RenderFailure::Error => assert!(
                matches!(result.unwrap().unwrap_err(), fusor_server::Error::Render(message) if message == "card rendering failed")
            ),
            RenderFailure::Panic => assert_eq!(
                result.unwrap_err().downcast_ref::<&str>(),
                Some(&"card rendering panicked")
            ),
            RenderFailure::None => unreachable!(),
        }
        state.set(RenderFailure::None);
        let html = context
            .island::<Descriptor>(Some("card"), &props, Activation::Manual, Prefetch::None)
            .unwrap();
        assert!(html.as_str().contains(" id=\"card\""));
        let html = context
            .island::<Descriptor>(None, &props, Activation::Manual, Prefetch::None)
            .unwrap();
        assert!(html.as_str().contains(" id=\"fusor-island-2\""));
    }
}
