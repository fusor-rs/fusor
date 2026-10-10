//! Reproducible native workloads, not a browser or cross-framework benchmark.
//! cargo run -p fusor-core --example memo_workload --release --locked
use fusor::{derived, effect, memo, signal};
use std::{cell::Cell, hint::black_box, rc::Rc, time::Instant};

#[derive(Clone, Copy, Debug)]
enum Projection {
    Derived,
    Memo,
}
impl Projection {
    fn read(self, compute: impl Fn() -> u64 + 'static) -> Rc<dyn Fn() -> u64> {
        match self {
            Self::Memo => {
                let value = memo(compute);
                Rc::new(move || value.get())
            }
            Self::Derived => {
                let value = derived(compute);
                Rc::new(move || value.get())
            }
        }
    }
}

struct Workload {
    name: &'static str,
    consumers: usize,
    updates: u64,
    work: u64,
    group: u64,
}

fn run(workload: &Workload, projection: Projection) {
    let input = signal(0_u64);
    let computations = Rc::new(Cell::new(0_u64));
    let renders = Rc::new(Cell::new(0_u64));
    let (work, group) = (workload.work, workload.group);
    let compute = {
        let (input, computations) = (input.clone(), computations.clone());
        move || {
            computations.set(computations.get() + 1);
            let mut n = input.get() / group;
            for _ in 0..work {
                n = black_box(n.wrapping_mul(6364136223846793005).wrapping_add(1));
            }
            n
        }
    };
    let read = projection.read(compute);
    let subscriptions: Vec<_> = (0..workload.consumers)
        .map(|_| {
            let (read, renders) = (read.clone(), renders.clone());
            effect(move || {
                black_box(read());
                renders.set(renders.get() + 1);
            })
        })
        .collect();
    computations.set(0);
    renders.set(0);
    let start = Instant::now();
    for value in 1..=workload.updates {
        input.set(black_box(value));
    }
    let elapsed = start.elapsed().as_micros();
    println!(
        "{},{projection:?},{},{},{},{},{elapsed}",
        workload.name,
        workload.consumers,
        workload.updates,
        computations.get(),
        renders.get()
    );
    assert_eq!(
        computations.get(),
        workload.updates
            * match projection {
                Projection::Memo => 1,
                Projection::Derived => workload.consumers as u64,
            }
    );
    assert_eq!(
        renders.get(),
        match projection {
            Projection::Memo => workload.updates / group,
            Projection::Derived => workload.updates,
        } * workload.consumers as u64
    );
    drop(subscriptions);
}

fn main() {
    println!("workload,mode,consumers,updates,computations,effects,microseconds");
    let workloads = [
        Workload {
            name: "cheap scalar",
            consumers: 1,
            updates: 100_000,
            work: 0,
            group: 1,
        },
        Workload {
            name: "shared costly projection",
            consumers: 8,
            updates: 2_000,
            work: 10_000,
            group: 1,
        },
        Workload {
            name: "mostly equal projection",
            consumers: 8,
            updates: 20_000,
            work: 0,
            group: 100,
        },
    ];
    for projection in [Projection::Derived, Projection::Memo] {
        for workload in &workloads {
            run(workload, projection);
        }
    }
}
