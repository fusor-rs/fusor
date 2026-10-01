//! First-party structural HTML components.
//!
//! `<App>` owns browser startup with inferred Rust state (`browser` feature).
//! `<Children>` places caller-supplied HTML without a wrapper or input field.
//! `<ForEach>` repeats authored HTML with stable keys. Its `item` and `index`
//! bindings are read-only reactive values; the collection remains application
//! state. The HTML compiler connects these types to the existing owned list
//! engine, including coherent publication and server rendering.
//!
//! [`BACKEND_VERSION`] versions the supported row/capture helpers used by
//! generated code for external renderers. They contain no browser dependencies.
use fusor::{Memo, Signal, memo};

/// Version of the backend row/capture protocol: [`Entry`], [`Value`],
/// [`RowValue`], [`Row`], [`ItemRow`], [`Captured`], and [`ForEach`]'s `entries`,
/// `values`, `key`, `value_key`, `row`, and `item_row` helpers. Browser mounting
/// and server-only helpers are separate contracts.
pub const BACKEND_VERSION: u32 = 1;

/// The built-in application boundary. Its state is ordinary inferred Rust.
pub struct App;

#[cfg(feature = "browser")]
impl App {
    /// Compiler entry point: retain the prepared root before activating work.
    #[doc(hidden)]
    pub fn mount(
        prepare: impl FnOnce() -> Result<fusor::dom::Scope, fusor::dom::JsValue>,
    ) -> Result<(), fusor::dom::JsValue> {
        fusor::dom::application::mount_scope(prepare)
    }
}

/// The built-in child-content placement. In HTML, `<Children></Children>`
/// renders the receiving component's nested HTML, without a wrapper or Rust input.
/// Each component may place it once. The compiler supplies its runtime factory.
pub struct Children;

/// Coordinates all participating Await reads under one native HTML root.
/// The compiler owns an automatic boundary unless `boundary` is supplied.
pub struct Async;

/// Displays a declared async read and names its successful value with `let`.
/// Updates independently outside Async, or joins the enclosing boundary.
pub struct Await;

/// Selects owned Route children. Requires fusor-router's browser feature.
/// Nested routers inherit the nearest matched prefix through component ownership.
pub struct Router;

/// One lazy HTML branch, with path captures or a sibling fallback.
pub struct Route;

/// The built-in keyed iteration component.
pub struct ForEach;

/// Mounts inline HTML while a boolean condition is true, with an optional Else.
pub struct If;
/// The final, exclusive fallback child of If.
pub struct Else;
/// Selects one inline HTML branch using an exhaustive native Rust match.
pub struct Match;
/// A Rust pattern and its lazy HTML. Captures are read-only reactive Memo values.
pub struct Case;

/// One collection value and its current position. Compiler/runtime protocol.
#[derive(Clone, PartialEq)]
pub struct Entry<T> {
    pub value: T,
    pub position: usize,
}

/// One collection value of a row proven never to read its index. Without a
/// position, moving the row leaves its source unchanged. Compiler/runtime protocol.
#[derive(Clone, PartialEq)]
pub struct Value<T>(pub T);

/// The collection value a row source carries. Compiler/runtime protocol.
pub trait RowValue {
    type Item;
    fn item(&self) -> &Self::Item;
}

impl<T> RowValue for Entry<T> {
    type Item = T;
    fn item(&self) -> &T {
        &self.value
    }
}

impl<T> RowValue for Value<T> {
    type Item = T;
    fn item(&self) -> &T {
        &self.0
    }
}

/// Lexical environment of an inline row. `parent` preserves the caller's scope.
pub struct Row<P, T> {
    pub parent: P,
    pub item: Memo<T>,
    pub index: Memo<usize>,
}

/// Compiler environment for a forwarding row proven not to expose its index.
/// Keep the same item Memo API without constructing an unreachable projection.
pub struct ItemRow<P, T> {
    pub parent: P,
    pub item: Memo<T>,
}

impl ForEach {
    /// Evaluate an authored key against an entry's collection value.
    pub fn key<T, K>(entry: &Entry<T>, key: impl FnOnce(&T) -> K) -> K {
        key(&entry.value)
    }

