use fusor::{ContextError, ContextKey, Owner, Signal, effect, signal};
use std::{cell::Cell, rc::Rc};

struct Locale;
impl ContextKey for Locale {
    type Value = Signal<String>;
}
struct OtherLocale;
impl ContextKey for OtherLocale {
    type Value = Signal<String>;
}

#[test]
fn nearest_provider_keys_and_roots_are_isolated() {
    let root = Owner::new();
    root.handle()
        .provide::<Locale>(signal("en".into()))
        .unwrap();
    root.handle()
        .provide::<OtherLocale>(signal("fr".into()))
        .unwrap();
    let child = Owner::child(&root.handle());
    let sibling = Owner::child(&root.handle());
    assert_eq!(child.handle().context::<Locale>().unwrap().get(), "en");
    child
        .handle()
        .provide::<Locale>(signal("es".into()))
        .unwrap();
    let leaf = Owner::child(&child.handle());
    assert_eq!(leaf.handle().context::<Locale>().unwrap().get(), "es");
    assert_eq!(leaf.handle().context::<OtherLocale>().unwrap().get(), "fr");
    assert_eq!(sibling.handle().context::<Locale>().unwrap().get(), "en");
    assert!(Owner::new().handle().context::<Locale>().is_none());
    assert_eq!(
        root.handle().provide::<Locale>(signal("de".into())),
        Err(ContextError::AlreadyProvided)
    );
    assert_eq!(root.handle().context::<Locale>().unwrap().get(), "en");
}

#[test]
fn supplied_signals_are_reactive_but_provider_lookup_is_not() {
    let root = Owner::new();
    let locale = signal("en".into());
    root.handle().provide::<Locale>(locale.clone()).unwrap();
    let child = Owner::child(&root.handle());
    let reads = Rc::new(Cell::new(0));
    let observed = reads.clone();
    let owner = child.handle();
    let _effect = effect(move || {
        owner.context::<Locale>().unwrap().get();
        observed.set(observed.get() + 1);
    });
    locale.set("fr".into());
    assert_eq!(reads.get(), 2);
    child
        .handle()
        .provide::<Locale>(signal("es".into()))
        .unwrap();
    assert_eq!(reads.get(), 2);
    // Shadowing does not resubscribe existing readers until their next run.
    locale.set("de".into());
    assert_eq!(reads.get(), 3);
    locale.set("it".into());
    assert_eq!(reads.get(), 3);
}

#[test]
fn disposal_releases_context_without_invalidating_already_obtained_rust_values() {
    struct Service;
    struct Key;
    impl ContextKey for Key {
        type Value = Service;
    }
    let root = Owner::new();
    let child = Owner::child(&root.handle());
    root.handle().provide::<Key>(Service).unwrap();
    let value = child.handle().context::<Key>().unwrap();
    let weak = Rc::downgrade(&value);
    root.dispose();
    assert!(root.handle().context::<Key>().is_none());
    assert!(child.handle().context::<Key>().is_none());
    assert_eq!(
        root.handle().provide::<Key>(Service),
        Err(ContextError::Disposed)
    );
    assert!(weak.upgrade().is_some());
    drop(value);
    assert!(weak.upgrade().is_none());
}

#[test]
fn provider_destructors_run_outside_borrows_after_child_cleanup() {
    struct Probe(Box<dyn Fn()>);
    impl Drop for Probe {
        fn drop(&mut self) {
            (self.0)();
        }
    }
    struct Key;
    impl ContextKey for Key {
        type Value = Probe;
    }
    let root = Owner::new();
    let child = Owner::child(&root.handle());
    let cleaned = Rc::new(Cell::new(false));
    let captured = cleaned.clone();
    let _cleanup = child.handle().on_cleanup(move || captured.set(true));
    let owner = root.handle();
    let descendant = child.handle();
    let drops = Rc::new(Cell::new(0));
    let captured = drops.clone();
    root.handle()
        .provide::<Key>(Probe(Box::new(move || {
            captured.set(captured.get() + 1);
            assert!(cleaned.get());
            assert!(descendant.is_disposed());
            assert!(owner.context::<Key>().is_none());
            assert_eq!(
                owner.provide::<Locale>(signal("en".into())),
                Err(ContextError::Disposed)
            );
        })))
        .unwrap();
    let owner = root.handle();
    let captured = drops.clone();
    assert_eq!(
        root.handle().provide::<Key>(Probe(Box::new(move || {
            captured.set(captured.get() + 1);
            assert!(owner.context::<Key>().is_some());
        }))),
        Err(ContextError::AlreadyProvided)
    );
    assert_eq!(drops.get(), 1);
    root.dispose();
    assert_eq!(drops.get(), 2);
}
