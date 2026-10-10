use fusor_router::{AppUrl, BasePath, Route, encode_query, encode_segment};

#[derive(Clone, Debug, PartialEq)]
enum Page {
    Home,
    File(String),
}
impl Route for Page {
    fn parse(url: &AppUrl) -> Option<Self> {
        match url.segments().ok()?.as_slice() {
            [root] if root.is_empty() => Some(Self::Home),
            [files, name] if files == "files" => Some(Self::File(name.clone())),
            _ => None,
        }
    }
    fn path(&self) -> String {
        match self {
            Self::Home => "/".into(),
            Self::File(name) => format!("/files/{}", encode_segment(name)),
        }
    }
}
#[test]
fn typed_routes_roundtrip_with_unicode_slashes_and_percent_signs() {
    let base = BasePath::new("/tools/files/").unwrap();
    for route in [
        Page::Home,
        Page::File("café/a?b#c%2f".into()),
        Page::File("document.txt".into()),
    ] {
        let href = base.href(&route).unwrap();
        let parsed = base.strip(&href).unwrap();
        assert_eq!(Page::parse(&parsed), Some(route));
    }
    assert!(base.strip("/tools/files-sibling/files/x").is_err());
    assert!(base.strip("/tools/files").is_err());
}
#[test]
fn queries_retain_repeated_values_and_fragment_is_separate() {
    let query = encode_query([("q", "café + a/b"), ("tag", "one"), ("tag", "two")]);
    let url = AppUrl::parse(&format!("/files/report?{query}#part-2")).unwrap();
    assert_eq!(url.query_first("q").unwrap(), "café + a/b");
    assert_eq!(
        url.query_pairs().collect::<Vec<_>>(),
        [
            ("q".into(), "café + a/b".into()),
            ("tag".into(), "one".into()),
            ("tag".into(), "two".into())
        ]
    );
    assert_eq!(url.fragment(), "part-2");
    assert_eq!(AppUrl::parse(&url.to_string()).unwrap(), url);
}
#[test]
fn invalid_paths_and_route_formatters_fail_explicitly() {
    for value in [
        "https://evil.test",
        "//evil.test",
        "relative",
        "/../x",
        "/%2E%2e/x",
        "/a\\b",
        "/a\n",
        "/%ff",
        "/%x",
        "/%",
    ] {
        assert!(AppUrl::parse(value).is_err(), "{value}");
    }
    assert!(BasePath::new("/bad//base/").is_err());
    #[derive(Clone, PartialEq)]
    struct Broken;
    impl Route for Broken {
        fn parse(_: &AppUrl) -> Option<Self> {
            None
        }
        fn path(&self) -> String {
            "/lost".into()
        }
    }
    assert!(BasePath::new("/").unwrap().href(&Broken).is_err());
}
