//! Versioned dependencies. Notifications mark memos stale; reads validate them.
use super::{CURRENT, EffectInner, NEXT_ID, NEXT_WAVE, QUEUE, memo::MemoNode};
use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    rc::{Rc, Weak},
};

// Small sources keep up to two observers inline in ascending ID order.
// Wider sources retain the ordinary ordered registry and its removal path.
enum Subscribers {
    One(Option<(u64, Weak<Observer>)>),
    Two([(u64, Weak<Observer>); 2]),
    Many(BTreeMap<u64, Weak<Observer>>),
}
impl Default for Subscribers {
    fn default() -> Self {
        Self::One(None)
    }
}
impl Subscribers {
    fn collect(&self) -> VecDeque<Rc<Observer>> {
        match self {
            Self::One(subscriber) => subscriber
                .iter()
                .filter_map(|(_, weak)| weak.upgrade())
                .collect(),
            Self::Two(subscribers) => subscribers
                .iter()
                .filter_map(|(_, weak)| weak.upgrade())
                .collect(),
            Self::Many(subscribers) => subscribers.values().filter_map(Weak::upgrade).collect(),
        }
    }

    fn insert(&mut self, id: u64, observer: Weak<Observer>) -> bool {
        match self {
            Self::One(None) => *self = Self::One(Some((id, observer))),
            Self::One(Some((first, _))) if *first == id => return false,
            Self::One(subscriber) => {
                let (first, old) = subscriber.take().expect("single subscriber");
                *self = Self::Two(if first < id {
                    [(first, old), (id, observer)]
                } else {
                    [(id, observer), (first, old)]
                });
            }
            Self::Two(subscribers) => {
                if subscribers.iter().any(|(existing, _)| *existing == id) {
                    return false;
                }
                let [first, second] =
                    std::mem::replace(subscribers, [(0, Weak::new()), (0, Weak::new())]);
                *self = Self::Many(BTreeMap::from([first, second, (id, observer)]));
            }
            Self::Many(subscribers) => match subscribers.entry(id) {
                std::collections::btree_map::Entry::Vacant(entry) => {
                    entry.insert(observer);
                }
                std::collections::btree_map::Entry::Occupied(_) => return false,
            },
        }
        true
    }

    fn remove(&mut self, id: &u64) {
        match self {
            Self::One(subscriber) => {
                if subscriber.as_ref().is_some_and(|(first, _)| first == id) {
                    *subscriber = None;
                }
            }
            Self::Two(subscribers) => {
                if let Some(index) = subscribers.iter().position(|(existing, _)| existing == id) {
                    let survivor = std::mem::replace(&mut subscribers[1 - index], (0, Weak::new()));
                    *self = Self::One(Some(survivor));
                }
            }
            Self::Many(subscribers) => {
                subscribers.remove(id);
            }
        }
    }
}

pub(super) struct Source {
    pub version: Cell<u64>,
    subscribers: RefCell<Subscribers>,
    memo: Option<Weak<dyn MemoNode>>,
}

impl Source {
    pub fn new(memo: Option<Weak<dyn MemoNode>>) -> Self {
        Self {
            version: Cell::new(0),
            subscribers: RefCell::new(Subscribers::default()),
            memo,
        }
    }

    pub fn advance(&self) {
        self.version.set(
            self.version
                .get()
                .checked_add(1)
                .expect("reactive version exhausted"),
        );
    }

    pub(super) fn version(&self) -> u64 {
        if let Some(memo) = self.memo.as_ref().and_then(Weak::upgrade) {
            memo.refresh();
        }
        self.version.get()
    }

    fn subscribers(&self) -> VecDeque<Rc<Observer>> {
        self.subscribers.borrow().collect()
    }

