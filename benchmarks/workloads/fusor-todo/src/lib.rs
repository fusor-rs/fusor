use fusor::{Signal, signal};
use std::{cell::Cell, rc::Rc};
#[derive(Clone)]
struct Task {
    id: u32,
    title: String,
    done: Signal<bool>,
}
impl PartialEq for Task {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
struct Model {
    tasks: Signal<Vec<Task>>,
    title: Signal<String>,
    filter: Signal<String>,
    next: Cell<u32>,
}
struct App {
    model: Rc<Model>,
}
impl App {
    fn new() -> Self {
        Self {
            model: Rc::new(Model {
                tasks: signal(
                    (0..20)
                        .map(|id| Task {
                            id,
                            title: format!("Task {}", id + 1),
                            done: signal(false),
                        })
                        .collect(),
                ),
                title: signal(String::new()),
                filter: signal("all".into()),
                next: Cell::new(20),
            }),
        }
    }
}
impl Model {
    fn add(&self) {
        let title = self.title.get().trim().to_owned();
        if title.is_empty() {
            return;
        }
        let id = self.next.get();
        self.next.set(id + 1);
        self.tasks.update(|tasks| {
            tasks.push(Task {
                id,
                title,
                done: signal(false),
            })
        });
        self.title.set(String::new());
    }
    fn visible(&self) -> Vec<Task> {
        let filter = self.filter.get();
        self.tasks
            .get()
            .into_iter()
            .filter(|task| filter == "all" || task.done.get() == (filter == "done"))
            .collect()
    }
    fn done(&self) -> usize {
        self.tasks
            .with(|tasks| tasks.iter().filter(|task| task.done.get()).count())
    }
}
#[derive(fusor::FromInputs)]
struct Row {
    #[input]
    item: fusor::Memo<Task>,
    #[input]
    model: Rc<Model>,
}
fusor::bindings!(app);
include!(env!("FUSOR_MODULE"));
