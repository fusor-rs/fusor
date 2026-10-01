use fusor_build::extract;

fn tokens(source: &str) -> String {
    source
        .parse::<proc_macro2::TokenStream>()
        .unwrap()
        .to_string()
}

const STATE: &str = "<script type=\"text/rust\">struct Counter;</script>";

#[test]
fn typed_text_keeps_original_closure_contract_for_returns_macros_and_attributes() {
    for expression in [
        "{ if state.done { return state.text.clone(); } state.value }",
        "opaque!(state.value)",
        "{ #[allow(unused)] let value = state.value; value }",
    ] {
        let page = extract(&format!(
            "{STATE}<p rust:component=Counter>{{{{ {expression} }}}}</p>"
        ))
        .unwrap();
        let rust = tokens(&page.rust);
        assert!(
            rust.contains("bundle_text_node_string"),
            "{expression}: {rust}"
        );
        assert!(
            !rust.contains("bundle_text_node_value"),
            "{expression}: {rust}"
        );
    }
    let page = extract(&format!(
        "{STATE}<p rust:component=Counter>{{{{ state.value.get() }}}}</p>"
    ))
    .unwrap();
    assert!(tokens(&page.rust).contains("bundle_text_node_value"));
}

#[test]
fn typed_attributes_keep_the_string_contract_unless_one_plain_interpolation() {
    for (value, typed) in [
        ("{{ state.value.get() }}", true),
        (" {{ state.value.get() }}", false),
        ("{{ state.value.get() }} ", false),
        ("row-{{ state.value.get() }}", false),
        ("{{ state.a }}{{ state.b }}", false),
        ("fixed", false),
        (
            "{{ { if state.done { return state.text.clone(); } state.value } }}",
            false,
        ),
        ("{{ opaque!(state.value) }}", false),
        (
            "{{ { #[allow(unused)] let value = state.value; value } }}",
            false,
        ),
    ] {
        for (host, bundled) in [("p", true), ("textarea", false)] {
            let page = extract(&format!(
                "{STATE}<template rust:component=Counter><section><{host} title=\"{value}\"></{host}></section></template>"
            ))
            .unwrap();
            let rust = tokens(&page.rust);
            let method = match (typed, bundled) {
                (true, true) => "bundle_attr_value",
                (true, false) => "__fusor_scope . attr_value",
                (false, true) => "bundle_attr (",
                (false, false) => "__fusor_scope . attr (",
            };
            if value == "fixed" {
                assert!(!rust.contains("attr_value"), "{value}: {rust}");
                continue;
            }
            assert!(rust.contains(method), "{host} {value}: {rust}");
            if !typed {
                assert!(!rust.contains("attr_value"), "{host} {value}: {rust}");
            }
        }
    }
}