    pub fn notify(&self) {
        let wave = NEXT_WAVE.with(|next| {
            let wave = next
                .get()
                .checked_add(1)
                .expect("reactive notification ID exhausted");
            next.set(wave);
            wave
        });
        let mut pending = self.subscribers();
        while let Some(observer) = pending.pop_front() {
            if observer.wave.replace(wave) == wave {
                continue;
            }
            match &observer.kind {
                ObserverKind::Effect(effect) => {
                    if let Some(effect) = effect.upgrade().filter(|e| e.active.get()) {
                        if !effect.queued.replace(true) {
                            QUEUE
                                .with(|queue| queue.borrow_mut().push_back(Rc::downgrade(&effect)));
                        }
                    }
                }
                ObserverKind::Memo(memo) => {
                    if let Some(memo) = memo.upgrade() {
                        memo.invalidate();
                        pending.extend(memo.source().subscribers());
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
struct Dependency {
    source: Rc<Source>,
    version: u64,
}

/// Most observers read one or two sources; keep those inline, so a rerun
/// neither allocates nor frees its dependency list.
#[derive(Clone, Default)]
enum Dependencies {
    #[default]
    None,
    One(Dependency),
    Two([Dependency; 2]),
    Many(Vec<Dependency>),
}

impl Dependencies {
    fn push(&mut self, dependency: Dependency) {
        if let Self::Many(dependencies) = self {
            return dependencies.push(dependency);
        }
        *self = match std::mem::take(self) {
            Self::None => Self::One(dependency),
            Self::One(first) => Self::Two([first, dependency]),
            Self::Two([first, second]) => Self::Many(vec![first, second, dependency]),
            Self::Many(_) => unreachable!("pushed above"),
        };
    }

    fn as_slice(&self) -> &[Dependency] {
        match self {
            Self::None => &[],
            Self::One(dependency) => std::slice::from_ref(dependency),
            Self::Two(dependencies) => dependencies,
            Self::Many(dependencies) => dependencies,
        }
    }
}

pub(super) enum ObserverKind {
    Effect(Weak<EffectInner>),
    Memo(Weak<dyn MemoNode>),
}

pub(super) struct Observer {
    id: u64,
    kind: ObserverKind,
    wave: Cell<u64>,
    dependencies: RefCell<Dependencies>,
}

impl Observer {
    pub fn new(kind: ObserverKind) -> Rc<Self> {
        let id = NEXT_ID.with(|next| {
            let id = next
                .get()
                .checked_add(1)
                .expect("reactive observer ID exhausted");
            next.set(id);
            id
        });
        Rc::new(Self {
            id,
            kind,
            wave: Cell::new(0),
            dependencies: RefCell::new(Dependencies::None),
        })
    }

    pub fn unsubscribe(&self) {
        for dependency in self.dependencies.take().as_slice() {
            dependency.source.subscribers.borrow_mut().remove(&self.id);
        }
    }

    pub fn changed(&self) -> bool {
        // Refreshing upstream memos executes user computations. Hold no graph
        // registry borrow across that boundary.
        let snapshot = self.dependencies.borrow().clone();
        snapshot
            .as_slice()
            .iter()
            .any(|d| d.source.version() != d.version)
    }
}

impl Drop for Observer {
    fn drop(&mut self) {
        self.unsubscribe();
    }
}

pub(super) fn track(source: &Rc<Source>) {
    super::versions::record(source);
    let current = CURRENT.with(|current| current.borrow().as_ref().and_then(Weak::upgrade));
    if let Some(current) = current {
        if matches!(&current.kind, ObserverKind::Effect(e) if e.upgrade().is_none_or(|e| !e.active.get()))
        {
            return;
        }
        if source
            .subscribers
            .borrow_mut()
            .insert(current.id, Rc::downgrade(&current))
        {
            current.dependencies.borrow_mut().push(Dependency {
                source: source.clone(),
                version: source.version.get(),
            });
        }
    }
}

#[cfg(test)]
mod subscriber_tests {
    use super::*;

    fn observer() -> Rc<Observer> {
        Observer::new(ObserverKind::Effect(Weak::new()))
    }

    fn ids(subscribers: &Subscribers) -> Vec<u64> {
        subscribers
            .collect()
            .iter()
            .map(|observer| observer.id)
            .collect()
    }

    #[test]
    fn two_inline_subscribers_keep_id_order_and_duplicate_identity() {
        let first = observer();
        let second = observer();
        let mut subscribers = Subscribers::default();
        assert!(subscribers.insert(second.id, Rc::downgrade(&second)));
        assert!(subscribers.insert(first.id, Rc::downgrade(&first)));
        assert!(matches!(subscribers, Subscribers::Two(_)));
        assert_eq!(ids(&subscribers), [first.id, second.id]);
        assert!(!subscribers.insert(first.id, Weak::new()));
        assert!(!subscribers.insert(second.id, Weak::new()));
        assert_eq!(ids(&subscribers), [first.id, second.id]);
    }

    #[test]
    fn inline_removal_preserves_the_other_subscriber_and_can_refill() {
        let first = observer();
        let second = observer();
        let third = observer();
        for removed in [first.id, second.id] {
            let mut subscribers = Subscribers::default();
            subscribers.insert(first.id, Rc::downgrade(&first));
            subscribers.insert(second.id, Rc::downgrade(&second));
            subscribers.remove(&u64::MAX);
            assert!(matches!(subscribers, Subscribers::Two(_)));
            subscribers.remove(&removed);
            let survivor = if removed == first.id {
                second.id
            } else {
                first.id
            };
            assert!(matches!(subscribers, Subscribers::One(Some(_))));
            assert_eq!(ids(&subscribers), [survivor]);
            assert!(subscribers.insert(third.id, Rc::downgrade(&third)));
            assert_eq!(ids(&subscribers), [survivor, third.id]);
            subscribers.remove(&third.id);
            subscribers.remove(&survivor);
            assert!(matches!(subscribers, Subscribers::One(None)));
        }
    }

    #[test]
    fn third_subscriber_promotes_without_changing_order_or_removal() {
        let first = observer();
        let second = observer();
        let third = observer();
        let mut subscribers = Subscribers::default();
        subscribers.insert(third.id, Rc::downgrade(&third));
        subscribers.insert(first.id, Rc::downgrade(&first));
        subscribers.insert(second.id, Rc::downgrade(&second));
        assert!(matches!(subscribers, Subscribers::Many(_)));
        assert_eq!(ids(&subscribers), [first.id, second.id, third.id]);
        assert!(!subscribers.insert(second.id, Weak::new()));
        subscribers.remove(&second.id);
        assert!(matches!(subscribers, Subscribers::Many(_)));
        assert_eq!(ids(&subscribers), [first.id, third.id]);
    }

    #[test]
    fn inline_subscribers_do_not_keep_observers_alive() {
        let first = observer();
        let second = observer();
        let second_weak = Rc::downgrade(&second);
        let mut subscribers = Subscribers::default();
        subscribers.insert(first.id, Rc::downgrade(&first));
        subscribers.insert(second.id, second_weak.clone());
        drop(second);
        assert!(second_weak.upgrade().is_none());
        assert_eq!(ids(&subscribers), [first.id]);
        let third = observer();
        assert!(subscribers.insert(third.id, Rc::downgrade(&third)));
        assert!(matches!(subscribers, Subscribers::Many(_)));
        assert_eq!(ids(&subscribers), [first.id, third.id]);
    }
}
