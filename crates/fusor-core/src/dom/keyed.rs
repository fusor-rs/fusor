use super::{
    ElementTarget, JsValue, Scope, document, reconcile, remove_tree, strings, with_native_root,
};
use crate::{Signal, signal, untrack};
use std::collections::BTreeMap;
use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlElement, HtmlInputElement};

type EncodeKey<K> = dyn Fn(&K) -> Result<String, JsValue>;

struct Row<T> {
    // Held until the row drops, like the row's scope, even when the scope does
    // not retain its item.
    _state: Signal<T>,
    scope: Scope,
}

impl Scope {
    /// Reconcile a list by stable keys, retaining nodes, focus, and row scopes.
    /// `render` runs once per inserted key. The row signal delivers later values.
    /// This binding owns the container's children. Duplicate keys are errors.
    pub fn keyed<T, K>(
        &mut self,
        target: impl ElementTarget,
        items: impl Fn() -> Vec<T> + 'static,
        key: impl Fn(&T) -> K + 'static,
        render: impl Fn(Signal<T>) -> Result<Scope, JsValue> + 'static,
    ) -> Result<(), JsValue>
    where
        T: Clone + PartialEq + 'static,
        K: Ord + Clone + 'static,
    {
        self.keyed_inner(target, items, key, render, None)
    }

    /// Generated shared templates carry serialized row identities in native HTML.
    #[doc(hidden)]
    pub fn keyed_hydrated<T, K>(
        &mut self,
        target: impl ElementTarget,
        items: impl Fn() -> Vec<T> + 'static,
        key: impl Fn(&T) -> K + 'static,
        render: impl Fn(Signal<T>) -> Result<Scope, JsValue> + 'static,
        encode: impl Fn(&K) -> Result<String, JsValue> + 'static,
    ) -> Result<(), JsValue>
    where
        T: Clone + PartialEq + 'static,
        K: Ord + Clone + 'static,
    {
        self.keyed_inner(target, items, key, render, Some(Box::new(encode)))
    }

    fn keyed_inner<T, K>(
        &mut self,
        target: impl ElementTarget,
        items: impl Fn() -> Vec<T> + 'static,
        key: impl Fn(&T) -> K + 'static,
        render: impl Fn(Signal<T>) -> Result<Scope, JsValue> + 'static,
        encode: Option<Box<EncodeKey<K>>>,
    ) -> Result<(), JsValue>
    where
        T: Clone + PartialEq + 'static,
        K: Ord + Clone + 'static,
    {
        let hydrating = self.is_hydrating();
        let container = target.resolve(self)?;
        // `rows` owns the rows and orders their lifecycle by key. `order` and
        // `states` hold the same keys in DOM order with each row's signal, so
        // rows that keep their index are updated without a map lookup.
        let mut rows: BTreeMap<K, Row<T>> = BTreeMap::new();
        let mut order: Vec<K> = Vec::new();
        let mut states: Vec<Signal<T>> = Vec::new();
        let mut initialized = false;
        self.bind(move || {
            let items = items();
            untrack(|| {
                let keys: Vec<K> = items.iter().map(&key).collect();
                let mut unique = reconcile::SortedKeys::new(&keys)
                    .ok_or_else(|| JsValue::from_str("fusor: duplicate key in list"))?;
                let native_rows = if hydrating && !initialized {
                    let encode = encode.as_ref().ok_or_else(|| {
                        JsValue::from_str("hydrated lists require generated key metadata")
                    })?;
                    server_rows(&container, &keys, encode)?
                } else {
                    Vec::new()
                };
                let document = document()?;
                let focused = focused(&document, &container);
                let retained = reconcile::previous_positions(&order, &unique);
                // Stage new scopes before touching the visible list. A failing
                // render drops all staged listeners and leaves old rows intact.
                let mut staged = BTreeMap::new();
                let mut positions = retained.clone();
                let mut fresh = Vec::new();
                for (index, (key, item)) in keys.iter().zip(&items).enumerate() {
                    if retained[index] != reconcile::NEW {
                        continue;
                    }
                    let state = signal(item.clone());
                    let native = native_rows.get(index);
                    let scope = with_native_root(native, || render(state.clone()))?;
                    // Server rows already occupy their final positions. Newly
                    // rendered roots are detached and must be inserted.
                    if native.is_some() {
                        positions[index] = index;
                    }
                    fresh.push(state.clone());
                    staged.insert(
                        key.clone(),
                        Row {
                            _state: state,
                            scope,
                        },
                    );
                }
                for row in staged.values() {
                    row.scope.finish_prepare()?;
                }
                if !initialized {
                    if !hydrating {
                        #[cfg(feature = "islands")]
                        super::delivery::dispose_tree(&container);
                        container.set_text_content(None);
                    }
                    initialized = true;
                }
                // Release the previous order before removed rows drop, as
                // their own rows hold the remaining references.
                let mut previous: Vec<_> =
                    std::mem::take(&mut states).into_iter().map(Some).collect();
                let mut fresh = fresh.into_iter();
                let next: Vec<Signal<T>> = retained
                    .iter()
                    .map(|&position| match position {
                        reconcile::NEW => fresh.next().expect("staged row"),
                        position => previous[position].take().expect("retained row"),
                    })
                    .collect();
                drop(previous);
                rows.retain(|key, row| {
                    let keep = unique.contains_next(key);
                    if !keep {
                        remove_tree(&row.scope.root);
                    }
                    keep
                });
                if rows.is_empty() {
                    rows = staged;
                } else if staged.len() <= rows.len() / (rows.len().ilog2() as usize + 1) {
                    // Avoid rebuilding the existing tree for sparse additions;
                    // keep the linear sorted merge when insertions are dense.
                    rows.extend(staged);
                } else {
                    rows.append(&mut staged);
                }
                let stationary = reconcile::stationary(&positions);
                for (state, item) in next.iter().zip(items) {
                    state.set(item);
                }
                (order, states) = (keys, next);
                for (index, keep) in stationary.iter().enumerate().rev() {
                    if !keep {
                        let anchor = order
                            .get(index + 1)
                            .map(|key| rows[key].scope.root.as_ref());
                        container.insert_before(&rows[&order[index]].scope.root, anchor)?;
                    }
                }
                for row in rows.values() {
                    row.scope.commit();
                }
                restore_focus(&document, &container, focused)
            })
        })
    }
}

