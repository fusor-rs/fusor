//! Windowed rows for `<ForEach>`: only the rows near the viewport are mounted.
use fusor::dom::Listener;
use fusor::{Memo, Signal, memo, signal};
use std::cell::RefCell;
use std::ops::Range;
use web_sys::{Element, EventTarget, Window};

/// Which element scrolls the canvas.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scroller {
    /// The canvas's parent element scrolls. Give it a fixed height and `overflow: auto`.
    Container,
    /// The page scrolls. The canvas can sit anywhere in the document.
    Page,
}

/// One row to mount, placed inside the canvas in CSS pixels.
#[derive(Clone, PartialEq, Debug)]
pub struct VirtualRow<T> {
    pub item: T,
    /// The row's position in the whole collection.
    pub index: usize,
    /// Distance from the canvas top to the row top.
    pub offset: f64,
    pub size: f64,
}

/// The part of the canvas the scroller shows, in CSS pixels from the canvas top.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
struct Viewport {
    start: f64,
    extent: f64,
}

#[derive(PartialEq)]
struct Layout<T> {
    rows: Vec<VirtualRow<T>>,
    total: f64,
}

impl<T> Layout<T> {
    fn new(items: Vec<T>, size: &dyn Fn(&T) -> f64) -> Self {
        let mut total = 0.0;
        let rows = items
            .into_iter()
            .enumerate()
            .map(|(index, item)| {
                let size = size(&item);
                let row = VirtualRow {
                    item,
                    index,
                    offset: total,
                    size,
                };
                total += size;
                row
            })
            .collect();
        Self { rows, total }
    }

    /// The rows intersecting the viewport, extended by `overscan` rows at each edge.
    fn window(&self, viewport: Viewport, overscan: usize) -> Range<usize> {
        let end = viewport.start + viewport.extent;
        let first = self
            .rows
            .partition_point(|row| row.offset + row.size <= viewport.start);
        let last = self.rows.partition_point(|row| row.offset < end);
        first.saturating_sub(overscan)..last.saturating_add(overscan).min(self.rows.len())
    }
}

/// Mounts only the rows of a long collection that are near the viewport.
///
/// Feed [`Self::visible`] to `<ForEach>` inside a canvas element whose `height`
/// comes from [`Self::total_size`], position each row absolutely from its
/// `offset` and `size`, and hand the canvas over with
/// `rust:element="state.rows.attach(element)"`. Rows scrolled out of view are
/// unmounted and mount afresh when they return; keep their state in signals
/// outside the row. Row sizes are declared, never measured.
pub struct VirtualRows<T> {
    layout: Memo<Layout<T>>,
    viewport: Signal<Viewport>,
    visible: Memo<Vec<VirtualRow<T>>>,
    scroller: Scroller,
    attachment: RefCell<Option<Attachment>>,
}

impl<T: Clone + PartialEq + 'static> VirtualRows<T> {
    /// `items` and `size` are read like bindings: the signals they read recompute
    /// the layout. `size` returns a row's height in CSS pixels. `overscan` is how
    /// many rows beyond each edge of the viewport stay mounted while scrolling.
    pub fn new(
        items: impl Fn() -> Vec<T> + 'static,
        size: impl Fn(&T) -> f64 + 'static,
        scroller: Scroller,
        overscan: usize,
    ) -> Self {
        let layout = memo(move || Layout::new(items(), &size));
        let viewport = signal(Viewport::default());
        let visible = {
            let layout = layout.clone();
            let viewport = viewport.clone();
            memo(move || {
                layout.with(|layout| layout.rows[layout.window(viewport.get(), overscan)].to_vec())
            })
        };
        Self {
            layout,
            viewport,
            visible,
            scroller,
            attachment: RefCell::new(None),
        }
    }

    /// The rows to mount now, in collection order. A tracked read.
    pub fn visible(&self) -> Vec<VirtualRow<T>> {
        self.visible.get()
    }

    /// The canvas height in CSS pixels: the sum of every row's size. A tracked read.
    pub fn total_size(&self) -> f64 {
        self.layout.with(|layout| layout.total)
    }

    /// Follow `canvas`: measure the viewport now and again on every scroll and
    /// resize, until this value drops. Call it from `rust:element` on the canvas.
    pub fn attach(&self, canvas: Element) {
        let attachment = Attachment::new(canvas, self.scroller, self.viewport.clone());
        *self.attachment.borrow_mut() = Some(attachment);
    }

    /// Scroll so the row at `index` starts at the top of the viewport. Does
    /// nothing before [`Self::attach`] or for an index past the last row.
    pub fn scroll_to(&self, index: usize) {
        let offset = self
            .layout
            .with_untracked(|layout| layout.rows.get(index).map(|row| row.offset));
        if let (Some(offset), Some(attachment)) = (offset, self.attachment.borrow().as_ref()) {
            attachment.scroll_to(offset);
        }
    }
}