    /// Attach positions to values without using positions as reconciliation keys.
    pub fn entries<T>(items: Vec<T>) -> Vec<Entry<T>> {
        items
            .into_iter()
            .enumerate()
            .map(|(position, value)| Entry { value, position })
            .collect()
    }

    /// Values for rows proven never to read their index: a move is not an input.
    pub fn values<T>(items: Vec<T>) -> Vec<Value<T>> {
        items.into_iter().map(Value).collect()
    }

    /// Evaluate an authored key against a row that does not read its index.
    pub fn value_key<T, K>(value: &Value<T>, key: impl FnOnce(&T) -> K) -> K {
        key(&value.0)
    }

    /// Native compiler path for an entry whose collection snapshot cannot change.
    /// Keep the ordinary lazy Memo API while avoiding an unexposed source signal.
    #[cfg(not(target_arch = "wasm32"))]
    #[doc(hidden)]
    pub fn server_row<P, T: Clone + PartialEq + 'static>(parent: P, entry: Entry<T>) -> Row<P, T> {
        let Entry { value, position } = entry;
        Row {
            parent,
            item: memo(move || value.clone()),
            index: memo(move || position),
        }
    }

    /// Native counterpart of a proven item-only forwarding row.
    #[cfg(not(target_arch = "wasm32"))]
    #[doc(hidden)]
    pub fn server_item_row<P, T: Clone + PartialEq + 'static>(
        parent: P,
        entry: Entry<T>,
    ) -> ItemRow<P, T> {
        let Entry { value, .. } = entry;
        ItemRow {
            parent,
            item: memo(move || value.clone()),
        }
    }

    /// Live item projection with ordinary Memo caching and equality suppression.
    /// Update the source signal to publish a row change. A direct
    /// [`Signal::with_render_value`] override does not invalidate a cached Memo.
    pub fn item_row<P, E: RowValue + 'static>(parent: P, source: Signal<E>) -> ItemRow<P, E::Item>
    where
        E::Item: Clone + PartialEq + 'static,
    {
        ItemRow {
            parent,
            item: memo(move || source.with(|entry| entry.item().clone())),
        }
    }

    /// Project item and position from one atomic list update. Update the source
    /// signal to publish the new row inputs; the projections suppress equal
    /// values independently. A direct [`Signal::with_render_value`] override
    /// does not invalidate their cached Memos.
    pub fn row<P, T: Clone + PartialEq + 'static>(
        parent: P,
        source: Signal<Entry<T>>,
    ) -> Row<P, T> {
        let value = source.clone();
        Row {
            parent,
            item: memo(move || value.with(|entry| entry.value.clone())),
            index: memo(move || source.with(|entry| entry.position)),
        }
    }
}

