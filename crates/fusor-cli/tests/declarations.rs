//! Declaration ownership survives failed checks and never extends to authored files.
mod support;

use std::fs;
use support::{Fixture, failure, success};

#[test]
fn declaration_cleanup_preserves_ownership_until_it_succeeds() {
    let fixture = Fixture::new();
    let app = fixture.scaffold("declarations");
    fixture.lock(&app);
    let directory = app.join(".fusor/types");
    fs::create_dir_all(&directory).unwrap();
    let index = directory.join(".generated.json");
    let stale = directory.join("stale.d.ts");
    let authored = directory.join("authored.d.ts");
    fs::write(&stale, "old generated declarations").unwrap();
    fs::write(&authored, "authored declarations").unwrap();

    for contents in ["{broken", "{}", r#"["../authored.d.ts"]"#] {
        fs::write(&index, contents).unwrap();
        let error = failure(&mut fixture.cli(&app, &["check", "--locked"]));
        assert!(error.contains(".generated.json"), "{error}");
        assert_eq!(fs::read_to_string(&index).unwrap(), contents);
        assert_eq!(
            fs::read_to_string(&stale).unwrap(),
            "old generated declarations"
        );
    }

    fs::remove_file(&index).unwrap();
    fs::create_dir(&index).unwrap();
    let error = failure(&mut fixture.cli(&app, &["check", "--locked"]));
    assert!(error.contains(".generated.json"), "{error}");
    fs::remove_dir(&index).unwrap();

    let previous = r#"["stale.d.ts", "already-removed.d.ts"]"#;
    fs::write(&index, previous).unwrap();
    fs::remove_file(&stale).unwrap();
    fs::create_dir(&stale).unwrap();
    let error = failure(&mut fixture.cli(&app, &["check", "--locked"]));
    assert!(error.contains("stale.d.ts"), "{error}");
    assert_eq!(fs::read_to_string(&index).unwrap(), previous);

    fs::remove_dir(&stale).unwrap();
    fs::write(&stale, "old generated declarations").unwrap();
    success(&mut fixture.cli(&app, &["check", "--locked"]));
    assert!(!stale.exists());
    assert_eq!(fs::read_to_string(&index).unwrap(), "[]");
    assert_eq!(
        fs::read_to_string(&authored).unwrap(),
        "authored declarations"
    );
}
