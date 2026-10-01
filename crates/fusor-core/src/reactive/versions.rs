//! Optimistic render validation. Values remain in their ordinary signals;
//! only source identities and versions are retained, never historical values.
use super::{CURRENT, graph::Source};
use std::{cell::RefCell, rc::Rc};

#[derive(Clone, Default)]
/// A set of reactive source identities and their observed versions. Capturing
/// versions does not retain historical values or create an effect subscription.
pub struct Versions(Vec<(Rc<Source>, u64)>);

struct Collector {
    observer: Option<usize>,
    versions: Versions,
}

thread_local! {
    static COLLECTORS: RefCell<Vec<Collector>> = const { RefCell::new(Vec::new()) };
}

fn observer() -> Option<usize> {
    CURRENT.with(|current| current.borrow().as_ref().map(|weak| weak.as_ptr() as usize))
}

pub(super) fn record(source: &Rc<Source>) {
    let current = observer();
    COLLECTORS.with(|collectors| {
        for collector in collectors.borrow_mut().iter_mut() {
            // A memo's implementation is its own observer. Capture the memo's
            // published version, not incidental reads while refreshing it.
            if collector.observer == current
                && !collector
                    .versions
                    .0
                    .iter()
                    .any(|(s, _)| Rc::ptr_eq(s, source))
            {
                collector
                    .versions
                    .0
                    .push((source.clone(), source.version.get()));
            }
        }
    });
}

struct Pop;
impl Drop for Pop {
    fn drop(&mut self) {
        COLLECTORS.with(|stack| {
            stack.borrow_mut().pop();
        });
    }
}

impl Versions {
    pub(crate) fn include(&self) {
        let current = observer();
        COLLECTORS.with(|collectors| {
            for collector in collectors
                .borrow_mut()
                .iter_mut()
                .filter(|collector| collector.observer == current)
            {
                for (source, version) in &self.0 {
                    if !collector
                        .versions
                        .0
                        .iter()
                        .any(|(s, _)| Rc::ptr_eq(s, source))
                    {
                        collector.versions.0.push((source.clone(), *version));
                    }
                }
            }
        });
    }
    /// Capture tracked sources read by this callback. Ordinary tracking remains
    /// enabled; nested collectors restore correctly even during unwinding.
    pub fn capture<R>(read: impl FnOnce() -> R) -> (R, Self) {
        COLLECTORS.with(|stack| {
            stack.borrow_mut().push(Collector {
                observer: observer(),
                versions: Self::default(),
            })
        });
        let guard = Pop;
        let value = read();
        let versions = COLLECTORS.with(|stack| {
            std::mem::take(&mut stack.borrow_mut().last_mut().expect("capture").versions)
        });
        drop(guard);
        (value, versions)
    }

    /// Check that every source still has its captured version. Memo sources may
    /// refresh their lazy computations while their versions are checked.
    pub fn is_current(&self) -> bool {
        self.0
            .iter()
            .all(|(source, version)| source.version() == *version)
    }

    /// Compare identities and versions, independent of capture order.
    pub fn same(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self.0.iter().all(|(source, version)| {
                other
                    .0
                    .iter()
                    .any(|(s, v)| Rc::ptr_eq(source, s) && version == v)
            })
    }

    /// Read a completion/status source without treating it as application input.
    /// Ordinary effect dependency tracking remains enabled.
    pub fn exclude<R>(read: impl FnOnce() -> R) -> R {
        struct Restore(Option<Vec<Collector>>);
        impl Drop for Restore {
            fn drop(&mut self) {
                COLLECTORS.with(|stack| *stack.borrow_mut() = self.0.take().expect("restore"));
            }
        }
        let _restore = Restore(Some(COLLECTORS.with(|stack| stack.take())));
        read()
    }
}