struct Attachment {
    canvas: Element,
    scroller: Scroller,
    viewport: Signal<Viewport>,
    _listeners: Vec<Listener>,
}

impl Attachment {
    fn new(canvas: Element, scroller: Scroller, viewport: Signal<Viewport>) -> Self {
        let window = window();
        let scrolling: EventTarget = match scroller {
            Scroller::Page => window.clone().into(),
            Scroller::Container => container(&canvas).into(),
        };
        let remeasure = {
            let canvas = canvas.clone();
            let viewport = viewport.clone();
            move |_event| viewport.set(measure(&canvas, scroller))
        };
        let listeners = vec![
            Listener::new(scrolling, "scroll", remeasure.clone()),
            Listener::new(window.into(), "resize", remeasure),
        ]
        .into_iter()
        .map(|listener| listener.expect("windows and elements accept event listeners"))
        .collect();
        viewport.set(measure(&canvas, scroller));
        Self {
            canvas,
            scroller,
            viewport,
            _listeners: listeners,
        }
    }

    fn scroll_to(&self, offset: f64) {
        let canvas_top = self.canvas.get_bounding_client_rect().top();
        match self.scroller {
            Scroller::Page => {
                let window = window();
                let x = window.scroll_x().expect("window.scrollX is readable");
                let y = window.scroll_y().expect("window.scrollY is readable");
                window.scroll_to_with_x_and_y(x, y + canvas_top + offset);
            }
            Scroller::Container => {
                let container = container(&self.canvas);
                let canvas_start = f64::from(container.scroll_top()) + canvas_top
                    - container.get_bounding_client_rect().top();
                container.set_scroll_top((canvas_start + offset).round() as i32);
            }
        }
        self.viewport.set(measure(&self.canvas, self.scroller));
    }
}

fn measure(canvas: &Element, scroller: Scroller) -> Viewport {
    let canvas_top = canvas.get_bounding_client_rect().top();
    match scroller {
        Scroller::Page => Viewport {
            start: -canvas_top,
            extent: inner_height(&window()),
        },
        Scroller::Container => {
            let container = container(canvas);
            Viewport {
                start: container.get_bounding_client_rect().top() - canvas_top,
                extent: f64::from(container.client_height()),
            }
        }
    }
}

fn window() -> Window {
    web_sys::window().expect("rust:element hands over an element of a browser document")
}

fn container(canvas: &Element) -> Element {
    canvas
        .parent_element()
        .expect("a Scroller::Container canvas is inside its scrolling parent element")
}

fn inner_height(window: &Window) -> f64 {
    window
        .inner_height()
        .expect("window.innerHeight is readable")
        .as_f64()
        .expect("window.innerHeight is a number")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout<u32> {
        Layout::new((0..10).collect(), &|item| f64::from(item % 3 + 1) * 10.0)
    }

    #[test]
    fn places_rows_back_to_back_from_declared_sizes() {
        let layout = layout();
        let offsets: Vec<f64> = layout.rows.iter().map(|row| row.offset).collect();
        assert_eq!(
            offsets,
            [
                0.0, 10.0, 30.0, 60.0, 70.0, 90.0, 120.0, 130.0, 150.0, 180.0
            ]
        );
        assert_eq!(layout.rows[4].index, 4);
        assert_eq!(layout.total, 190.0);
    }

    #[test]
    fn windows_the_rows_touching_the_viewport_plus_overscan() {
        let layout = layout();
        let viewport = |start, extent| Viewport { start, extent };
        assert_eq!(layout.window(viewport(0.0, 0.0), 0), 0..0);
        assert_eq!(layout.window(viewport(0.0, 0.0), 2), 0..2);
        assert_eq!(layout.window(viewport(0.0, 25.0), 0), 0..2);
        assert_eq!(layout.window(viewport(30.0, 40.0), 0), 2..4);
        assert_eq!(layout.window(viewport(30.0, 40.0), 1), 1..5);
        assert_eq!(layout.window(viewport(-50.0, 40.0), 0), 0..0);
        assert_eq!(layout.window(viewport(185.0, 100.0), 3), 6..10);
        assert_eq!(layout.window(viewport(500.0, 100.0), 3), 7..10);
        assert_eq!(
            Layout::<u32>::new(Vec::new(), &|_| 10.0).window(viewport(0.0, 100.0), 3),
            0..0
        );
    }
}