/// Retained branch/Await snapshot. Pointer identity avoids requiring
/// application response types to implement PartialEq or cloning their contents.
pub struct Captured<T>(std::rc::Rc<T>);
impl<T> Clone for Captured<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}
impl<T> PartialEq for Captured<T> {
    fn eq(&self, other: &Self) -> bool {
        std::rc::Rc::ptr_eq(&self.0, &other.0)
    }
}
impl<T> Captured<T> {
    pub fn new(value: std::rc::Rc<T>) -> Self {
        Self(value)
    }
    pub fn get(&self) -> std::rc::Rc<T> {
        self.0.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fusor::{effect, signal};
    use std::{cell::Cell, rc::Rc};

    #[test]
    fn item_and_position_are_separate_reactive_projections() {
        let source = signal(Entry {
            value: "Ada",
            position: 0,
        });
        let row = ForEach::row((), source.clone());
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        let item = row.item.clone();
        let _effect = effect(move || {
            let _ = item.get();
            observed.set(observed.get() + 1);
        });
        source.set(Entry {
            value: "Ada",
            position: 3,
        });
        assert_eq!(row.index.get(), 3);
        assert_eq!(row.item.get(), "Ada");
        assert_eq!(
            calls.get(),
            1,
            "moving a row must not invalidate equal item projections"
        );
        source.set(Entry {
            value: "Grace",
            position: 3,
        });
        assert_eq!(row.item.get(), "Grace");
        assert_eq!(calls.get(), 2);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn server_row_keeps_item_projection_lazy_and_cached() {
        struct Item {
            value: u32,
            clones: Rc<Cell<usize>>,
        }
        impl Clone for Item {
            fn clone(&self) -> Self {
                self.clones.set(self.clones.get() + 1);
                Self {
                    value: self.value,
                    clones: self.clones.clone(),
                }
            }
        }
        impl PartialEq for Item {
            fn eq(&self, other: &Self) -> bool {
                self.value == other.value
            }
        }
        let clones = Rc::new(Cell::new(0));
        let row = ForEach::server_row(
            "parent",
            Entry {
                value: Item {
                    value: 7,
                    clones: clones.clone(),
                },
                position: 3,
            },
        );
        assert_eq!(row.parent, "parent");
        assert_eq!(
            clones.get(),
            0,
            "constructing a row must not evaluate its item"
        );
        assert_eq!(row.index.get(), 3);
        assert_eq!(
            clones.get(),
            0,
            "reading the index must not evaluate its item"
        );
        assert_eq!(row.item.with(|item| item.value), 7);
        assert_eq!(clones.get(), 1);
        assert_eq!(row.item.with(|item| item.value), 7);
        assert_eq!(clones.get(), 1, "repeated borrows must use the memo cache");
        let shared = row.item.clone();
        assert_eq!(clones.get(), 1, "cloning a memo must share its cache");
        assert_eq!(shared.get().value, 7);
        assert_eq!(
            clones.get(),
            2,
            "get clones the cached value for its caller"
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn server_row_preserves_nested_signal_identity_and_tracking() {
        #[derive(Clone)]
        struct Item {
            id: u32,
            value: Signal<u32>,
        }
        impl PartialEq for Item {
            fn eq(&self, other: &Self) -> bool {
                self.id == other.id
            }
        }
        let source = signal(1_u32);
        let row = ForEach::server_row(
            (),
            Entry {
                value: Item {
                    id: 7,
                    value: source.clone(),
                },
                position: 3,
            },
        );
        let observed = Rc::new(std::cell::RefCell::new(Vec::new()));
        let values = observed.clone();
        let item = row.item.clone();
        let subscription = effect(move || {
            let value = item.with(|item| item.value.get());
            values.borrow_mut().push(value);
        });
        assert_eq!(*observed.borrow(), [1]);
        source.set(2);
        assert_eq!(*observed.borrow(), [1, 2]);
        assert_eq!(row.item.with(|item| item.value.get()), 2);
        assert_eq!(row.index.get(), 3);
        drop(subscription);
        source.set(3);
        assert_eq!(*observed.borrow(), [1, 2]);
    }

    #[test]
    fn item_only_rows_keep_item_projection_lazy_and_shared() {
        struct Item {
            value: u32,
            clones: Rc<Cell<usize>>,
        }
        impl Clone for Item {
            fn clone(&self) -> Self {
                self.clones.set(self.clones.get() + 1);
                Self {
                    value: self.value,
                    clones: self.clones.clone(),
                }
            }
        }
        impl PartialEq for Item {
            fn eq(&self, other: &Self) -> bool {
                self.value == other.value
            }
        }
        fn verify(make: impl FnOnce(Entry<Item>) -> ItemRow<&'static str, Item>) {
            let clones = Rc::new(Cell::new(0));
            let row = make(Entry {
                value: Item {
                    value: 7,
                    clones: clones.clone(),
                },
                position: 9,
            });
            assert_eq!(row.parent, "parent");
            assert_eq!(clones.get(), 0, "row construction must not read the item");
            assert_eq!(row.item.with(|item| item.value), 7);
            assert_eq!(clones.get(), 1);
            let shared = row.item.clone();
            assert_eq!(shared.with(|item| item.value), 7);
            assert_eq!(clones.get(), 1, "the ordinary Memo cache must be shared");
            assert_eq!(shared.get().value, 7);
            assert_eq!(
                clones.get(),
                2,
                "get clones the cached value for its caller"
            );
        }
        verify(|entry| ForEach::item_row("parent", signal(entry)));
        #[cfg(not(target_arch = "wasm32"))]
        verify(|entry| ForEach::server_item_row("parent", entry));
    }

    #[test]
    fn item_only_row_suppresses_moves_and_preserves_nested_signal_tracking() {
        #[derive(Clone)]
        struct Item {
            id: u32,
            value: Signal<u32>,
        }
        impl PartialEq for Item {
            fn eq(&self, other: &Self) -> bool {
                self.id == other.id
            }
        }
        let nested = signal(1_u32);
        let source = signal(Entry {
            value: Item {
                id: 7,
                value: nested.clone(),
            },
            position: 0,
        });
        let row = ForEach::item_row((), source.clone());
        let values = Rc::new(std::cell::RefCell::new(Vec::new()));
        let observed = values.clone();
        let item = row.item.clone();
        let subscription = effect(move || {
            let value = item.with(|item| item.value.get());
            observed.borrow_mut().push(value);
        });
        source.update(|entry| entry.position = 3);
        assert_eq!(
            *values.borrow(),
            [1],
            "an equal item's move must not republish it"
        );
        nested.set(2);
        assert_eq!(*values.borrow(), [1, 2]);
        source.set(Entry {
            value: Item {
                id: 8,
                value: signal(3),
            },
            position: 3,
        });
        assert_eq!(*values.borrow(), [1, 2, 3]);
        drop(subscription);
        nested.set(4);
        assert_eq!(*values.borrow(), [1, 2, 3]);
    }

    #[test]
    fn value_rows_republish_only_changed_values_and_keep_nested_tracking() {
        #[derive(Clone)]
        struct Item {
            id: u32,
            value: Signal<u32>,
        }
        impl PartialEq for Item {
            fn eq(&self, other: &Self) -> bool {
                self.id == other.id
            }
        }
        let nested = signal(1_u32);
        let items = vec![
            Item {
                id: 7,
                value: nested.clone(),
            },
            Item {
                id: 9,
                value: signal(5),
            },
        ];
        let values = ForEach::values(items.clone());
        assert_eq!(values.len(), 2);
        assert_eq!(ForEach::value_key(&values[1], |item| item.id), 9);
        let source = signal(values[0].clone());
        let row = ForEach::item_row((), source.clone());
        let observed = Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = observed.clone();
        let item = row.item.clone();
        let subscription = effect(move || {
            let value = item.with(|item| item.value.get());
            seen.borrow_mut().push(value);
        });
        source.set(Value(items[0].clone()));
        assert_eq!(*observed.borrow(), [1], "an equal value must not republish");
        nested.set(2);
        assert_eq!(*observed.borrow(), [1, 2]);
        source.set(Value(items[1].clone()));
        assert_eq!(*observed.borrow(), [1, 2, 5]);
        drop(subscription);
        nested.set(4);
        assert_eq!(*observed.borrow(), [1, 2, 5]);
    }

    #[test]
    fn item_only_rows_release_source_and_cached_payload_with_last_memo() {
        #[derive(Clone)]
        struct Payload {
            lifetime: Rc<()>,
            drops: Rc<Cell<usize>>,
        }
        impl PartialEq for Payload {
            fn eq(&self, other: &Self) -> bool {
                Rc::ptr_eq(&self.lifetime, &other.lifetime)
            }
        }
        impl Drop for Payload {
            fn drop(&mut self) {
                self.drops.set(self.drops.get() + 1);
            }
        }
        fn verify(read: bool, make: impl FnOnce(Entry<Payload>) -> ItemRow<(), Payload>) {
            let lifetime = Rc::new(());
            let weak = Rc::downgrade(&lifetime);
            let drops = Rc::new(Cell::new(0));
            let row = make(Entry {
                value: Payload {
                    lifetime,
                    drops: drops.clone(),
                },
                position: 0,
            });
            if read {
                row.item.with(|_| ());
            }
            let forwarded = row.item.clone();
            drop(row);
            assert!(
                weak.upgrade().is_some(),
                "forwarded Memo owns its source and cached value"
            );
            assert_eq!(drops.get(), 0);
            drop(forwarded);
            assert!(weak.upgrade().is_none());
            assert_eq!(
                drops.get(),
                if read { 2 } else { 1 },
                "source and cached payload must both be released"
            );
        }
        for read in [false, true] {
            verify(read, |entry| ForEach::item_row((), signal(entry)));
            #[cfg(not(target_arch = "wasm32"))]
            verify(read, |entry| ForEach::server_item_row((), entry));
        }
    }
}
