use super::{
    ElementTarget, JsValue, Scope, document, reconcile, remove_tree, strings, with_native_root,
};
use crate::{Signal, signal, untrack};
use std::collections::BTreeMap;
use wasm_bindgen::JsCast;
use web_sys::{Document, Element, HtmlElement, HtmlInputElement};

type EncodeKey<K> = dyn Fn(&K) -> Result<String, JsValue>;

/// Key extraction and serialization used to match server-rendered rows.
#[doc(hidden)]
pub struct HydratedKeys<F, E> {
    pub key: F,
    pub encode: E,
}

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
        self.keyed_inner(
            target,
            items,
            key,
            RowFactory {
                render,
                encode: None,
            },
        )
    }

    /// Generated shared templates carry serialized row identities in native HTML.
    #[doc(hidden)]
    pub fn keyed_hydrated<T, K>(
        &mut self,
        target: impl ElementTarget,
        items: impl Fn() -> Vec<T> + 'static,
        keys: HydratedKeys<
            impl Fn(&T) -> K + 'static,
            impl Fn(&K) -> Result<String, JsValue> + 'static,
        >,
        render: impl Fn(Signal<T>) -> Result<Scope, JsValue> + 'static,
    ) -> Result<(), JsValue>
    where
        T: Clone + PartialEq + 'static,
        K: Ord + Clone + 'static,
    {
        self.keyed_inner(
            target,
            items,
            keys.key,
            RowFactory {
                render,
                encode: Some(Box::new(keys.encode)),
            },
        )
    }

    fn keyed_inner<T, K, R>(
        &mut self,
        target: impl ElementTarget,
        items: impl Fn() -> Vec<T> + 'static,
        key: impl Fn(&T) -> K + 'static,
        factory: RowFactory<K, R>,
    ) -> Result<(), JsValue>
    where
        T: Clone + PartialEq + 'static,
        K: Ord + Clone + 'static,
        R: Fn(Signal<T>) -> Result<Scope, JsValue> + 'static,
    {
        let hydrating = self.is_hydrating();
        let mut list = KeyedRows {
            container: target.resolve(self)?,
            rows: BTreeMap::new(),
            order: Vec::new(),
            states: Vec::new(),
            initialized: false,
            settled: false,
            hydrating,
            factory,
            queue: self.mount_queue.clone(),
        };
        self.bind(move || {
            let items = items();
            untrack(|| {
                let keys = items.iter().map(&key).collect();
                list.update(items, keys)
            })
        })
    }
}

struct RowFactory<K, R> {
    render: R,
    encode: Option<Box<EncodeKey<K>>>,
}

struct KeyedRows<T, K, R> {
    container: Element,
    // The map orders cleanup by key; order and states mirror DOM order.
    rows: BTreeMap<K, Row<T>>,
    order: Vec<K>,
    states: Vec<Signal<T>>,
    initialized: bool,
    settled: bool,
    hydrating: bool,
    factory: RowFactory<K, R>,
    queue: Option<std::rc::Rc<super::commit::CommitQueue>>,
}

struct PreparedRows<T, K> {
    rows: BTreeMap<K, Row<T>>,
    positions: Vec<usize>,
    states: Vec<Signal<T>>,
}

