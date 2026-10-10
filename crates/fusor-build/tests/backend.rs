//! Metadata and capability validation through the supported external facade.
//! Generated-code execution is covered by the out-of-workspace backend consumer.
use fusor_build::{
    ExtractError,
    backend::{
        self, Anchor, Backend, Capability, ComponentCode, NodeKind, Operation, Origin, Runtime,
        Template,
    },
};
use proc_macro2::TokenStream;
use quote::quote;
use std::cell::RefCell;

struct RecordingBackend {
    version: u32,
    text: bool,
    app: bool,
    router: bool,
    coherent: bool,
    templates: RefCell<Vec<Template>>,
    operations: RefCell<Vec<Operation>>,
}

impl Default for RecordingBackend {
    fn default() -> Self {
        Self {
            version: backend::VERSION,
            text: true,
            app: true,
            router: true,
            coherent: false,
            templates: RefCell::new(Vec::new()),
            operations: RefCell::new(Vec::new()),
        }
    }
}

impl Backend for RecordingBackend {
    fn version(&self) -> u32 {
        self.version
    }

    fn name(&self) -> &str {
        "metadata-test"
    }

    fn runtime(&self) -> Runtime {
        Runtime {
            scope: syn::parse_quote!(::test_runtime::Scope),
            error: syn::parse_quote!(::test_runtime::Error),
            children: syn::parse_quote!(::test_runtime::Children),
            convert_error: syn::parse_quote!(::test_runtime::convert_error),
            coherent_frame: self
                .coherent
                .then(|| syn::parse_quote!(::test_runtime::Frame)),
        }
    }

    fn supports(&self, capability: Capability<'_>) -> bool {
        match capability {
            Capability::Text => self.text,
            Capability::App => self.app,
            Capability::Router => self.router,
            Capability::Async => self.coherent,
            _ => true,
        }
    }

    fn validate_binding(
        &self,
        template: &Template,
        capability: Capability<'_>,
        anchor: Anchor,
        origin: &Origin,
    ) -> Result<(), ExtractError> {
        if let Capability::Event(event) = capability {
            let button = template.nodes.iter().any(|node| {
                matches!(&node.kind, NodeKind::Element { tag, anchor: Some(id), .. }
                    if tag == "button" && anchor == Anchor::Element(*id))
            });
            if event != "click" || !button {
                return Err(origin.error("only direct button clicks are supported"));
            }
        }
        Ok(())
    }

    fn validate(&self, template: &Template) -> Result<(), ExtractError> {
        for node in &template.nodes {
            if let NodeKind::Element {
                tag, attributes, ..
            } = &node.kind
            {
                if tag == "canvas" {
                    return Err(node.origin.error("canvas is unsupported"));
                }
                if let Some(attribute) = attributes.iter().find(|attribute| attribute.name == "bad")
                {
                    return Err(attribute.origin.error("attribute is unsupported"));
                }
            }
        }
        self.templates.borrow_mut().push(template.clone());
        Ok(())
    }

    fn mount(&self, _: &Template) -> TokenStream {
        quote! { ::test_runtime::mount(parent) }
    }

    fn operation(&self, operation: Operation) -> TokenStream {
        self.operations.borrow_mut().push(operation);
        TokenStream::new()
    }

    fn component(&self, _: ComponentCode) -> TokenStream {
        TokenStream::new()
    }
}

fn fail(source: &str, backend: &RecordingBackend) -> ExtractError {
    match backend::generate(source, backend) {
        Ok(_) => panic!("expected a backend diagnostic"),
        Err(error) => error,
    }
}

fn location(source: &str, needle: &str) -> (usize, usize) {
    let before = &source[..source.find(needle).expect("source contains needle")];
    (
        before.chars().filter(|ch| *ch == '\n').count() + 1,
        before.rsplit('\n').next().unwrap().chars().count() + 1,
    )
}

