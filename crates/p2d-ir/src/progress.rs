//! Where a long conversion is, for the waiting screen. The host sets a sink on the thread that
//! converts (`with_sink`) and the steps report to it; without one, `report` does nothing.

use std::cell::RefCell;
use std::sync::Arc;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "step", rename_all = "snake_case")]
pub enum Step {
    /// Reading the text of page `page` (from 1) of `pages`.
    Read { page: usize, pages: usize },
    /// Drawing the pictures and drawings of the pages: `done` of `pages`.
    Draw { done: usize, pages: usize },
    /// Placing the text of page `page` (from 1) of `pages`.
    Layout { page: usize, pages: usize },
    /// Downloading a font the Mac lacks.
    Font { family: String },
    /// Sampling the colour behind the lines of page `page` (from 1) of `pages`.
    Colors { page: usize, pages: usize },
    /// Writing the new file.
    Write,
}

pub type Sink = Arc<dyn Fn(Step) + Send + Sync>;

thread_local! {
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
}

/// Run `work` with `sink` receiving the steps it reports on this thread.
pub fn with_sink<T>(sink: Sink, work: impl FnOnce() -> T) -> T {
    // Put back even if `work` panics (the server catches it and goes on).
    struct Restore(Option<Sink>);
    impl Drop for Restore {
        fn drop(&mut self) {
            SINK.with(|s| *s.borrow_mut() = self.0.take());
        }
    }
    let _restore = Restore(SINK.with(|s| s.borrow_mut().replace(sink)));
    work()
}

/// The sink of this thread, to hand to the threads it starts.
pub fn current() -> Option<Sink> {
    SINK.with(|s| s.borrow().clone())
}

pub fn report(step: Step) {
    if let Some(sink) = current() {
        sink(step);
    }
}
