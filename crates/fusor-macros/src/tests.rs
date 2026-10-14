use super::*;

#[test]
fn rejects_ambiguous_or_incomplete_field_contracts() {
    for (source, message) in [
        ("struct Missing { value: u32 }", "field needs #[input]"),
        (
            "struct Both { #[input] #[local(init = 0)] value: u32 }",
            "choose exactly one",
        ),
        (
            "struct Twice { #[input] #[input] value: u32 }",
            "choose exactly one",
        ),
        (
            "struct Local { #[local()] value: u32 }",
            "local state requires",
        ),
        (
            "struct Bad { #[local(default)] value: u32 }",
            "expected `init = expression`",
        ),
        (
            "struct Bad { #[local(init = 1, init = 2)] value: u32 }",
            "duplicate local initializer",
        ),
        (
            "struct Bad { #[input(other)] value: u32 }",
            "expected `default`, `default = expression` or `required`",
        ),
        (
            "struct Bad { #[input(default, default = 1)] value: u32 }",
            "choose one of",
        ),
        (
            "struct Bad { #[input(required, default)] value: Option<u32> }",
            "choose one of",
        ),
        (
            "struct Bad { #[input()] value: u32 }",
            "expected #[input(default)]",
        ),
        (
            "struct Bad { #[input] build: u32 }",
            "`build` finishes the generated input builder",
        ),
        ("struct Bad(u32);", "requires named fields"),
        ("enum Bad { A }", "only be derived for a struct"),
        (
            "struct Generic<T> { #[input] value: T }",
            "implement FromInputs manually",
        ),
        ("#[input] struct Bad;", "on a field"),
        (
            "#[from_inputs(wrong = name)] struct Bad;",
            "expected `crate = path`",
        ),
        (
            "#[from_inputs(crate = ::one, crate = ::two)] struct Bad;",
            "duplicate crate path",
        ),
        ("#[from_inputs()] struct Bad;", "expected #[from_inputs"),
        (
            "struct Bad { #[from_inputs(crate = ::rf)] #[input] value: u32 }",
            "belongs on the struct",
        ),
    ] {
        let error = expand(syn::parse_str(source).unwrap())
            .unwrap_err()
            .to_string();
        assert!(error.contains(message), "{source}: {error}");
    }
}

#[test]
fn collects_errors_for_multiple_unmarked_fields() {
    let errors = expand(parse_quote!(
        struct Bad {
            one: u32,
            two: bool,
        }
    ))
    .unwrap_err()
    .into_iter()
    .count();
    assert_eq!(errors, 2);
}

#[test]
fn expansion_is_valid_rust_for_all_supported_shapes() {
    for source in [
        "struct Empty;",
        "struct Empty {}",
        "struct Local { #[local(init = Vec::new())] values: Vec<u32> }",
        "pub struct Public { #[input] value: u32 }",
        "pub struct Optional { #[input] a: u32, #[input(default)] b: u32, #[input(default = 3)] c: u32 }",
        "struct Options { #[input] a: Option<u32>, #[input(required)] b: core::option::Option<u32> }",
        "#[from_inputs(crate = ::renamed)] pub(crate) struct Named { #[input] r#type: String, #[local(init = 0)] count: u32 }",
    ] {
        let expanded = expand(syn::parse_str(source).unwrap()).unwrap();
        syn::parse2::<syn::File>(expanded).unwrap();
    }
}

#[test]
fn hand_written_inputs_expand_for_generic_structs() {
    for source in [
        "struct Unit;",
        "pub struct Plain { pub value: u32, #[input(default)] pub open: bool }",
        "struct Generic<'a, T: Clone = u32, const N: usize = 2> where T: 'a { items: &'a [T; N] }",
        "#[inputs(crate = ::renamed)] struct Named { r#type: String }",
    ] {
        let expanded = inputs::expand(syn::parse_str(source).unwrap()).unwrap();
        syn::parse2::<syn::File>(expanded).unwrap();
    }
}

#[test]
fn hand_written_inputs_reject_invalid_markers() {
    for (source, message) in [
        (
            "struct Bad { #[input] #[input] value: u32 }",
            "one #[input(...)]",
        ),
        (
            "struct Bad { #[inputs(crate = ::rf)] value: u32 }",
            "belongs on the struct",
        ),
        ("#[input] struct Bad;", "on a field"),
        ("struct Bad { build: u32 }", "`build` finishes"),
        ("struct Bad(u32);", "requires named fields"),
    ] {
        let error = inputs::expand(syn::parse_str(source).unwrap())
            .unwrap_err()
            .to_string();
        assert!(error.contains(message), "{source}: {error}");
    }
}

#[test]
fn javascript_inputs_are_explicit_and_bounded() {
    let source = r#"struct Scene {
        #[input] #[js] speed: fusor::Signal<f64>,
        #[js] points: Signal<Vec<Option<i32>>>,
        #[js] object: Signal<wasm_bindgen::JsValue>,
        private: Signal<String>,
    }"#;
    let output = expand_js_inputs(syn::parse_str(source).unwrap()).unwrap();
    let text = output.to_string();
    syn::parse2::<syn::File>(output).unwrap();
    assert!(text.contains("\"speed\""));
    assert!(text.contains("\"points\""));
    assert!(text.contains("\"object\""));
    assert!(!text.contains("private"));
    for source in [
        "struct Bad { #[js] field: Signal<i64> }",
        "struct Bad { #[js] field: Signal<u64> }",
        "struct Bad { #[js] field: Signal<MyStruct> }",
        "struct Bad { #[js] field: Signal<Vec<f32>> }",
        "struct Bad { #[js] field: f64 }",
        "struct Bad { #[js] field: Memo<f64> }",
    ] {
        let error = expand_js_inputs(syn::parse_str(source).unwrap())
            .unwrap_err()
            .to_string();
        assert!(
            error.contains("#[js] requires Signal<T>"),
            "{source}: {error}"
        );
    }
}

#[test]
fn javascript_inputs_reject_invalid_shapes_and_markers() {
    for (source, message) in [
        ("struct Bad(#[js] Signal<f64>);", "requires named fields"),
        (
            "enum Bad { A }",
            "JsInputs can only be derived for a struct",
        ),
        (
            "struct Bad<T> { #[js] field: Signal<T> }",
            "concrete structs",
        ),
        (
            "struct Bad { #[js(rename=\"x\")] field: Signal<f64> }",
            "single #[js] marker",
        ),
    ] {
        let error = expand_js_inputs(syn::parse_str(source).unwrap())
            .unwrap_err()
            .to_string();
        assert!(error.contains(message), "{source}: {error}");
    }
}