fn static_element(nodes: &[backend::Node], tag: &str, parent: Option<usize>) -> usize {
    nodes
        .iter()
        .position(|node| {
            node.parent == parent
                && matches!(&node.kind,
                    NodeKind::Element { tag: name, anchor: None, .. } if name == tag
                )
        })
        .expect("the static element is retained")
}

fn assert_retained_anchors(nodes: &[backend::Node], operations: &[Operation]) {
    for operation in operations {
        let present = nodes
            .iter()
            .any(|node| match (&node.kind, operation.anchor) {
                (
                    NodeKind::Element {
                        anchor: Some(id), ..
                    },
                    Anchor::Element(anchor),
                )
                | (
                    NodeKind::Text {
                        anchor: Some(id), ..
                    },
                    Anchor::Text(anchor),
                ) => *id == anchor,
                _ => false,
            });
        assert!(present, "binding anchor exists in the complete static tree");
    }
}

#[test]
fn full_static_tree_preserves_nesting_entities_comments_and_binding_anchors() {
    let source = r#"<template rust:component="Panel">
  <section title="A &amp; B">
    <p>plain &amp; static <strong>nested</strong> tail</p>
    <!-- retained -->
    <button on:click="state.increment()">Add</button>
    <p>{{ state.label.get() }}</p>
  </section>
</template>"#;
    let backend = RecordingBackend::default();
    backend::generate(source, &backend).unwrap();
    let templates = backend.templates.borrow();
    assert_eq!(templates.len(), 1);
    let nodes = &templates[0].nodes;
    assert!(
        nodes.iter().all(|node| {
            !matches!(&node.kind, NodeKind::Element { tag, .. } if tag == "template")
        })
    );
    for (index, node) in nodes.iter().enumerate() {
        if let Some(parent) = node.parent {
            assert!(parent < index, "parents precede their children");
        }
    }
    let section = static_element(nodes, "section", None);
    assert!(
        matches!(&nodes[section].kind, NodeKind::Element { attributes, .. }
        if attributes.iter().any(|attribute| attribute.name == "title" && attribute.value == "A & B"))
    );
    let paragraph = static_element(nodes, "p", Some(section));
    let strong = static_element(nodes, "strong", Some(paragraph));
    let static_text: Vec<_> = nodes
        .iter()
        .filter_map(|node| {
            if let NodeKind::Text {
                value,
                anchor: None,
            } = &node.kind
            {
                (node.parent == Some(paragraph)).then_some(value.as_str())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(static_text.concat(), "plain & static  tail");
    assert!(nodes.iter().any(|node| node.parent == Some(strong)
        && matches!(&node.kind, NodeKind::Text { value, .. } if value == "nested")));
    assert!(
        nodes
            .iter()
            .any(|node| matches!(&node.kind, NodeKind::Comment(value) if value == " retained "))
    );
    assert_retained_anchors(nodes, &backend.operations.borrow());
    assert_eq!(backend.operations.borrow().len(), 2);
}

#[test]
fn nested_structural_regions_reach_the_backend_without_flattening_their_templates() {
    let source = r#"<template rust:component="Panel"><section>
<If condition="{{ state.visible.get() }}"><p>conditional {{ state.title }}</p><Else><span>hidden</span></Else></If>
<ul><ForEach items="{{ state.items.get() }}" key="{{ |item| item.id }}"><li>{{ item.get().title }}</li></ForEach></ul>
</section></template>"#;
    let backend = RecordingBackend::default();
    backend::generate(source, &backend).unwrap();
    let templates = backend.templates.borrow();
    assert!(templates.len() >= 4);
    assert!(
        templates[0]
            .nodes
            .iter()
            .any(|node| matches!(node.kind, NodeKind::Mount { .. }))
    );
    assert!(templates.iter().skip(1).any(|template| {
        template
            .nodes
            .iter()
            .any(|node| matches!(&node.kind, NodeKind::Element { tag, .. } if tag == "li"))
    }));
    let operations = backend.operations.borrow();
    assert!(
        operations
            .iter()
            .any(|operation| matches!(operation.kind, backend::OperationKind::Branch { .. }))
    );
    assert!(
        operations
            .iter()
            .any(|operation| matches!(operation.kind, backend::OperationKind::Keyed { .. }))
    );
}

#[test]
fn rewritten_static_attributes_retain_their_authored_location() {
    let source = "<template rust:component=\"Panel\">\n  <button\n    on:click=\"state.go()\"\n    title=\"é &amp; text\"\n    bad=\"unsupported\">Go</button>\n</template>";
    let error = fail(source, &RecordingBackend::default());
    assert_eq!(error.message, "attribute is unsupported");
    assert_eq!((error.line, error.column), location(source, "bad="));
}

#[test]
fn unbound_unsupported_elements_are_validated_at_the_authored_tag() {
    let source = "<template rust:component=\"Panel\">\n  <div><canvas></canvas></div>\n</template>";
    let error = fail(source, &RecordingBackend::default());
    assert_eq!(error.message, "canvas is unsupported");
    assert_eq!((error.line, error.column), location(source, "<canvas>"));
}

#[test]
fn target_sensitive_event_checks_distinguish_native_controls() {
    let source =
        "<template rust:component=\"Panel\">\n  <div on:click=\"state.go()\">Go</div>\n</template>";
    let error = fail(source, &RecordingBackend::default());
    assert_eq!(error.message, "only direct button clicks are supported");
    assert_eq!((error.line, error.column), location(source, "state.go()"));
    backend::generate(
        &source.replace("div", "button"),
        &RecordingBackend::default(),
    )
    .unwrap();
}

#[test]
fn rejected_features_and_versions_fail_before_backend_emission() {
    let source = "<template rust:component=\"Panel\">\n  <p>{{ state.title }}</p>\n</template>";
    let backend = RecordingBackend {
        text: false,
        ..RecordingBackend::default()
    };
    let error = fail(source, &backend);
    assert!(error.message.contains("does not support Text"));
    assert_eq!(
        (error.line, error.column),
        location(source, "{{ state.title")
    );
    assert!(backend.operations.borrow().is_empty());
    let backend = RecordingBackend {
        version: backend::VERSION + 1,
        ..RecordingBackend::default()
    };
    let error = fail(source, &backend);
    assert!(error.message.contains("requires compiler contract"));
    assert!(backend.templates.borrow().is_empty());
}

#[test]
fn browser_delivery_and_unimplemented_coherent_features_are_explicitly_rejected() {
    for (source, expected) in [
        (
            r#"<template rust:component="Panel"><script type="text/rust">fn unused() {}</script></template>"#,
            "script integration is unsupported",
        ),
        (
            r#"<template rust:component="Panel" rust:render="shared"><p>shared</p></template>"#,
            "browser/server delivery contract",
        ),
        (
            r#"<template rust:component="Panel"><Async><section><p>pending</p></section></Async></template>"#,
            "coherent Async/Await",
        ),
        (
            r#"<template rust:component="Panel"><button type="button" hydrate:target="island">Open</button></template>"#,
            "hydration",
        ),
    ] {
        let backend = RecordingBackend::default();
        let error = fail(source, &backend);
        assert!(error.message.contains(expected), "{error}");
        assert!(backend.operations.borrow().is_empty());
    }
}

#[test]
fn async_supplies_coherent_operations_and_rejects_editable_regions() {
    let source = r#"<template rust:component="Panel"><Async boundary="{{ state.boundary }}"><section>
      <Await value="{{ state.value }}" let="result"><p>{{ result.as_str() }}</p></Await>
      <ul><ForEach items="{{ state.rows.get() }}" key="{{ |row| row.id }}"><li>{{ item.get().label }}</li></ForEach></ul>
    </section></Async></template>"#;
    let backend = RecordingBackend {
        coherent: true,
        ..RecordingBackend::default()
    };
    backend::generate(source, &backend).unwrap();
    let operations = backend.operations.borrow();
    assert!(
        operations
            .iter()
            .any(|operation| matches!(operation.kind, backend::OperationKind::Async { .. }))
    );
    assert!(operations.iter().any(
        |operation| operation.mode == backend::OperationMode::Coherent
            && matches!(operation.kind, backend::OperationKind::Keyed { .. })
    ));
    drop(operations);
    let invalid = source.replace("<section>", "<section><input bind=\"state.input\">");
    let error = fail(&invalid, &backend);
    assert!(error.message.contains("support display bindings only"));
    assert_eq!((error.line, error.column), location(&invalid, "<input"));
}

#[test]
fn nested_children_emit_one_factory_for_both_binding_modes() {
    let source = format!(
        "<template rust:component=\"Panel\"><div>{}<p>{{{{ state.leaf }}}}</p>{}</div></template>",
        "<Frame>".repeat(8),
        "</Frame>".repeat(8)
    );
    let backend = RecordingBackend {
        coherent: true,
        ..RecordingBackend::default()
    };
    backend::generate(&source, &backend).unwrap();
    assert_eq!(
        backend
            .operations
            .borrow()
            .iter()
            .filter(|operation| matches!(operation.kind, backend::OperationKind::Text { .. }))
            .count(),
        2,
        "one reactive and one coherent text operation, regardless of wrapper depth"
    );
}

#[test]
fn routing_supplies_factories_and_requires_backend_support() {
    let source = r#"<template rust:component="Panel"><div><Router>
        <Route path="/jobs/:id" let="route"><p>{{ route.id }}</p></Route>
        <Route fallback><p>missing</p></Route>
    </Router></div></template>"#;
    let backend = RecordingBackend::default();
    backend::generate(source, &backend).unwrap();
    let operations = backend.operations.borrow();
    let routes = operations
        .iter()
        .find_map(|operation| match &operation.kind {
            backend::OperationKind::Router { routes } => Some(routes),
            _ => None,
        })
        .unwrap();
    assert_eq!(routes.len(), 2);
    assert_eq!(routes[0].pattern.as_deref(), Some("/jobs/:id"));
    assert_eq!(routes[1].pattern, None);
    let unsupported = RecordingBackend {
        router: false,
        ..RecordingBackend::default()
    };
    let error = fail(source, &unsupported);
    assert!(error.message.contains("does not support Router"));
    assert!(unsupported.operations.borrow().is_empty());
}

#[test]
fn static_markup_styles_and_text_outside_components_cannot_disappear() {
    let component = "<template rust:component=\"Panel\"><p>inside</p></template>";
    for outside in [
        "<style>p { color: red; }</style>",
        "<div>outside</div>",
        "outside text",
        "<!doctype html>",
        "<link rel=\"stylesheet\" href=\"style.css\">",
    ] {
        let source = format!("{component}\n{outside}");
        let backend = RecordingBackend::default();
        let error = fail(&source, &backend);
        assert!(error.message.contains("requires markup inside"), "{error}");
        assert_eq!((error.line, error.column), (2, 1));
        assert!(backend.templates.borrow().is_empty());
        assert!(backend.operations.borrow().is_empty());
    }
}

#[test]
fn sources_without_a_component_or_app_are_rejected() {
    for source in ["", "<!-- only a comment -->", "<p>unowned</p>"] {
        let error = fail(source, &RecordingBackend::default());
        assert!(
            error
                .message
                .contains("require a rust:component declaration or App")
        );
    }
}

#[test]
fn app_entry_requires_an_explicit_backend_capability() {
    let source = "<App state=\"{{ State::new() }}\"><section>entry</section></App>";
    let backend = RecordingBackend {
        app: false,
        ..RecordingBackend::default()
    };
    let error = fail(source, &backend);
    assert!(error.message.contains("does not support App"));
    assert_eq!((error.line, error.column), (1, 1));
    assert!(backend.templates.borrow().is_empty());

    let backend = RecordingBackend::default();
    backend::generate(source, &backend).unwrap();
    assert!(backend.templates.borrow().iter().any(|template| {
        template
            .nodes
            .iter()
            .any(|node| matches!(&node.kind, NodeKind::Element { tag, .. } if tag == "section"))
    }));
}
