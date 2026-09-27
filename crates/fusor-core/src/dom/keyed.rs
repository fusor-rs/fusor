use super::{
    ElementTarget, JsValue, Scope, document, reconcile, remove_tree, strings, with_native_root,
};
use crate::{Signal, signal, untrack};
use std::collections::BTreeMap;
use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlElement, HtmlInputElement};

type EncodeKey<K> = dyn Fn(&K) -> Result<String, JsValue>;

/// How a reconcile finds the previous rows whose keys are gone.
enum Removal<'a, K> {
    /// Merge every key against the ordered map.
    Sorted(reconcile::SortedKeys<'a, K>),
    /// The previous indices of the few removed keys.
    Direct(Vec<usize>),
}

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
        // Every row in `rows` was committed by a reconcile that completed.
        let mut settled = false;
        let queue = self.mount_queue.clone();
        self.bind(move || {
            let items = items();
            untrack(|| {
                let keys: Vec<K> = items.iter().map(&key).collect();
                let duplicate = || JsValue::from_str("fusor: duplicate key in list");
                // A small edit resolves its few changed keys directly. Any other
                // change validates and merges through every key in order.
                let (retained, mut removal) =
                    match reconcile::small_edit(&order, &keys, |key| rows.contains_key(key)) {
                        Some(plan) => {
                            let (positions, removed) = plan.map_err(|()| duplicate())?;
                            (positions, Removal::Direct(removed))
                        }
                        None => {
                            let unique =
                                reconcile::SortedKeys::new(&keys).ok_or_else(duplicate)?;
                            let positions = reconcile::previous_positions(&order, &unique);
                            (positions, Removal::Sorted(unique))
                        }
                    };
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
                match &mut removal {
                    Removal::Sorted(unique) => rows.retain(|key, row| {
                        let keep = unique.contains_next(key);
                        if !keep {
                            remove_tree(&row.scope.root);
                        }
                        keep
                    }),
                    Removal::Direct(removed) => {
                        // In ascending key order, as `retain` visits the map.
                        reconcile::sort_few(removed, &order);
                        for &index in removed.iter() {
                            if let Some(row) = rows.remove(&order[index]) {
                                remove_tree(&row.scope.root);
                            }
                        }
                    }
                }
                // Committing a settled row again only finishes setup that a
                // descendant queued, so without pending setup only new rows
                // need committing.
                let fresh_keys: Option<Vec<K>> =
                    settled.then(|| staged.keys().cloned().collect());
                settled = false;
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
                let idle = queue.as_ref().is_none_or(|queue| queue.is_idle());
                match fresh_keys.filter(|_| idle) {
                    Some(keys) => {
                        for key in &keys {
                            rows[key].scope.commit();
                        }
                    }
                    None => {
                        for row in rows.values() {
                            row.scope.commit();
                        }
                    }
                }
                settled = true;
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
    // Compare every row natively in one call. Keys encode in order: a failed
    // encoding is reported after the rows before it and its own row are
    // checked, as when comparing one row at a time.
    let mut encoded = String::new();
    let mut failure = None;
    let mut count = 0;
    for key in keys {
        match encode(key) {
            // A separator inside an encoding needs the one-at-a-time path.
            Ok(value) if value.contains('\n') => {
                return server_rows_one_by_one(container, keys, encode);
            }
            Ok(value) => {
                if count > 0 {
                    encoded.push('\n');
                }
                encoded.push_str(&value);
                count += 1;
            }
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    let rows = strings::server_rows(container, &encoded, count as u32, failure.is_none())?;
    if let Some(error) = failure {
        return Err(error);
    }
    Ok((0..count as u32)
        .map(|index| rows.get(index).unchecked_into())
        .collect())
}

fn server_rows_one_by_one<K>(
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