/// Adopt the server-rendered rows, which must match `keys` in order.
fn server_rows<K>(
    container: &Element,
    keys: &[K],
    encode: &EncodeKey<K>,
) -> Result<Vec<Element>, JsValue> {
    let mut node = container.first_element_child();
    let mut rows = Vec::with_capacity(keys.len());
    for key in keys {
        let row = node
            .take()
            .ok_or_else(|| JsValue::from_str("missing native row"))?;
        if strings::attribute(&row, strings::Name::Key).as_deref() != Some(encode(key)?.as_str()) {
            return Err(JsValue::from_str("native row key mismatch"));
        }
        node = row.next_element_sibling();
        rows.push(row);
    }
    if node.is_some() {
        return Err(JsValue::from_str("unexpected native row"));
    }
    Ok(rows)
}

type Selection = (u32, u32, String);

/// The focused descendant of `container` and, for an input, its selection.
fn focused(document: &Document, container: &Element) -> Option<(HtmlElement, Option<Selection>)> {
    let focused = document
        .active_element()
        .filter(|node| container.contains(Some(node)))?
        .dyn_into::<HtmlElement>()
        .ok()?;
    let selection = focused.dyn_ref::<HtmlInputElement>().and_then(|input| {
        Some((
            input.selection_start().ok()??,
            input.selection_end().ok()??,
            input.selection_direction().ok()??,
        ))
    });
    Some((focused, selection))
}

// insertBefore can blur a node even when moving it within the same list.
// Restore focus only if that original node survives.
fn restore_focus(
    document: &Document,
    container: &Element,
    focused: Option<(HtmlElement, Option<Selection>)>,
) -> Result<(), JsValue> {
    let Some((focused, selection)) = focused.filter(|(node, _)| container.contains(Some(node)))
    else {
        return Ok(());
    };
    if document
        .active_element()
        .is_some_and(|node| node.is_same_node(Some(&focused)))
    {
        return Ok(());
    }
    focused.focus()?;
    if let (Some(input), Some((start, end, direction))) =
        (focused.dyn_ref::<HtmlInputElement>(), selection)
    {
        input.set_selection_range_with_direction(start, end, &direction)?;
    }
    Ok(())
}