impl<T, K, R> KeyedRows<T, K, R>
where
    T: Clone + PartialEq + 'static,
    K: Ord + Clone + 'static,
    R: Fn(Signal<T>) -> Result<Scope, JsValue>,
{
    fn update(&mut self, items: Vec<T>, keys: Vec<K>) -> Result<(), JsValue> {
        let (retained, removal) = self.plan(&keys)?;
        let native_rows = self.native_rows(&keys)?;
        let document = document()?;
        let focused = focused(&document, &self.container);
        let mut staged = self.prepare_rows(&items, &keys, &retained, &native_rows)?;
        self.initialize();
        let next = self.reorder_states(&retained, staged.states);
        self.remove_rows(removal);
        // Settled rows need committing again only when descendants queued setup.
        let fresh_keys = self.settled.then(|| staged.rows.keys().cloned().collect());
        self.settled = false;
        self.merge_rows(&mut staged.rows);
        let stationary = reconcile::stationary(&staged.positions);
        for (state, item) in next.iter().zip(items) {
            state.set(item);
        }
        (self.order, self.states) = (keys, next);
        self.position_rows(&stationary)?;
        self.commit_rows(fresh_keys);
        self.settled = true;
        restore_focus(&document, &self.container, focused)
    }

    fn plan<'a>(&self, keys: &'a [K]) -> Result<(Vec<usize>, Removal<'a, K>), JsValue> {
        let duplicate = || JsValue::from_str("fusor: duplicate key in list");
        // Small edits resolve their changed keys directly; other edits use a sorted merge.
        match reconcile::small_edit(&self.order, keys, |key| self.rows.contains_key(key)) {
            Some(plan) => {
                let reconcile::EditPlan { positions, removed } = plan.map_err(|()| duplicate())?;
                Ok((positions, Removal::Direct(removed)))
            }
            None => {
                let unique = reconcile::SortedKeys::new(keys).ok_or_else(duplicate)?;
                let positions = reconcile::previous_positions(&self.order, &unique);
                Ok((positions, Removal::Sorted(unique)))
            }
        }
    }

    fn native_rows(&self, keys: &[K]) -> Result<Vec<Element>, JsValue> {
        if !self.hydrating || self.initialized {
            return Ok(Vec::new());
        }
        let encode =
            self.factory.encode.as_ref().ok_or_else(|| {
                JsValue::from_str("hydrated lists require generated key metadata")
            })?;
        server_rows(&self.container, keys, encode)
    }

    fn prepare_rows(
        &self,
        items: &[T],
        keys: &[K],
        retained: &[usize],
        native_rows: &[Element],
    ) -> Result<PreparedRows<T, K>, JsValue> {
        // Stage and validate every scope before changing the visible list.
        let mut staged = PreparedRows {
            rows: BTreeMap::new(),
            positions: retained.to_vec(),
            states: Vec::new(),
        };
        for (index, (key, item)) in keys.iter().zip(items).enumerate() {
            if retained[index] != reconcile::NEW {
                continue;
            }
            let state = signal(item.clone());
            let native = native_rows.get(index);
            let scope = with_native_root(native, || (self.factory.render)(state.clone()))?;
            // Adopted rows already occupy their final positions.
            if native.is_some() {
                staged.positions[index] = index;
            }
            staged.states.push(state.clone());
            staged.rows.insert(
                key.clone(),
                Row {
                    _state: state,
                    scope,
                },
            );
        }
        for row in staged.rows.values() {
            row.scope.finish_prepare()?;
        }
        Ok(staged)
    }

    fn initialize(&mut self) {
        if self.initialized {
            return;
        }
        if !self.hydrating {
            #[cfg(feature = "islands")]
            super::delivery::dispose_tree(&self.container);
            self.container.set_text_content(None);
        }
        self.initialized = true;
    }

    fn reorder_states(&mut self, retained: &[usize], fresh: Vec<Signal<T>>) -> Vec<Signal<T>> {
        // Release the old order before removed rows drop their remaining references.
        let mut previous: Vec<_> = std::mem::take(&mut self.states)
            .into_iter()
            .map(Some)
            .collect();
        let mut fresh = fresh.into_iter();
        retained
            .iter()
            .map(|&position| match position {
                reconcile::NEW => fresh.next().expect("staged row"),
                position => previous[position].take().expect("retained row"),
            })
            .collect()
    }

    fn remove_rows(&mut self, mut removal: Removal<'_, K>) {
        match &mut removal {
            Removal::Sorted(unique) => self.rows.retain(|key, row| {
                let keep = unique.contains_next(key);
                if !keep {
                    remove_tree(&row.scope.root);
                }
                keep
            }),
            Removal::Direct(removed) => {
                // Match retain's ascending key order and detach before dropping the key.
                reconcile::sort_few(removed, &self.order);
                for &index in removed.iter() {
                    if let Some(entry) = self.rows.remove_entry(&self.order[index]) {
                        remove_tree(&entry.1.scope.root);
                        drop(entry);
                    }
                }
            }
        }
    }

    fn merge_rows(&mut self, staged: &mut BTreeMap<K, Row<T>>) {
        if self.rows.is_empty() {
            self.rows = std::mem::take(staged);
        } else if staged.len() <= self.rows.len() / (self.rows.len().ilog2() as usize + 1) {
            // Sparse insertions preserve the existing tree; dense insertions merge linearly.
            self.rows.extend(std::mem::take(staged));
        } else {
            self.rows.append(staged);
        }
    }

    fn position_rows(&self, stationary: &[bool]) -> Result<(), JsValue> {
        for (index, keep) in stationary.iter().enumerate().rev() {
            if !keep {
                let anchor = self
                    .order
                    .get(index + 1)
                    .map(|key| self.rows[key].scope.root.as_ref());
                strings::insert_before(
                    &self.container,
                    &self.rows[&self.order[index]].scope.root,
                    anchor,
                )?;
            }
        }
        Ok(())
    }

    fn commit_rows(&self, fresh_keys: Option<Vec<K>>) {
        let idle = self.queue.as_ref().is_none_or(|queue| queue.is_idle());
        match fresh_keys.filter(|_| idle) {
            Some(keys) => {
                for key in &keys {
                    self.rows[key].scope.commit();
                }
            }
            None => {
                for row in self.rows.values() {
                    row.scope.commit();
                }
            }
        }
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