#[test]
fn bind_chooses_its_runtime_from_the_markup_with_source_origins() {
    let page = extract(&format!(
        r#"{STATE}
<main rust:component="Counter">
  <input type="text" bind="state.title">
  <textarea bind="state.body"></textarea>
  <input type="range" min="1" max="10" bind="state.step">
  <input type="checkbox" bind="state.enabled">
  <input type="checkbox" value="{{{{ state.tag() }}}}" bind="state.tags">
  <input type="radio" name="size" value="small" bind="state.size">
  <select bind="state.size"><option value="small">Small</option><option>Large</option></select>
  <select multiple bind="state.sizes"><option>S</option><option value="{{{{ state.large() }}}}">L</option></select>
</main>"#
    ))
    .unwrap();
    let rust = tokens(&page.rust);
    for (function, count) in [
        ("text", 3),
        ("select", 1),
        ("select_multiple", 1),
        ("checkbox", 2),
        ("radio", 1),
        ("adopt_text", 3),
    ] {
        let call = format!(":: fusor :: dom :: controls :: {function} (");
        assert_eq!(rust.matches(&call).count(), count, "{function}");
    }
    // An unvalued checkbox submits "on"; a radio compares its own value.
    assert!(rust.contains(&tokens(r#"move || ::std::string::String::from("on")"#)));
    assert!(rust.contains(&tokens(r#"move || ::std::string::String::from("small")"#)));
    assert!(!page.html.contains("bind="));
    for line in 3..=10 {
        assert!(
            page.locations.iter().any(|location| location.line == line),
            "line {line}"
        );
    }
    for markup in [
        r#"<div bind="state.title"></div>"#,
        r#"<input bind="">"#,
        r#"<input type="{{ state.kind }}" bind="state.title">"#,
        r#"<input type="file" bind="state.title">"#,
        r#"<input type="hidden" bind="state.title">"#,
        r#"<input type="radio" bind="state.size">"#,
        r#"<input type="checkbox" checked bind="state.enabled">"#,
        r#"<input bind="state.title" value="default">"#,
        r#"<textarea bind="state.body">initial</textarea>"#,
        r#"<textarea bind="state.body" rust:slot="content"></textarea>"#,
        r#"<select multiple="{{ state.many() }}" bind="state.size"><option>Small</option></select>"#,
        r#"<select bind="state.size"><optgroup label="S"><option selected>Small</option></optgroup></select>"#,
        r#"<select bind="state.size"><option>{{ state.label() }}</option></select>"#,
    ] {
        let error = extract(&format!(
            "{STATE}\n<main rust:component=Counter>{markup}</main>"
        ))
        .expect_err(markup);
        assert_eq!(error.line, 2, "{markup}: {error}");
    }
}

#[test]
fn bind_writes_server_state_on_each_control() {
    let page = extract(&format!(
        r#"{STATE}
<main rust:component="Counter" rust:render="shared">
  <input bind="state.title"><input type="password" bind="state.secret">
  <textarea bind="state.body"></textarea>
  <input type="radio" value="small" bind="state.size">
  <select bind="state.size"><option value="small">Small</option><option> Extra
    large </option><option value="{{{{ state.large() }}}}">Large</option></select>
</main>"#
    ))
    .unwrap();
    let rust = tokens(&page.rust);
    for (statement, count) in [
        // Password text is never rendered.
        (
            r#"__fusor_writer.attr("value", ::fusor::bind::TextValue::text(&(state.title)));"#,
            1,
        ),
        (
            r#"__fusor_writer.attr("value", ::fusor::bind::TextValue::text(&(state.secret)));"#,
            0,
        ),
        (
            r#"__fusor_writer.text(::fusor::bind::TextValue::text(&(state.body)));"#,
            1,
        ),
        (
            r#"__fusor_writer.boolean("checked", ::fusor::bind::TextValue::shows(&(state.size), & ::std::string::String::from("small")));"#,
            1,
        ),
        (
            r#"__fusor_writer.boolean("selected", ::fusor::bind::TextValue::shows(&(state.size), & ::std::string::String::from("small")));"#,
            1,
        ),
        // Option text is compared as the browser reads it: trimmed and collapsed.
        (
            r#"__fusor_writer.boolean("selected", ::fusor::bind::TextValue::shows(&(state.size), & ::std::string::String::from("Extra large")));"#,
            1,
        ),
        (
            r#"__fusor_writer.boolean("selected", ::fusor::bind::TextValue::shows(&(state.size), & ::std::string::ToString::to_string(&(state.large()))));"#,
            1,
        ),
    ] {
        assert_eq!(
            rust.matches(&tokens(statement)).count(),
            count,
            "{statement}"
        );
    }
}

#[test]
fn managed_application_and_router_keep_constructors_as_native_rust() {
    let page = extract(&format!(
        r#"{STATE}
<App state="{{{{ Counter::new(owner)? }}}}"><main>
  <Router><Route path="/">Home</Route></Router>
</main></App>"#
    ))
    .unwrap();
    let rust = tokens(&page.rust);
    assert!(rust.contains(&tokens("::fusor_components::App::mount")),);
    assert!(rust.contains(&tokens("Counter::new(owner)?")));
    assert!(rust.contains(&tokens(
        "::fusor_router::browser::declarative::mount_routes"
    )));
    assert!(page.locations.iter().any(|location| location.line == 2));
    assert!(page.locations.iter().any(|location| location.line == 3));
    for markup in [
        r#"<App><main></main></App>"#,
        r#"<App state="Counter"><main></main></App>"#,
        r#"<App state="{{ Counter }}"><main></main></App><App state="{{ Other }}"><aside></aside></App>"#,
        r#"<App state="{{ Counter }}"><main rust:component="Counter"></main></App>"#,
    ] {
        assert!(
            extract(&format!("{STATE}{markup}")).is_err(),
            "accepted {markup}"
        );
    }
}

#[test]
fn slots_use_native_content_values_and_reject_conflicting_child_ownership() {
    let source = format!(
        r#"{STATE}
<section rust:component="Counter">
  <div rust:slot="state.body.clone()" rust:if="state.visible.get()" rust:key="state.reset.get()"></div>
</section>"#
    );
    let page = extract(&source).unwrap();
    let rust = tokens(&page.rust);
    assert!(rust.contains(&tokens("__fusor_scope.slot_with")));
    assert!(rust.contains(&tokens("::fusor::dom::Content")));
    assert!(rust.contains(&tokens("ChildPolicy::Managed")));
    assert!(!page.html.contains("rust:slot"));
    assert!(page.locations.iter().any(|location| location.line == 3));
    for markup in [
        r#"<div rust:slot=""></div>"#,
        r#"<div rust:slot="state.body"><Child></Child></div>"#,
        r#"<div rust:slot="state.body"><ForEach items="{{ state.rows }}" key="{{ |item| item.id }}"><li></li></ForEach></div>"#,
        r#"<div rust:slot="state.body"><b>Fallback</b></div>"#,
        r#"<div rust:slot="state.body">Fallback</div>"#,
        r#"<input rust:slot="state.body">"#,
        r#"<template rust:slot="state.body"></template>"#,
    ] {
        let source = format!("{STATE}<main rust:component=Counter>{markup}</main>");
        assert!(extract(&source).is_err(), "{markup}");
    }
}

#[test]
fn text_slots_preserve_mixed_markup_and_generate_native_rust() {
    let source = format!(
        "{STATE}\n<section rust:component=Counter><p>Count: {{{{ state.count.get() }}}} <strong>items</strong> · {{{{ state.count.get() * 2 }}}}</p></section>"
    );
    let page = extract(&source).unwrap();
    assert!(page.html.contains(
        "<p>Count: <!--fusor:0--><!--/fusor:0--> <strong>items</strong> · <!--fusor:1--><!--/fusor:1--></p>"
    ));
    assert!(tokens(&page.rust).contains(&tokens("state.count.get() * 2")));
    assert!(tokens(&page.rust).contains(&tokens("impl ::fusor::dom::Component for Counter")));
    assert!(!page.html.contains("rust:component"));
    assert!(!page.html.contains("state.count"));
    assert!(page.locations.iter().all(|location| location.line == 2));
}

#[test]
fn sole_child_text_reuses_bound_parents_without_allocating_unused_element_handles() {
    let source = format!(
        r#"{STATE}<main rust:component="Counter"><span>{{{{ state.first }}}}</span><button on:click="state.reset()">{{{{ state.second }}}}</button><b>{{{{ state.third }}}}</b><i title="{{{{ state.title }}}}"></i></main>"#
    );
    let page = extract(&source).unwrap();
    let rust = tokens(&page.rust);
    assert!(page.html.contains(r#"<span data-fusor-text="0"></span>"#));
    assert!(
        page.html
            .contains(r#"<button data-fusor-node="1" data-fusor-text="1"></button>"#)
    );
    assert!(page.html.contains(r#"<b data-fusor-text="2"></b>"#));
    assert!(page.html.contains(r#"<i data-fusor-node="2"></i>"#));
    assert!(!page.html.contains("<!--fusor:"));
    assert_eq!(page.html.matches("data-fusor-node=").count(), 2);
    assert!(rust.contains(&tokens("texts: &[],")));
    assert!(rust.contains(&tokens(
        "text_elements: &[
            ::fusor::template::TextElementDescriptor {
                id: ::fusor::template::TextId::new(0usize), host: ::std::option::Option::None, tag: \"span\",
            },
            ::fusor::template::TextElementDescriptor {
                id: ::fusor::template::TextId::new(1usize), host: ::std::option::Option::Some(::fusor::template::ElementId::new(1usize)), tag: \"button\",
            },
            ::fusor::template::TextElementDescriptor {
                id: ::fusor::template::TextId::new(2usize), host: ::std::option::Option::None, tag: \"b\",
            }
        ]"
    )));
    assert_eq!(rust.matches(&tokens("__fusor_nodes.take_text")).count(), 3);
    assert_eq!(
        rust.matches(&tokens("__fusor_nodes.take_element")).count(),
        2
    );
    assert_eq!(
        rust.matches(&tokens("template::ElementDescriptor")).count(),
        2
    );
}

#[test]
fn direct_text_requires_exact_native_element_contents() {
    for contents in [
        "{{ state.value }}{{ state.other }}",
        " {{ state.value }}",
        "{{ state.value }} ",
        "\n{{ state.value }}\n",
        "&#32;{{ state.value }}",
        "{{ state.value }}&#32;",
        "before {{ state.value }} after",
        "<!--before-->{{ state.value }}",
        "{{ state.value }}<!--after-->",
        "<b>sibling</b>{{ state.value }}",
        "{{ state.value }}<b>sibling</b>",
    ] {
        let page = extract(&format!(
            "{STATE}<main rust:component=Counter><p>{contents}</p></main>"
        ))
        .unwrap();
        assert!(!page.html.contains("data-fusor-text="), "{contents}");
        assert!(
            page.html.contains("<!--fusor:0--><!--/fusor:0-->"),
            "{contents}"
        );
        assert!(tokens(&page.rust).contains(&tokens("text_elements: &[]")));
    }
    // Preformatted containers strip a leading line feed while parsing HTML;
    // their opening text marker shields an SSR expression's initial newline.
    // `noscript` changes parsing modes when scripting is enabled. Preserve its
    // existing path instead of certifying it as an ordinary direct-text host.
    for host in [
        "x-widget",
        "button is='custom-button'",
        "pre",
        "listing",
        "noscript",
    ] {
        let closing = host.split_whitespace().next().unwrap();
        let page = extract(&format!(
            "{STATE}<main rust:component=Counter><{host}>{{{{ state.value }}}}</{closing}></main>"
        ))
        .unwrap();
        assert!(!page.html.contains("data-fusor-text="), "{host}");
        assert!(
            page.html.contains("<!--fusor:0--><!--/fusor:0-->"),
            "{host}"
        );
    }
}

#[test]
fn direct_text_covers_native_roots_and_inline_children_without_claiming_fragment_hosts() {
    for markup in [
        "<p rust:component=Counter>{{ state.value }}</p>",
        "<template rust:component=Counter><p>{{ state.value }}</p></template>",
        "<App state='{{ Counter::new(owner) }}'><p>{{ state.value }}</p></App>",
        "<main rust:component=Counter><Child><p>{{ state.value }}</p></Child></main>",
        "<main rust:component=Counter><ForEach items='{{ state.rows }}' key='{{ |item| item.id }}'><p>{{ item.get().title }}</p></ForEach></main>",
    ] {
        let page = extract(&format!("{STATE}{markup}")).unwrap();
        assert_eq!(page.html.matches("data-fusor-text=").count(), 1, "{markup}");
        assert!(!page.html.contains("<!--fusor:0-->"), "{markup}");
        assert!(tokens(&page.rust).contains(&tokens("__fusor_nodes.take_text")));
    }
    let page = extract(&format!(
        "{STATE}<main rust:component=Counter><Child>{{{{ state.value }}}}</Child></main>"
    ))
    .unwrap();
    assert!(!page.html.contains("data-fusor-text="));
    assert!(page.html.contains("<!--fusor:0--><!--/fusor:0-->"));
}

#[test]
fn server_direct_text_uses_escaped_writer_content_and_preserves_anchored_siblings() {
    for target in ["server", "shared"] {
        let page = extract(&format!(
            r#"{STATE}<main rust:component="Counter" rust:render="{target}"><p>{{{{ state.value }}}}</p><p>prefix {{{{ state.other }}}}</p></main>"#
        ))
        .unwrap();
        let rust = tokens(&page.rust);
        let direct = tokens(
            r#"__fusor_writer.static_markup("<main data-fusor-component=\"0\" data-fusor-version=\"3\"><p data-fusor-text=\"0\">", ::std::option::Option::Some(5usize), false);
                __fusor_writer.text(&(state.value));"#,
        );
        assert!(rust.contains(&direct), "{target}: {rust}");
        assert_eq!(
            rust.matches(&tokens("__fusor_writer.text(&(state.value))"))
                .count(),
            1
        );
        assert!(rust.contains(&tokens(
            r#"__fusor_writer.static_markup("</p><p>prefix <!--fusor:1-->", ::std::option::Option::Some(6usize), false);
                __fusor_writer.text(&(state.other));
                __fusor_writer.static_markup("<!--/fusor:1--></p></main>", ::std::option::Option::None, false);"#
        )));
        assert!(!page.html.contains("<!--fusor:0-->"));
        assert!(page.html.contains("<!--fusor:1--><!--/fusor:1-->"));
        if target == "shared" {
            assert!(rust.contains(&tokens("texts: &[::fusor::template::TextId::new(1usize)]")));
            assert_eq!(rust.matches(&tokens("__fusor_nodes.take_text")).count(), 2);
        }
    }
}

#[test]
fn handles_rust_blocks_strings_raw_strings_comments_and_html_entities() {
    let source = format!(
        r###"{STATE}
<p rust:component="Counter">{{{{ {{ let n = if 1 &lt; 2 {{ 3 }} else {{ 4 }}; format!("{{n}} }}}} {{}}", r#"}}}}"#) }} }}}}
{{{{ 1 /* }}}} */ + 2 }}}}
{{{{ 1 // }}}} is a comment, not the delimiter
+ 2 }}}}</p>"###
    );
    let page = extract(&source).unwrap();
    assert_eq!(page.html.matches("<!--fusor:").count(), 3);
    assert!(tokens(&page.rust).contains(&tokens("if 1 < 2")));
    assert!(page.rust.contains(r##"r#"}}"#"##));
    assert!(tokens(&page.rust).contains(&tokens("1 // }} is a comment, not the delimiter\n+ 2")));
}

#[test]
fn compiles_attributes_properties_events_and_two_way_bindings() {
    let source = format!(
        r#"{STATE}<section rust:component="Counter">
<button title="Count: {{{{ state.count.get() }}}} &amp; {{braces}}" disabled="{{{{ state.count.get() == 0 }}}}" on:click="state.count.set(0)">Reset</button>
<input bind="state.name" />
<input type="checkbox" bind="state.enabled" />
<input checked="{{{{ state.enabled.get() }}}}" value="{{{{ state.name.get() }}}}" />
<p class="label" class:active="state.enabled.get()" aria-hidden="{{{{ !state.enabled.get() }}}}">Text</p>
</section>"#
    );
    let page = extract(&source).unwrap();
    for method in ["attr", "on", "checked", "value", "class"] {
        assert!(
            tokens(&page.rust).contains(&tokens(&format!("__fusor_scope.{method}"))),
            "missing {method}"
        );
    }
    assert!(page.rust.contains("Count: {} & {{braces}}"));
    assert!(tokens(&page.rust).contains(&tokens("let value: bool")));
    assert!(!page.html.contains("on:click"));
    assert!(!page.html.contains("bind="));
    assert!(!page.html.contains("{{"));
}

#[test]
fn foreach_inline_rows_use_native_typed_bindings() {
    let source = format!(
        r#"{STATE}
<ul rust:component="Counter"><ForEach items="{{{{ state.items.get() }}}}" key="{{{{ |item| item.id }}}}">
<li data-id="{{{{ item.get().id }}}}">{{{{ index.get() }}}}: {{{{ item.get().title }}}}</li>
</ForEach></ul>"#
    );
    let page = extract(&source).unwrap();
    assert!(tokens(&page.rust).contains(&tokens("__fusor_scope.keyed")));
    assert!(tokens(&page.rust).contains(&tokens("fusor_components::ForEach::entries")));
    assert!(page.html.contains("<template data-fusor-component="));
    assert!(!page.html.contains("ForEach"));
    assert_eq!(
        tokens(&page.rust)
            .matches(&tokens("impl ::fusor::dom::Component"))
            .count(),
        1
    );
}

#[test]
fn loader_position_is_correct_when_bindings_precede_the_first_rust_block() {
    let source = format!("<p rust:component=Counter>こんにちは {{{{ 1 + 2 }}}}</p>{STATE}");
    let html = extract(&source).unwrap().with_loader();
    assert!(html.contains("</p><script type=\"module\" src=\"./boot.js\"></script>"));
}

#[test]
fn rust_inside_a_component_template_loads_outside_its_inert_content() {
    let source = "<template rust:component=Counter><p>{{ 7 }}</p><script type=text/rust>struct Counter;</script></template>";
    let page = extract(source).unwrap();
    assert!(
        page.with_loader()
            .starts_with("<script type=\"module\" src=\"./boot.js\"></script><template")
    );
    assert!(page.rust.contains("struct Counter;"));
    assert_eq!(page.with_loader().matches("src=\"./boot.js\"").count(), 1);
}

#[test]
fn malformed_or_ambiguous_binding_markup_fails_at_the_html_source() {
    for markup in [
        "<p rust:component=Counter>{{ }}</p>",
        "<p rust:component=Counter>{{ state.get() }</p>",
        "<p rust:component=Counter>{{ (1 + 2 }}</p>",
        "<p rust:component=Counter><b>{{ 1 }}</p>",
        "<p rust:component=Counter />",
        "<p rust:component=Counter><i rust:component=Child></i></p>",
        "<p rust:component=Counter></p><p rust:component=Counter></p>",
        "<button on:click='state.reset()'></button>",
        "<input rust:component=Counter bind='state.name' value='{{ state.name.get() }}'>",
        "<button rust:component=Counter disabled='prefix {{ true }}'></button>",
        "<p rust:component=Counter class='{{ state.classes() }}' class:active='true'></p>",
        "<p rust:component=Counter onclick='{{ state.code() }}'></p>",
        r#"<ul rust:component=Counter><ForEach items="{{ state.items() }}"><li></li></ForEach></ul>"#,
        "<p rust:component=Counter><template>{{ state.get() }}</template></p>",
        "<p rust:component=Counter><template><i title='{{ state.get() }}'></i></template></p>",
        "<template rust:component=Counter on:click='state.reset()'><p>Hello</p></template>",
        r#"<ul rust:component=Counter><li>Loading</li><ForEach items="{{ state.items() }}" key="{{ |item| item.id }}"><li>{{ item.get().title }}</li></ForEach></ul>"#,
        "<textarea rust:component=Counter>{{ state.name.get() }}</textarea>",
        "<p data-fusor-node='0'></p>",
    ] {
        let source = format!("{STATE}\n{markup}");
        let error = extract(&source).expect_err(markup);
        assert_eq!(error.line, 2, "{markup}: {error}");
    }
}

#[test]
fn script_and_style_strings_stay_literal_and_rust_syntax_is_left_to_rustc() {
    let source = format!(
        r#"{STATE}<section rust:component=Counter>
<script>const literal = "{{{{ not_rust }}}}";</script>
<style>/* {{{{ still_not_rust }}}} */</style>
<p>{{{{ this is invalid Rust syntax }}}}</p></section>"#
    );
    let page = extract(&source).unwrap();
    assert!(page.html.contains("{{ not_rust }}"));
    assert!(page.html.contains("{{ still_not_rust }}"));
    assert!(tokens(&page.rust).contains(&tokens("this is invalid Rust syntax")));
}

#[test]
fn validates_template_roots_before_any_browser_code_runs() {
    for template in [
        "<template rust:component=Counter></template>",
        "<template rust:component=Counter><p>One</p><p>Two</p></template>",
        "<template rust:component=Counter>Lost text<p>One</p></template>",
    ] {
        let source = format!("{STATE}\n{template}");
        let problem = extract(&source).expect_err(template);
        assert_eq!(problem.line, 2);
        assert!(problem.message.contains("root element"));
    }
    let source = "<template rust:component=Counter> &#32;<!-- comment --><p>{{ 1 }}</p><script type=text/rust>struct Counter;</script></template>";
    assert!(extract(source).is_ok());
}

#[test]
fn component_tags_replace_mount_constructors_in_every_render_target() {
    for target in ["", " rust:render=\"server\"", " rust:render=\"shared\""] {
        let page = extract(&format!(r#"{STATE}<section rust:component="Counter"{target}><crate::ui::Child value="{{{{ state.value.clone() }}}}" rust:if="state.visible.get()" rust:key="state.id.get()"></crate::ui::Child></section>"#)).unwrap();
        let rust = tokens(&page.rust);
        assert!(rust.contains(&tokens("FromInputs")));
        assert!(rust.contains(&tokens("value: { state.value.clone() }")));
        if !target.is_empty() {
            assert!(rust.contains(&tokens("__fusor_context.try_child_into_with_children")));
        }
    }
    let page = extract(&format!(r#"{STATE}<main rust:component="Counter"><section rust:async="state.view"><Child value="{{{{ state.value.clone() }}}}"></Child><section rust:await="state.data"><Child value="{{{{ ready.clone() }}}}"></Child></section></section></main>"#)).unwrap();
    let rust = tokens(&page.rust);
    assert!(rust.contains(&tokens("__fusor_frame.component_at")));
    assert_eq!(
        rust.matches(&tokens("__fusor_nodes.take_mount_point"))
            .count(),
        2
    );
}

#[test]
fn unknown_rust_directives_are_rejected_by_name() {
    for (markup, name) in [
        (
            "<div rust:component=Counter rust:mount='Child'></div>",
            "rust:mount",
        ),
        (
            "<main rust:component=Counter><ul rust:each='state.items'></ul></main>",
            "rust:each",
        ),
    ] {
        let error = extract(&format!("{STATE}{markup}")).unwrap_err();
        assert_eq!(error.message, format!("unknown Rust directive {name:?}"));
    }
}

#[test]
fn selective_and_coherent_contracts_reject_unsupported_authoring() {
    for markup in [
        r#"<main rust:component="Counter" rust:render="server"><div rust:async="state.view"></div></main>"#,
        r#"<main rust:component="Counter" rust:render="shared"><input value="{{ state.value }}"></main>"#,
        r#"<main rust:component="Counter" rust:render="shared"><ul><li>Stale placeholder</li><ForEach items="{{ state.items }}" key="{{ |item| item.id }}"><li>{{ item.get().title }}</li></ForEach></ul></main>"#,
        r#"<main rust:component="Counter"><div rust:async="state.view"><input bind="state.value"></div></main>"#,
        r#"<main rust:component="Counter"><div rust:async="state.view"><div rust:async="state.other"></div></div></main>"#,
        r#"<main rust:component="Counter"><div rust:async="state.view"><x-widget></x-widget></div></main>"#,
    ] {
        let error = extract(&format!("{STATE}\n{markup}")).expect_err(markup);
        assert_eq!(error.line, 2, "{markup}: {error}");
    }
}

#[test]
fn native_and_browser_lowerings_share_a_contract_and_refresh_ignores_embedded_literals() {
    let source = format!(
        r#"{STATE}<template rust:component="Counter" rust:render="shared"><p>{{{{ state.value }}}}</p></template>"#
    );
    let page = extract(&source).unwrap();
    assert!(page.rust.contains("fusor_server"));
    assert!(page.rust.contains("TEMPLATE_HASH"));
    assert!(page.rust.contains("TEMPLATE_HTML"));
    let changed = extract(&source.replace("<p>", "<p class='copy'>")).unwrap();
    assert_ne!(page.rust, changed.rust);
    assert_ne!(page.fingerprint, changed.fingerprint);
    let browser = source.replace(" rust:render=\"shared\"", "");
    assert_eq!(
        tokens(&extract(&browser).unwrap().fingerprint),
        tokens(
            &extract(&browser.replace("<p>", "<p class='copy'>"))
                .unwrap()
                .fingerprint
        )
    );
    let coherent=extract(&format!(r#"{STATE}<main rust:component="Counter"><section rust:async="state.view"><p rust:await="state.read">{{{{ ready.value }}}}</p></section></main>"#)).unwrap();
    assert!(tokens(&coherent.rust).contains("async_region"));
    assert!(tokens(&coherent.rust).contains("AsyncRead :: Ready"));
}

#[test]
fn component_tags_lower_native_inputs_aliases_and_explicit_content() {
    let source = format!(
        r#"{STATE}
<main rust:component="Counter">
  <widgets::CounterAlias count="{{{{ state.count.clone() }}}}" title="literal &amp; text" rust:if="state.visible.get()" rust:key="state.key.get()">
    <template rust:content="body"><section><p>{{{{ state.count.get() }}}}</p>
      <Panel><template rust:content="body"><strong>{{{{ state.count.get() }}}}</strong></template></Panel>
    </section></template>
  </widgets::CounterAlias>
</main>"#
    );
    let page = extract(&source).unwrap();
    let rust = tokens(&page.rust);
    assert!(rust.contains(&tokens(
        "<widgets::CounterAlias as ::fusor::FromInputs> ::Inputs"
    )),);
    assert!(rust.contains(&tokens("count: { state.count.clone() }")));
    assert!(rust.contains(&tokens("title: { \"literal & text\" }")));
    assert!(rust.contains(&tokens("::fusor::dom::Content::from_prepared")));
    assert!(rust.contains(&tokens("let __fusor_capture = ::std::rc::Rc::clone(state)")));
    assert_eq!(
        page.html.matches("<template data-fusor-component").count(),
        2
    );
    assert!(
        page.html
            .contains("<!--fusor:mount:0--><!--/fusor:mount:0-->")
    );
    assert!(!page.html.contains("widgets::CounterAlias"));
    assert!(!page.html.contains("rust:content"));
    assert!(page.html.find("</main>").unwrap() < page.html.find("<template").unwrap());
    assert!(page.locations.iter().any(|location| location.line == 3));
}

#[test]
fn component_tag_contract_rejects_ambiguous_inputs() {
    for markup in [
        r#"<Child count="before {{ value }}"></Child>"#,
        r#"<Child count="{{ a }} {{ b }}"></Child>"#,
        r#"<Child count="a" count="b"></Child>"#,
        r#"<Child Count="{{ value }}"></Child>"#,
        r#"<Child rust:slot="state.content"></Child>"#,
        r#"<Child on:click="handler()"></Child>"#,
        r#"<Child />"#,
        r#"<Child></child>"#,
        r#"<Child body="{{ content }}"><template rust:content="body"><p></p></template></Child>"#,
        r#"<Child><template rust:content="body"><p></p><p></p></template></Child>"#,
        r#"<Child><template rust:content="body">text</template></Child>"#,
        r#"<Child><template rust:content="body"><p></p><script type="text/rust">struct Hidden;</script></template></Child>"#,
        r#"<Child><template rust:content="body"><Other></Other></template></Child>"#,
        r#"<template rust:content="body"><p>outside a tag</p></template>"#,
        r#"<section rust:async="value"><Child><template rust:content="body"><p>Opaque</p></template></Child></section>"#,
    ] {
        let error = extract(&format!(
            "{STATE}\n<main rust:component=Counter>{markup}</main>"
        ))
        .expect_err(markup);
        assert!(error.line >= 2, "{markup}: {error}");
    }
    for markup in [
        r#"<template rust:component="Counter"><Child></Child></template>"#,
        r#"<main rust:component="Counter" rust:render="server"><Child><template rust:content="body"><p>Opaque</p></template></Child></main>"#,
        r#"<main rust:component="Counter" rust:render="shared"><Child><template rust:content="body"><p>Opaque</p></template></Child></main>"#,
    ] {
        assert!(extract(&format!("{STATE}{markup}")).is_err(), "{markup}");
    }
}

#[test]
fn component_tags_preserve_native_case_insensitive_html_and_table_parents() {
    let page = extract(&format!(r#"{STATE}<MAIN rust:component="Counter"><DIV><BUTTON>native</BUTTON><MY-WIDGET></MY-WIDGET></DIV><table><tbody><Row value="{{ 1 }}"></Row></tbody></table></MAIN>"#)).unwrap();
    assert!(page.html.contains("<BUTTON>native</BUTTON>"));
    assert!(page.html.contains("<MY-WIDGET>"));
    assert!(
        page.html
            .contains("<tbody><!--fusor:mount:0--><!--/fusor:mount:0--></tbody>")
    );
    assert!(!page.html.contains("<Row"));
}

#[test]
fn foreach_rejects_ambiguous_or_unsafe_scopes() {
    for markup in [
        r#"<ul><ForEach items="{{ state.items }}"><li></li></ForEach></ul>"#,
        r#"<ul><ForEach items="state.items" key="{{ |x| x.id }}"><li></li></ForEach></ul>"#,
        r#"<ul><ForEach items="{{ state.items }}" key="{{ |x| x.id }}" item="state"><li></li></ForEach></ul>"#,
        r#"<ul><ForEach items="{{ state.items }}" key="{{ |x| x.id }}" item="x" index="x"><li></li></ForEach></ul>"#,
        r#"<ul><ForEach items="{{ state.items }}" key="{{ |x| x.id }}"><li></li><li></li></ForEach></ul>"#,
        r#"<ul><li>Static sibling</li><ForEach items="{{ state.items }}" key="{{ |x| x.id }}"><li></li></ForEach></ul>"#,
        r#"<ul><ForEach items="{{ items }}" key="{{ key }}"><Row rust:if="true"></Row></ForEach></ul>"#,
        r#"<svg><g><ForEach items="{{ items }}" key="{{ key }}"><path></path></ForEach></g></svg>"#,
        r#"<ul><foreach items="{{ items }}" key="{{ key }}"><li></li></foreach></ul>"#,
    ] {
        assert!(
            extract(&format!(
                "{STATE}<main rust:component=Counter>{markup}</main>"
            ))
            .is_err(),
            "accepted {markup}"
        );
    }
}

#[test]
fn foreach_supports_named_nested_scopes() {
    let page = extract(&format!(r#"{STATE}<main rust:component="Counter"><ul>
<ForEach items="{{{{ state.groups }}}}" key="{{{{ |group| group.id }}}}" item="group" index="group_index">
<li><h2>{{{{ group.get().title }}}}</h2><ul><ForEach items="{{{{ group.get().items }}}}" key="{{{{ |item| item.id }}}}">
<li>{{{{ group_index.get() }}}}.{{{{ index.get() }}}}: {{{{ item.get().title }}}}</li>
</ForEach></ul></li></ForEach></ul></main>"#)).unwrap();
    assert!(tokens(&page.rust).contains(&tokens("let group_index")));
    assert!(tokens(&page.rust).contains(&tokens("let index")));
    assert!(!page.html.contains("ForEach"));
}

#[test]
fn foreach_refresh_ignores_static_row_text_but_tracks_rust() {
    let source = format!(
        r#"{STATE}<ul rust:component="Counter"><ForEach items="{{{{ state.items.get() }}}}" key="{{{{ |item| item.id }}}}"><li>First {{{{ item.get().title }}}}</li></ForEach></ul>"#
    );
    let before = extract(&source).unwrap();
    assert_eq!(
        tokens(&before.fingerprint),
        tokens(
            &extract(&source.replace("First", "Other"))
                .unwrap()
                .fingerprint
        )
    );
    assert_ne!(
        tokens(&before.fingerprint),
        tokens(
            &extract(&source.replace("item.get().title", "item.get().other"))
                .unwrap()
                .fingerprint
        )
    );
}

#[test]
fn app_boundary_validates_structure() {
    for markup in [
        r#"<App state="{{ build() }}" />"#,
        r#"<App state="{{ build() }}"></App>"#,
        r#"<App state="{{ build() }}"><main></main><aside></aside></App>"#,
        r#"<App state="{{ build() }}">text<main></main></App>"#,
        r#"<App state="{{ build() }}"><Other></Other></App>"#,
        r#"<App state="{{ build() }}"><main><App state="{{ build() }}"><p></p></App></main></App>"#,
        r#"<App state="{{ build() }}"><template><p></p></template></App>"#,
        r#"<App state="{{ build() }}"><main rust:render="shared"></main></App>"#,
        r#"<App state="{{ build() }}" class="page"><main></main></App>"#,
        r#"<App state="{{ build() }}"><main></main></app>"#,
        r#"<app state="{{ build() }}"><main></main></app>"#,
    ] {
        assert!(extract(markup).is_err(), "accepted {markup}");
    }
    let html = r#"<App state="{{ factory(owner)? }}"><main><p>{{ state.title }}</p></main></App>"#;
    let page = extract(&format!("{STATE}{html}")).unwrap();
    assert!(!page.html.contains("<App"));
    assert!(!page.html.contains("</App>"));
    assert!(page.html.contains("<main data-fusor-component="));
    assert!(!page.rust.contains("impl ::fusor::dom::Component for"));
    assert!(tokens(&page.rust).contains(&tokens("factory(owner)?")));
}

#[test]
fn children_lower_to_lazy_fragments_with_lexical_bindings_and_no_wrapper() {
    let page = extract(&format!(r#"{STATE}
<main rust:component="Counter"><Panel>Hello {{{{ state.name.get() }}}}!<p>One</p><p>Two</p></Panel></main>
<template rust:component="Panel"><section><Children></Children></section></template>"#)).unwrap();
    let rust = tokens(&page.rust);
    assert!(rust.contains(&tokens("::fusor::dom::Children::new")));
    assert!(rust.contains("prepare_fragment"));
    assert!(rust.contains(&tokens("__FUSOR_MOUNTS, parent")));
    assert!(!rust.contains("prepare_owner"));
    assert!(rust.contains("children_at"));
    assert!(!page.html.contains("<Children>"));
    assert!(!page.html.contains("<div>"));
    assert!(page.html.contains("<p>One</p><p>Two</p>"));
    assert!(page.locations.iter().any(|location| location.line == 2));
}

#[test]
fn children_reject_duplicate_placement_attributes_fallbacks_and_app_forwarding() {
    for markup in [
        "<Children></Children><Children></Children>",
        "<Children name=body></Children>",
        "<Children>Fallback</Children>",
        "<Children />",
        "<children></children>",
        "<Panel><Children></Children></Panel><Children></Children>",
        "<ul><ForEach items=\"{{ state.items.get() }}\" key=\"{{ |item| item.id }}\"><li><Children></Children></li></ForEach></ul>",
    ] {
        assert!(
            extract(&format!(
                "{STATE}<template rust:component=Counter><section>{markup}</section></template>"
            ))
            .is_err(),
            "accepted {markup}"
        );
    }
    assert!(extract(&format!("{STATE}<App state=\"{{{{ Counter }}}}\"><main><Panel><Children></Children></Panel></main></App>")).is_err());
    assert!(
        extract(&format!(
            "{STATE}<template rust:component=Counter><Children></Children></template>"
        ))
        .is_err()
    );
}

#[test]
fn children_forward_once_and_empty_invocations_need_no_factory() {
    let page = extract(&format!("{STATE}<template rust:component=Counter><section><Panel><Children></Children></Panel></section></template>")).unwrap();
    assert!(tokens(&page.rust).contains("__fusor_forward"));
    let empty = extract(&format!(
        "{STATE}<main rust:component=Counter><Panel></Panel></main>"
    ))
    .unwrap();
    assert!(!tokens(&empty.rust).contains(&tokens("::fusor::dom::Children::new")));
}

#[test]
fn children_html_changes_rebuild_instead_of_statically_refreshing_live_fragments() {
    let source = format!("{STATE}<main rust:component=Counter><Panel><p>Before</p></Panel></main>");
    let first = extract(&source).unwrap();
    let second = extract(&source.replace("Before", "After")).unwrap();
    assert_ne!(first.fingerprint, second.fingerprint);
    let shared = source.replace(
        "rust:component=Counter",
        "rust:component=Counter rust:render=shared",
    );
    let first = extract(&shared).unwrap();
    let second = extract(&shared.replace("Before", "After")).unwrap();
    fn hashes(rust: &str) -> Vec<&str> {
        rust.split("TEMPLATE_HASH")
            .skip(1)
            .map(|part| part.split(';').next().unwrap())
            .collect()
    }
    assert_ne!(hashes(&first.rust), hashes(&second.rust));
}

#[test]
fn async_components_infer_boundaries_and_name_values_without_dom_wrappers() {
    let source = format!(
        r#"{STATE}<main rust:component=Counter>
<Async><section><h2>{{{{ state.title.get() }}}}</h2><Await value="{{{{ state.read }}}}" let="result"><p>{{{{ result.as_str() }}}}</p></Await></section></Async>
<Await value="{{{{ state.other }}}}" let="other"><aside>{{{{ other.as_str() }}}}</aside></Await>
</main>"#
    );
    let page = extract(&source).unwrap();
    let rust = tokens(&page.rust);
    assert!(rust.contains("AsyncBoundary :: coherent"));
    assert!(rust.contains("AsyncRead :: Ready (result)"));
    assert!(rust.contains("AsyncRead :: Ready (other)"));
    assert!(!page.html.contains("<Async"));
    assert!(!page.html.contains("<Await"));
    assert!(!page.html.contains("let="));
    assert!(page.locations.iter().any(|location| location.line == 2));
    let explicit =
        extract(&source.replace("<Async>", "<Async boundary=\"{{ state.view }}\"> ")).unwrap();
    assert!(tokens(&explicit.rust).contains("(state . view) . clone"));
}

#[test]
fn async_components_validate_roots_scopes_inputs_and_render_targets() {
    for markup in [
        "<Async></Async>",
        "<Async><p></p><p></p></Async>",
        "<Async>text<p></p></Async>",
        "<Async><Panel></Panel></Async>",
        "<Async><input></Async>",
        "<Async class=bad><p></p></Async>",
        "<Async boundary=state.view><p></p></Async>",
        "<Async><section><Async><p></p></Async></section></Async>",
        "<async><p></p></async>",
        "<Await value=\"{{ state.read }}\"><p></p></Await>",
        "<Await value=\"{{ state.read }}\" let=state><p></p></Await>",
        "<Await value=\"{{ state.read }}\" let=\"a + b\"><p></p></Await>",
        "<Await value=\"{{ state.read }}\" let=result><p></p></await>",
        "<Await value=\"{{ state.read }}\" let=result><p><input bind=state.draft></p></Await>",
        "<Await value=\"{{ state.read }}\" let=result><div><Await value=\"{{ state.other }}\" let=result><p></p></Await></div></Await>",
    ] {
        assert!(
            extract(&format!(
                "{STATE}<main rust:component=Counter>{markup}</main>"
            ))
            .is_err(),
            "accepted {markup}"
        );
    }
    assert!(extract(&format!("{STATE}<template rust:component=Counter rust:render=shared><Await value=\"{{{{ state.read }}}}\" let=result><p></p></Await></template>")).is_err());
    // Structural built-ins may surround a reusable component's native root.
    assert!(extract(&format!("{STATE}<template rust:component=Counter><Async><Await value=\"{{{{ state.read }}}}\" let=result><p>{{{{ result }}}}</p></Await></Async></template>")).is_ok());
}

#[test]
fn native_root_keeps_text_optimization_and_region_finalization_together() {
    let page = extract(&format!(
        r#"{STATE}
<main rust:component="Counter" rust:async="state.boundary">{{{{ state.value }}}}</main>"#
    ))
    .unwrap();
    assert!(page.html.contains("data-fusor-text=\"0\""));
    assert!(!page.html.contains("rust:async"));
    let rust = tokens(&page.rust);
    assert!(rust.contains("async_region"));
    assert!(rust.contains(&tokens("__fusor_frame.text")));
    assert!(page.locations.iter().any(|location| location.line == 2));
    syn::parse_file(&page.rust).unwrap();

    let page = extract(&format!(
        r#"{STATE}<main rust:component="Counter">
<Await value="{{{{ state.first }}}}" let="outer"><Await value="{{{{ state.second }}}}" let="inner">
<article>{{{{ outer }}}} / {{{{ inner }}}}</article>
</Await></Await></main>"#
    ))
    .unwrap();
    let rust = tokens(&page.rust);
    assert!(rust.contains(&tokens("AsyncRead::Ready(outer)")));
    assert!(rust.contains(&tokens("AsyncRead::Ready(inner)")));
    assert_eq!(rust.matches(&tokens(".read(__fusor_attempt)")).count(), 2);
    syn::parse_file(&page.rust).unwrap();
}

#[test]
fn async_root_and_spelling_errors_stay_on_the_authored_closing_tag() {
    let root = "Async and Await require exactly one native HTML root";
    for (opening, body, closing, message) in [
        ("<Async>", "", "</Async>", root),
        (
            "<Async>",
            "<section></section><aside></aside>",
            "</Async>",
            root,
        ),
        (
            "<Await value=\"{{ state.read }}\" let=\"result\">",
            "<section></section>",
            "</await>",
            "close Await with </Await>",
        ),
    ] {
        let source =
            format!("{STATE}\n<main rust:component=Counter>\n{opening}{body}\n{closing}\n</main>");
        let error = extract(&source).unwrap_err();
        assert_eq!((error.line, error.column), (4, 1), "{source}");
        assert_eq!(error.message, message);
    }
}

#[test]
fn custom_properties_and_events_preserve_authored_case_and_ownership() {
    let page = extract(&format!(r#"{STATE}<main rust:component="Counter"><a-widget prop:someValue="state.value.get()" on:ValueChanged="state.changed(event)"></a-widget></main>"#)).unwrap();
    let code = tokens(&page.rust);
    assert!(
        code.contains("property (& __fusor_element_1 , \"someValue\""),
        "{code}"
    );
    assert!(code.contains("\"ValueChanged\""));
    assert!(!page.html.contains("prop:"));
    let mixed = extract(&format!(r#"{STATE}<main rust:component="Counter"><a-widget PROP:someValue="state.value" On:ValueChanged="state.changed(event)"></a-widget></main>"#)).unwrap();
    assert!(tokens(&mixed.rust).contains("\"someValue\""));
    assert!(tokens(&mixed.rust).contains("\"ValueChanged\""));
    assert!(!mixed.html.contains("PROP:"));
    for markup in [
        r#"<div prop:someValue="state.value"></div>"#,
        r#"<x-widget prop:innerHTML="state.value"></x-widget>"#,
        r#"<x-widget prop:onchange="state.value"></x-widget>"#,
        r#"<x-widget prop:foo="state.a" prop:Foo="state.b"></x-widget>"#,
    ] {
        extract(&format!(
            r#"{STATE}<main rust:component="Counter">{markup}</main>"#
        ))
        .expect_err(markup);
    }
    extract(&format!(r#"{STATE}<main rust:component="Counter" rust:render="shared"><x-widget prop:value="state.value"></x-widget></main>"#)).unwrap_err();
    extract(&format!(r#"{STATE}<main rust:component="Counter"><div rust:async="state.view"><x-widget prop:value="state.value"></x-widget></div></main>"#)).unwrap_err();
}

#[test]
fn component_javascript_is_extracted_once_and_page_modules_keep_native_behavior() {
    let source = r#"<script type="text/rust">struct Scene; struct App;</script>
<script type="module">globalThis.pageRuns = 1;</script>
<App state="{{ App }}"><script type="module">import utility from 'utility'; export function onMount({ root }) { root.dataset.ready = utility(); }</script><main></main></App>
<template rust:component="Scene"><script type="module" src="./scene.ts"></script><section><canvas></canvas></section></template>"#;
    let page = fusor_build::extract(source).unwrap();
    assert_eq!(page.javascript.len(), 2);
    assert_eq!(page.javascript[0].component, "App");
    assert!(
        page.javascript[0]
            .content
            .contains("export function onMount")
    );
    assert_eq!(page.javascript[1].src.as_deref(), Some("./scene.ts"));
    assert!(page.html.contains("globalThis.pageRuns = 1"));
    assert!(!page.html.contains("export function onMount"));
    assert!(!page.html.contains("./scene.ts"));
    assert!(page.rust.contains("InputSource"));
    assert_eq!(page.rust.matches(":: fusor :: js :: mount").count(), 2);
}

#[test]
fn component_modules_diagnose_placement_duplicates_and_bad_sources() {
    for (body, message) in [
        (
            r#"<template rust:component="Scene"><section><script type="module"></script></section></template>"#,
            "direct children",
        ),
        (
            r#"<template rust:component="Scene"><script type="module"></script><script type="module"></script><section></section></template>"#,
            "one module",
        ),
        (
            r#"<template rust:component="Scene" rust:render="shared"><script type="module"></script><section></section></template>"#,
            "server/island",
        ),
        (
            r#"<template rust:component="Scene"><script type="module" src="https://example.org/module.js"></script><section></section></template>"#,
            "relative",
        ),
        (
            r#"<template rust:component="Scene"><script type="module" src="./scene.js">run()</script><section></section></template>"#,
            "empty body",
        ),
        (
            r#"<template rust:component="Scene"><script type="module" async></script><section></section></template>"#,
            "optional src",
        ),
        (
            r#"<template rust:component="Scene"><section><ForEach items="{{ items }}" key="{{ item.id }}" item="item"><script type="module"></script><article></article></ForEach></section></template>"#,
            "direct children",
        ),
    ] {
        let source = format!("<script type=\"text/rust\">struct Scene;</script>\n{body}");
        let error = fusor_build::extract(&source).unwrap_err();
        assert_eq!(error.line, 2);
        assert!(error.message.contains(message), "{body}: {}", error.message);
    }
}

#[test]
fn routes_validate_structure_patterns_and_local_names() {
    for markup in [
        r#"<Route path="/">Outside</Route>"#,
        r#"<Router><p>Invalid</p></Router>"#,
        r#"<Router>Invalid</Router>"#,
        r#"<Router><Router></Router></Router>"#,
        r#"<Router base="/"></Router>"#,
        r#"<Router><Route>Missing path</Route></Router>"#,
        r#"<Router><Route path="/:id" let="state"></Route></Router>"#,
        r#"<Router><Route path="/:type" let="params"></Route></Router>"#,
        r#"<Router><Route path="/:id/:id"></Route></Router>"#,
        r#"<Router><Route path="/*/edit"></Route></Router>"#,
        r#"<Router><Route path="/:id"></Route><Route path="/:slug"></Route></Router>"#,
        r#"<Router><Route fallback></Route><Route fallback></Route></Router>"#,
        r#"<Router><Route fallback path="/"></Route></Router>"#,
        r#"<Router><Route path="/" other="x"></Route></Router>"#,
        r#"<router></router>"#,
        r#"<Router><Route path="/" /></Router>"#,
        r#"<select><Router></Router></select>"#,
        r#"<svg><Router></Router></svg>"#,
    ] {
        let result = extract(&format!(
            "{STATE}<main rust:component=Counter>{markup}</main>"
        ));
        assert!(result.is_err(), "accepted {markup}");
    }
    let result = extract(&format!(
        r#"{STATE}<main rust:component="Counter"><Router>
        <Route path="/articles/:slug" let="params"><p>{{{{ params.slug }}}}</p></Route>
        <Route path="/articles/new"><p>New</p></Route>
        <Route fallback>Missing</Route>
    </Router></main>"#
    ))
    .unwrap();
    assert!(!result.html.contains("<Router"));
    assert!(result.rust.contains("slug"));
    assert!(result.locations.iter().any(|location| location.line == 2));
}

#[test]
fn route_fragments_preserve_children_exclusivity_and_refresh_identity() {
    let source = format!(
        r#"{STATE}<template rust:component="Counter"><main><Router>
      <Route path="/"><Children></Children></Route>
      <Route path="/other"><Children></Children></Route>
    </Router></main></template>"#
    );
    extract(&source).unwrap();
    assert!(extract(&source.replace("</Router>", "</Router><Children></Children>")).is_err());
    assert!(
        extract(&source.replace(
            "<Children></Children>",
            "<Children></Children><Children></Children>"
        ))
        .is_err()
    );
    let app = format!(
        r#"{STATE}<App state="{{{{ Counter }}}}"><main><Router><Route path="/"><Children></Children></Route></Router></main></App>"#
    );
    assert!(extract(&app).is_err());
    let first = extract(&source).unwrap();
    let second = extract(&source.replace("/other", "/next")).unwrap();
    assert_ne!(first.fingerprint, second.fingerprint);
    let body = source.replace("<Children></Children>", "<p>Before</p>");
    assert_ne!(
        extract(&body).unwrap().fingerprint,
        extract(&body.replace("Before", "After"))
            .unwrap()
            .fingerprint
    );
}

#[test]
fn hydration_tags_use_typed_props_and_existing_native_delivery() {
    let page = extract(&format!(
        r#"{STATE}
<main rust:component="Counter" rust:render="server">
  <catalog::Cart hydrate="visible" hydrate:prefetch="idle"
      product_id="{{{{ state.id }}}}" title="Your &quot;cart&quot;" quantity="1"></catalog::Cart>
  <button type="button" hydrate:target="designer">Open</button>
  <Designer hydrate="interaction" hydrate:id="designer" product_id="{{{{ 42 }}}}"></Designer>
</main>"#
    ))
    .unwrap();
    let rust = tokens(&page.rust);
    assert!(rust.contains(&tokens(
        "<catalog::Cart as ::fusor_islands::Island> ::Props"
    )));
    assert!(rust.contains(&tokens("product_id: { state.id }")));
    assert!(rust.contains(&tokens(
        r#"title: ::core::convert::Into::into("Your \"cart\"")"#
    )));
    assert!(rust.contains(&tokens("::fusor_islands::Activation::Visible")));
    assert!(rust.contains(&tokens("::fusor_islands::Prefetch::Idle")));
    assert!(rust.contains("prepare_island"));
    assert!(
        page.html
            .contains("data-fusor-activate-target=\"designer\"")
    );
    assert!(page.html.contains("id=\"designer\""));
    assert!(!page.html.contains("<catalog::Cart"));
    assert!(!page.html.contains("hydrate="));
    assert!(page.locations.iter().any(|location| location.line == 3));
}

#[test]
fn hydration_rejects_ambiguous_policies_placement_and_owned_contents() {
    for (markup, message) in [
        (r#"<Cart hydrate="hover"></Cart>"#, "hydrate must be"),
        (
            r#"<Cart hydrate="{{ state.policy }}"></Cart>"#,
            "hydrate must be",
        ),
        (
            r#"<Cart hydrate="load" hydrate:prefetch="interaction"></Cart>"#,
            "hydrate:prefetch must be",
        ),
        (
            r#"<Cart hydrate="interaction"></Cart>"#,
            "requires hydrate:id",
        ),
        (
            r#"<Cart hydrate="load" hydrate:id=""></Cart>"#,
            "nonempty static",
        ),
        (
            r#"<Cart hydrate="load" hydrate:id="{{ state.id }}"></Cart>"#,
            "nonempty static",
        ),
        (r#"<Cart hydrate="load"/>"#, "explicit closing tag"),
        (r#"<Cart hydrate="load"></cart>"#, "close Cart with </Cart>"),
        (r#"<Cart hydrate="load">lost</Cart>"#, "must be empty"),
        (
            r#"<Cart hydrate="load"><p>lost</p></Cart>"#,
            "must be empty",
        ),
        (
            r#"<Cart hydrate="load" rust:if="state.show"></Cart>"#,
            "cannot use rust:if",
        ),
        (
            r#"<Cart hydrate="load" hydrate:prefech="idle"></Cart>"#,
            "unknown attribute hydrate:prefech; component tags accept hydrate, hydrate:id, hydrate:prefetch",
        ),
        (
            r#"<Cart hydrate:id="cart"></Cart>"#,
            "hydrate:id requires hydrate on the same component tag",
        ),
        (
            r#"<Cart hydrate="load" hydrate:target="cart"></Cart>"#,
            "hydrate:target belongs on the native button that activates an island",
        ),
        (r#"<div hydrate="load"></div>"#, "Rust component tag"),
        (r#"<div hydrate:prefetch="idle"></div>"#, "hydrate belongs"),
        (
            r#"<table><Cart hydrate="load"></Cart></table>"#,
            "div boundary",
        ),
        (
            r#"<button hydrate:target="cart">Open</button>"#,
            "type=button",
        ),
        (r#"<a hydrate:target="cart">Open</a>"#, "native type=button"),
    ] {
        let source = format!(
            r#"{STATE}<main rust:component="Counter" rust:render="server">{markup}</main>"#
        );
        let failure = extract(&source).expect_err(markup);
        assert!(failure.message.contains(message), "{markup}: {failure}");
    }
    for target in ["", " rust:render=\"shared\""] {
        let source = format!(
            r#"{STATE}<main rust:component="Counter"{target}><Cart hydrate="load"></Cart></main>"#
        );
        assert!(
            extract(&source)
                .unwrap_err()
                .message
                .contains("server-rendered")
        );
    }
}

#[test]
fn generated_native_and_template_roots_prepare_the_final_owner_before_the_factory() {
    for markup in [
        "<main rust:component=Counter><span>{{ state.value }}</span></main>",
        "<template rust:component=Counter><main>{{ state.value }}</main></template>",
    ] {
        let page = extract(&format!("{STATE}{markup}")).unwrap();
        let rust = tokens(&page.rust);
        let prepare = rust
            .find("prepare_with_binding_bundle")
            .expect("prepared descriptor entry point");
        let factory = rust
            .find(&tokens("prepare_state(__fusor_scope.owner(), make)"))
            .expect("factory receives prepared owner");
        assert!(
            prepare < factory,
            "descriptor must validate before the factory"
        );
        assert!(rust.contains(&tokens(
            "__FUSOR_TEMPLATE.prepare_with_binding_bundle({ Self::TEMPLATE_HTML }, parent)?"
        )));
        assert!(!rust.contains("prepare_owner"));
    }
}

#[test]
fn binding_bundle_uses_dense_ordinals_and_defers_typed_extraction_to_fallback() {
    // Earlier components consume IDs. Direct text precedes anchored text in
    // source order, but the bundle groups anchored texts before direct texts.
    let page = extract(&format!(
        r#"{STATE}
<template rust:component=Prelude><main title="{{{{ state.title }}}}">{{{{ state.value }}}}</main></template>
<template rust:component=Counter><section title="{{{{ state.title }}}}"><span>{{{{ state.direct }}}}</span><p>before {{{{ state.anchored }}}} after</p><button on:click="state.click()">+</button></section></template>"#
    ))
    .unwrap();
    let rust = tokens(&page.rust);
    let start = rust
        .find(&tokens("impl ::fusor::dom::Component for Counter"))
        .unwrap();
    let rust = &rust[start..];
    assert!(rust.contains(&tokens(
        "__fusor_scope.bundle_attr_value(&__fusor_bundle, 0u32, \"title\", move || { use ::fusor::dom::text_value::Convert as _; (& ::fusor::dom::text_value::Value(&(state.title))).__fusor_into_text() })? ;"
    )), "{rust}");
    assert!(rust.contains(&tokens(
        "__fusor_scope.bundle_on(&__fusor_bundle, 1u32, \"click\", move |event| { state.click() })? ;"
    )));
    assert!(rust.contains(&tokens(
        "__fusor_scope.bundle_text_node_value(&__fusor_bundle, 3u32, move || { use ::fusor::dom::text_value::Convert as _; (& ::fusor::dom::text_value::Value(&(state.direct))).__fusor_into_text() })? ;"
    )));
    assert!(rust.contains(&tokens(
        "__fusor_scope.bundle_text_node_value(&__fusor_bundle, 2u32, move || { use ::fusor::dom::text_value::Convert as _; (& ::fusor::dom::text_value::Value(&(state.anchored))).__fusor_into_text() })? ;"
    )));
    let branch = rust.find("take_binding_bundle").unwrap();
    assert!(branch < rust.find(&tokens("__fusor_nodes.take_element")).unwrap());
    assert!(branch < rust.find(&tokens("__fusor_nodes.take_text")).unwrap());
    assert!(rust.contains("set_coherent_renderer"));
    assert!(rust.contains(&tokens("__fusor_scope.text_node_value")));
    assert!(!rust.contains("ElementId :: new (0usize)"));
    assert!(!rust.contains("TextId :: new (0usize)"));
}

#[test]
fn binding_bundle_leaves_controls_and_other_binding_kinds_on_typed_fallback() {
    for contents in [
        r#"<input on:click="state.click()">"#,
        r#"<textarea title="{{ state.title }}"></textarea>"#,
        r#"<select on:change="state.change()"><option>one</option></select>"#,
        r#"<input type="text" bind="state.value">"#,
        r#"<p class:active="state.active">text</p>"#,
        r#"<div rust:slot="state.content"></div>"#,
        r#"<Child></Child>"#,
        r#"<Children></Children>"#,
        r#"<ul><ForEach items="{{ state.rows }}" key="{{ |item| item.id }}"><li>fixed</li></ForEach></ul>"#,
        r#"<section rust:async="state.view"><span>{{ state.value }}</span></section>"#,
        r#"<p>static only</p>"#,
    ] {
        let page = extract(&format!(
            "{STATE}<template rust:component=Counter><main>{contents}</main></template>"
        ))
        .unwrap();
        let rust = tokens(&page.rust);
        assert!(!rust.contains("prepare_with_binding_bundle"), "{contents}");
        assert!(!rust.contains("take_binding_bundle"), "{contents}");
        assert!(rust.contains("prepare_with_points"), "{contents}");
    }
}

#[test]
fn binding_bundle_does_not_claim_projected_fragments() {
    let page = extract(&format!(
        "{STATE}<template rust:component=Counter><main><Child><strong>{{{{ state.value }}}}</strong></Child></main></template>"
    ))
    .unwrap();
    let rust = tokens(&page.rust);
    assert!(rust.contains("prepare_fragment"));
    assert!(!rust.contains("prepare_with_binding_bundle"));
}

#[test]
fn binding_bundle_keeps_large_flat_children_eligible_under_managed_parents() {
    let mut contents = String::new();
    for index in 0..256 {
        contents.push_str(&format!(
            "<output title='{{{{ state.value }}}}'>{{{{ state.direct_{index} }}}}</output><p>[{{{{ state.anchored_{index} }}}}]</p>"
        ));
    }
    let page = extract(&format!(
        "{STATE}<template rust:component=Counter><main><Large></Large></main></template><template rust:component=Large><section>{contents}</section></template>"
    ))
    .unwrap();
    let rust = tokens(&page.rust);
    let parent_start = rust
        .find(&tokens("impl ::fusor::dom::Component for Counter"))
        .unwrap();
    let child_start = rust
        .find(&tokens("impl ::fusor::dom::Component for Large"))
        .unwrap();
    let parent = &rust[parent_start..child_start];
    let child = &rust[child_start..];
    assert!(!parent.contains("prepare_with_binding_bundle"));
    assert!(parent.contains("prepare_with_points"));
    assert!(child.contains("prepare_with_binding_bundle"));
    assert_eq!(child.matches("bundle_text_node_value").count(), 512);
    assert_eq!(child.matches("bundle_attr").count(), 256);
    assert!(child.contains(&tokens(
        "__fusor_scope.bundle_text_node_value(&__fusor_bundle, 767u32, move || { use ::fusor::dom::text_value::Convert as _; (& ::fusor::dom::text_value::Value(&(state.direct_255))).__fusor_into_text() })? ;"
    )), "{child}");
    assert!(child.contains(&tokens(
        "__fusor_scope.bundle_text_node_value(&__fusor_bundle, 511u32, move || { use ::fusor::dom::text_value::Convert as _; (& ::fusor::dom::text_value::Value(&(state.anchored_255))).__fusor_into_text() })? ;"
    )));
}

#[test]
fn foreach_forwarding_rows_omit_only_proven_unused_index_projections() {
    for target in ["", " rust:render=shared", " rust:render=server"] {
        let page = extract(&format!(
            r#"{STATE}<template rust:component=Counter{target}><ul><ForEach items="{{{{ state.rows }}}}" key="{{{{ |item| item.id }}}}"><Row value="{{{{ item.clone() }}}}" label="{{{{ state.label.clone() }}}}"></Row></ForEach></ul></template>"#
        )).unwrap();
        let rust = tokens(&page.rust);
        if target != " rust:render=server" {
            assert!(
                rust.contains(&tokens("::fusor_components::ForEach::item_row")),
                "{rust}"
            );
            assert!(!rust.contains(&tokens("::fusor_components::ForEach::row")));
        }
        if !target.is_empty() {
            assert!(rust.contains(&tokens("::fusor_components::ForEach::server_item_row")));
            assert!(!rust.contains(&tokens("::fusor_components::ForEach::server_row")));
        }
        assert!(rust.contains(&tokens("let item = &__fusor_context_0.item;")));
        assert!(!rust.contains(&tokens("let index = &__fusor_context_0.index;")));
    }
}

#[test]
fn foreach_index_proof_falls_back_for_raw_names_macros_attributes_and_context_escapes() {
    for (index, expression) in [
        ("index", "index.get()"),
        ("index", "r#index.get()"),
        ("position", "position.get()"),
        ("position", "r#position.get()"),
        ("index", "move || r#index.get()"),
        ("index", "hidden_index!()"),
        ("index", "identity(|| hidden_index!())"),
        (
            "index",
            "{ #[allow(unused)] let copy = item.clone(); copy }",
        ),
        ("index", "identity(__fusor_context_0)"),
        ("index", "identity(r#__fusor_context_0)"),
        ("index", "!state.flag"),
        ("index", "state.café()"),
        ("index", "{ let index = 0; item.clone() }"),
    ] {
        let page = extract(&format!(
            r#"{STATE}<template rust:component=Counter><ul><ForEach items="{{{{ state.rows }}}}" key="{{{{ |item| item.id }}}}" index="{index}"><Row value="{{{{ {expression} }}}}"></Row></ForEach></ul></template>"#
        )).unwrap();
        let rust = tokens(&page.rust);
        assert!(
            !rust.contains(&tokens("::fusor_components::ForEach::item_row")),
            "{expression}"
        );
        assert!(
            rust.contains(&tokens("::fusor_components::ForEach::row")),
            "{expression}"
        );
        assert!(
            rust.contains(&tokens(&format!("let {index} = &__fusor_context_0.index;"))),
            "{expression}"
        );
    }
    let page = extract(&format!(
        r#"{STATE}<template rust:component=Counter><ul><ForEach items="{{{{ state.rows }}}}" key="{{{{ |item| item.id }}}}" item="value" index="position"><Row value="{{{{ r#value.clone() }}}}"></Row></ForEach></ul></template>"#
    )).unwrap();
    assert!(tokens(&page.rust).contains(&tokens("::fusor_components::ForEach::item_row")));
}

#[test]
fn foreach_index_proof_keeps_descendant_and_nested_environments_complete() {
    for contents in [
        r#"<Row value="{{ item.clone() }}"><span>{{ index.get() }}</span></Row>"#,
        r#"<Row value="{{ item.clone() }}"><!-- static child --></Row>"#,
        r#"<Row value="{{ item.clone() }}"><span>fixed child</span></Row>"#,
        r#"<li><Row value="{{ item.clone() }}" rust:if="state.visible"></Row></li>"#,
        r#"<li><Row value="{{ item.clone() }}" rust:key="item.get().id"></Row></li>"#,
        r#"<li on:click="state.click(index.get())">{{ item.get() }}</li>"#,
        r#"<li><ul><ForEach items="{{ item.get().children }}" key="{{ |item| item.id }}"><Row value="{{ item.clone() }}"></Row></ForEach></ul></li>"#,
    ] {
        let page = extract(&format!(
            r#"{STATE}<template rust:component=Counter><ul><ForEach items="{{{{ state.rows }}}}" key="{{{{ |item| item.id }}}}">{contents}</ForEach></ul></template>"#
        )).unwrap();
        let rust = tokens(&page.rust);
        assert!(
            !rust.contains(&tokens("::fusor_components::ForEach::item_row")),
            "{contents}"
        );
        assert!(
            rust.contains(&tokens("::fusor_components::ForEach::row")),
            "{contents}"
        );
    }
}

#[test]
fn foreach_index_proof_keeps_named_async_and_route_environments_complete() {
    for contents in [
        r#"<Async><Await value="{{ state.read }}" let="result"><ul><ForEach items="{{ result.rows }}" key="{{ |item| item.id }}"><Row value="{{ item.clone() }}"></Row></ForEach></ul></Await></Async>"#,
        r#"<Router><Route path="/:id" let="params"><ul><ForEach items="{{ state.rows }}" key="{{ |item| item.id }}"><Row value="{{ item.clone() }}"></Row></ForEach></ul></Route></Router>"#,
    ] {
        let page = extract(&format!(
            "{STATE}<template rust:component=Counter><main>{contents}</main></template>"
        ))
        .unwrap();
        let rust = tokens(&page.rust);
        assert!(
            !rust.contains(&tokens("::fusor_components::ForEach::item_row")),
            "{contents}"
        );
        assert!(
            rust.contains(&tokens("::fusor_components::ForEach::row")),
            "{contents}"
        );
    }
}

#[test]
fn foreach_empty_children_placeholder_keeps_forwarding_proof_narrow() {
    let error = extract(&format!(
        r#"{STATE}<template rust:component=Counter><ul><ForEach items="{{{{ state.rows }}}}" key="{{{{ |item| item.id }}}}"><li><Panel><template rust:content="body"><span>{{{{ index.get() }}}}</span></template></Panel></li></ForEach></ul></template>"#
    )).unwrap_err();
    assert!(
        error
            .message
            .contains("named content inside ForEach is not supported")
    );
    for children in ["", " \n\t"] {
        let page = extract(&format!(
            r#"{STATE}<template rust:component=Counter><ul><ForEach items="{{{{ state.rows }}}}" key="{{{{ |item| item.id }}}}"><Row value="{{{{ item.clone() }}}}">{children}</Row></ForEach></ul></template>"#
        )).unwrap();
        let rust = tokens(&page.rust);
        assert!(rust.contains(&tokens("::fusor_components::ForEach::item_row")));
        assert!(!rust.contains(&tokens("::fusor::dom::Children::new")));
    }
    for attribute in [r#"rust:if="state.visible""#, r#"rust:key="item.get().id""#] {
        let error = extract(&format!(
            r#"{STATE}<template rust:component=Counter><ul><ForEach items="{{{{ state.rows }}}}" key="{{{{ |item| item.id }}}}"><Row value="{{{{ item.clone() }}}}" {attribute}></Row></ForEach></ul></template>"#
        )).unwrap_err();
        assert!(error.message.contains("ForEach owns row identity"));
    }
}

#[test]
fn structural_control_flow_lowers_native_patterns_and_owned_fragments() {
    for render in ["", " rust:render=shared", " rust:render=server"] {
        let source = format!(
            r#"{STATE}<template rust:component=Counter{render}><main>
<If condition="{{{{ state.visible.get() }}}}"><h1>Yes</h1><span>Sibling</span><Else><p>No</p></Else></If>
<If condition="{{{{ false }}}}"></If>
<Match value="{{{{ state.value.get() }}}}">
<Case pattern="Some(user)"><p>{{{{ user.get().name }}}}</p><Details user="{{{{ user.clone() }}}}"></Details></Case>
<Case pattern="None"><p>Empty</p></Case>
</Match></main></template>"#
        );
        let page = extract(&source).unwrap();
        let rust = tokens(&page.rust);
        assert!(rust.contains("match"));
        assert!(rust.contains(&tokens("Some(user) =>")));
        assert!(!page.html.contains("<If"));
        assert!(!page.html.contains("<Match"));
        syn::parse_file(&page.rust).unwrap();
        if render != " rust:render=server" {
            assert!(rust.contains("branch_at"));
        }
        if !render.is_empty() {
            assert!(rust.contains("fusor:branch:0"));
        }
    }
}

// Growth checks need native token parsing and copy counts. A second recursive
// syn AST parse can exhaust the test-thread stack under workspace features;
// the real consumer fixtures check generated Rust syntax and types.
#[test]
fn nested_branches_do_not_multiply_emitted_child_bodies() {
    let mut body = "<span>{{ state.shared_leaf() }}</span>".to_owned();
    for depth in 1..=6 {
        body = format!("<If condition=\"{{{{ state.visible.get() }}}}\">{body}</If>");
        let page = extract(&format!(
            "{STATE}<template rust:component=Counter><main>{body}</main></template>"
        ))
        .unwrap();
        let rust = tokens(&page.rust);
        // Ordinary and coherent text operations remain distinct. Nesting a
        // constructor must not multiply either operation at every mode split.
        assert_eq!(
            rust.matches(&tokens("state.shared_leaf()")).count(),
            2,
            "branch depth {depth} duplicated the leaf"
        );
    }
}

#[test]
fn nested_lists_do_not_multiply_emitted_row_bodies() {
    let mut body = "<span>{{ state.shared_leaf() }}</span>".to_owned();
    for depth in 1..=6 {
        body = format!(
            "<ul><ForEach items=\"{{{{ state.rows() }}}}\" key=\"{{{{ |entry| entry.id }}}}\" item=\"entry{depth}\" index=\"position{depth}\"><li>{body}</li></ForEach></ul>"
        );
        let page = extract(&format!(
            "{STATE}<template rust:component=Counter><main>{body}</main></template>"
        ))
        .unwrap();
        // This native row also supports the ordinary binding-bundle path.
        assert_eq!(
            tokens(&page.rust)
                .matches(&tokens("state.shared_leaf()"))
                .count(),
            3,
            "list depth {depth} duplicated the row"
        );
    }
}

#[test]
fn nested_supplied_children_share_bodies_across_wrapper_mode_selection() {
    let mut body = "<span>{{ state.shared_leaf() }}</span>".to_owned();
    for depth in 1..=6 {
        body = format!(
            "<Wrapper><section><If condition=\"{{{{ state.visible.get() }}}}\">{body}</If></section></Wrapper>"
        );
        let page = extract(&format!(
            "{STATE}<template rust:component=Counter><main>{body}</main></template>\
             <template rust:component=Wrapper><section><Children></Children></section></template>"
        ))
        .unwrap();
        assert_eq!(
            tokens(&page.rust)
                .matches(&tokens("state.shared_leaf()"))
                .count(),
            2,
            "supplied-children depth {depth} duplicated the body"
        );
    }
}

#[test]
fn nested_awaits_emit_each_read_and_coherent_body_once() {
    let mut body = "<span>{{ state.shared_leaf() }}</span>".to_owned();
    for depth in 1..=6 {
        body = format!(
            "<Await value=\"{{{{ state.shared_read() }}}}\" let=\"result{depth}\"><section>{body}</section></Await>"
        );
        let page = extract(&format!(
            "{STATE}<template rust:component=Counter><main>{body}</main></template>"
        ))
        .unwrap();
        let rust = tokens(&page.rust);
        assert_eq!(rust.matches(&tokens("state.shared_read()")).count(), depth);
        assert_eq!(rust.matches(&tokens("state.shared_leaf()")).count(), 1);
    }
}

#[test]
fn structural_control_flow_rejects_ambiguous_structure_and_invalid_patterns() {
    for markup in [
        r#"<Else><p>Orphan</p></Else>"#,
        r#"<Case pattern="_"><p>Orphan</p></Case>"#,
        r#"<Match value="{{ state.value }}"><p>Not a Case</p></Match>"#,
        r#"<Match value="{{ state.value }}">text<Case pattern="_"></Case></Match>"#,
        r#"<Match value="{{ state.value }}"></Match>"#,
        r#"<If condition="{{ true }}"><Else></Else><Else></Else></If>"#,
        r#"<If condition="{{ true }}"><Else></Else><span>Late</span></If>"#,
        r#"<If condition="{{ true }}"><Else></Else>Late</If>"#,
        r#"<If condition="true"></If>"#,
        r#"<If condition="{{ true }}" class="invalid"></If>"#,
        r#"<If condition="{{ true }}" />"#,
        r#"<if condition="{{ true }}"></if>"#,
        r#"<If condition="{{ true }}"></if>"#,
        r#"<Match value="{{ state.value }}"><Case pattern="Some("></Case></Match>"#,
        r#"<Match value="{{ state.value }}"><Case pattern="Some(state)"></Case></Match>"#,
        r#"<Match value="{{ state.value }}"><Case pattern="Some(ref value)"></Case></Match>"#,
        r#"<Match value="{{ state.value }}"><Case pattern="Some(value)"><Match value="{{ value.get() }}"><Case pattern="Some(value)"></Case></Match></Case></Match>"#,
        r#"<table><If condition="{{ true }}"><tr><td>Unsafe HTML context</td></tr></If></table>"#,
    ] {
        assert!(
            extract(&format!(
                "{STATE}<template rust:component=Counter><main>{markup}</main></template>"
            ))
            .is_err(),
            "{markup}"
        );
    }
}

#[test]
fn exclusive_branches_can_each_place_children_but_not_duplicate_them() {
    let source = format!(
        r#"{STATE}<template rust:component=Counter><main><If condition="{{{{ true }}}}"><Children></Children><Else><Children></Children></Else></If></main></template>"#
    );
    extract(&source).unwrap();
    assert!(extract(&source.replace("<main>", "<main><Children></Children>")).is_err());
}

/// Extract `markup` inside a component and return the error's column and message.
/// `STATE` and the wrapper have no newlines, so the column is a byte position.
fn built_in_error(markup: &str) -> (usize, String) {
    let source = format!("{STATE}<main rust:component=Counter>{markup}</main>");
    let error = extract(&source).expect_err(markup);
    assert_eq!(error.line, 1, "{markup}: {error}");
    let base = source.find(markup).unwrap();
    (error.column - 1 - base, error.message)
}

#[test]
fn built_in_tag_attributes_name_the_tag_and_point_at_the_value() {
    let foreach = |attributes: &str| {
        format!(
            r#"<ul><ForEach items="{{{{ state.items }}}}" key="{{{{ |x| x.id }}}}"{attributes}><li></li></ForEach></ul>"#
        )
    };
    let route = |attributes: &str| format!(r#"<Router><Route {attributes}>Page</Route></Router>"#);
    let cases = [
        (r#"<If></If>"#.to_owned(), "<If", r#"If requires condition="{{ Rust expression }}""#),
        (r#"<If condition="true"></If>"#.into(), "true", "If condition requires exactly one {{ Rust expression }}"),
        (r#"<If condition="{{ true }}" class="x"></If>"#.into(), "<If", "If accepts only condition"),
        (r#"<If condition="{{ true }}" />"#.into(), "<If", "If, Else, Match and Case require exact spelling and explicit closing tags"),
        (r#"<If condition="{{ true }}"><Else hidden></Else></If>"#.into(), "<Else", "Else accepts no attributes"),
        (r#"<Match value="{{ 1 }}"><Case></Case></Match>"#.into(), "<Case", r#"Case requires pattern="Rust pattern""#),
        (foreach(r#" item="state""#), "state\"", "ForEach item cannot shadow framework scope names"),
        (foreach(r#" index="Position""#), "Position", "ForEach index must be a snake_case Rust identifier"),
        (foreach(r#" item="x" index="x""#), "<ForEach", "ForEach item and index names must differ"),
        (foreach(r#" rows="{{ 1 }}""#), "<ForEach", "ForEach accepts items, key, and optional item and index names"),
        (r#"<ul><foreach items="{{ 1 }}" key="{{ 1 }}"><li></li></foreach></ul>"#.into(), "<foreach", "the built-in component is spelled ForEach"),
        (r#"<Async><section><Await value="{{ state.read }}"><p></p></Await></section></Async>"#.into(), "<Await", r#"Await requires let="name" to name its resolved value"#),
        (r#"<Async><section><Await value="{{ state.read }}" let="ready"><p></p></Await></section></Async>"#.into(), "ready\"", "Await let cannot shadow framework scope names"),
        (r#"<Async><section><Await let="value"><p></p></Await></section></Async>"#.into(), "<Await", r#"Await requires value="{{ Rust expression }}""#),
        (r#"<Async boundary="state.view"><section></section></Async>"#.into(), "state.view", "Async boundary requires exactly one {{ Rust expression }}"),
        (route(r#"path="/" let="state""#), "state\"", "Route let cannot shadow framework scope names"),
        (route(r#"fallback path="/""#), "<Route ", "write <Route fallback> without path or let"),
        (route(r#"to="/""#), "<Route ", "Route accepts path and optional let, or fallback"),
    ];
    for (markup, at, message) in cases {
        let (column, actual) = built_in_error(&markup);
        assert_eq!(actual, message, "{markup}");
        assert_eq!(column, markup.find(at).unwrap(), "{markup}: {actual}");
    }
}

#[test]
fn app_state_is_one_expression_on_a_closed_tag() {
    for (markup, message) in [
        (
            r#"<App state="{{ Counter }}" />"#,
            "App requires an explicit closing tag",
        ),
        (
            r#"<App state="Counter"><main></main></App>"#,
            "App state requires exactly one {{ Rust expression }}",
        ),
        (
            r#"<App state="{{ Counter }}" class="x"><main></main></App>"#,
            r#"App accepts only state="{{ Rust expression }}"; put HTML attributes on its native root"#,
        ),
    ] {
        let error = extract(&format!("{STATE}{markup}")).expect_err(markup);
        assert_eq!(error.message, message, "{markup}");
    }
}

#[test]
fn comments_around_named_content_are_not_ordinary_children() {
    let content = r#"<template rust:content="body"><p>Body</p></template>"#;
    for markup in [
        format!("<Child><!-- the body -->{content}</Child>"),
        format!("<Child>\n  <!-- first -->\n  {content}\n  <!-- after -->\n</Child>"),
    ] {
        let page = extract(&format!(
            "{STATE}<main rust:component=Counter>{markup}</main>"
        ))
        .unwrap_or_else(|error| panic!("{markup}: {error}"));
        assert!(page.html.contains("<!--fusor:mount:0-->"), "{markup}");
    }
    let mixed =
        format!("{STATE}<main rust:component=Counter><Child><b>text</b>{content}</Child></main>");
    let error = extract(&mixed).unwrap_err();
    assert_eq!(
        error.message,
        "do not mix named content and ordinary children in one invocation"
    );
}

/// Prelude names the generated code must spell by path, because an application
/// module may shadow them (`enum Choice { Some, None }` with a glob import).
fn unqualified_prelude_names(tokens: proc_macro2::TokenStream, found: &mut Vec<String>) {
    let mut previous_colon = false;
    for token in tokens {
        match &token {
            proc_macro2::TokenTree::Group(group) => {
                unqualified_prelude_names(group.stream(), found)
            }
            proc_macro2::TokenTree::Ident(ident)
                if !previous_colon
                    && [
                        "Some", "None", "Ok", "Err", "Option", "Result", "String", "Vec",
                    ]
                    .contains(&ident.to_string().as_str()) =>
            {
                found.push(ident.to_string());
            }
            _ => {}
        }
        previous_colon = matches!(&token, proc_macro2::TokenTree::Punct(p) if p.as_char() == ':');
    }
}

#[test]
fn generated_code_names_prelude_items_by_path() {
    for markup in [
        // Server rendering: children, a branch and a keyed child.
        r#"<main rust:component="Counter" rust:render="server"><Panel><p>{{ state.count }}</p></Panel><If condition="{{ state.open }}"><p>open</p><Else><p>closed</p></Else></If></main>"#,
        // Coherent browser rendering: a keyed child inside an Async boundary.
        r#"<main rust:component="Counter"><Async><section><Await value="{{ state.read }}" let="result"><div><Child value="{{ result.clone() }}" rust:key="state.key.get()"></Child></div></Await></section></Async></main>"#,
        // Shared rendering emits both lowerings.
        r#"<main rust:component="Counter" rust:render="shared"><Panel><b>{{ state.count }}</b></Panel></main>"#,
    ] {
        let page = extract(&format!("{STATE}{markup}"))
            .unwrap_or_else(|error| panic!("{markup}: {error}"));
        let mut found = Vec::new();
        unqualified_prelude_names(page.rust.parse().unwrap(), &mut found);
        assert!(found.is_empty(), "{markup}: {found:?}");
    }
}

#[test]
fn hydrated_components_in_rows_keep_their_type_outside_lexical_aliases() {
    let page = extract(&format!(
        r#"{STATE}<main rust:component="Counter" rust:render="server"><ul><ForEach items="{{{{ state.items }}}}" key="{{{{ |item| item.id }}}}"><li><Cart hydrate="load" product_id="{{{{ item.get().id }}}}" title="Row"></Cart></li></ForEach></ul></main>"#
    ))
    .unwrap();
    let rust = tokens(&page.rust);
    assert!(page.rust.contains("prepare_island :: < Cart > ("), "{rust}");
    assert!(rust.contains(&tokens("<Cart as ::fusor_islands::Island> ::Props")));
    assert!(rust.contains(&tokens(r#"title: ::core::convert::Into::into("Row")"#)));
}

#[test]
fn template_hash_covers_named_content() {
    let hash = |text: &str| {
        let page = extract(&format!(
            r#"{STATE}<main rust:component="Counter"><Child><template rust:content="body"><p>{text}</p></template></Child></main>"#
        ))
        .unwrap();
        let start = page.rust.find("__FUSOR_TEMPLATE_HASH_0 : & str =").unwrap();
        page.rust[start..start + 110].to_owned()
    };
    assert_ne!(hash("first"), hash("second"));
}
