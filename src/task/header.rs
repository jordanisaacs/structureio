use std::task::Waker;

use super::{raw::TaskVTable, state::State, utils::abort_on_panic};

pub(super) struct Header {
    /// Current state of the task
    pub(super) state: State,

    /// The virtual table
    ///
    /// In addition to the actual waker virtual table, it also contains pointers to serveral other
    /// methods necessary for bookkeeping the heap-allocated task
    pub(super) vtable: &'static TaskVTable,

    /// The task that is blocked on the `JoinHandle`
    ///
    /// This waker needs to be woken up once the task completes or is cancelled
    pub(super) awaiter: Option<Waker>,

    pub(super) debugging: bool,

    #[cfg(feature = "debugging")]
    thread_id: usize
}

impl Header {
    #[cfg(not(feature="debugging"))]
    pub(super) fn new(vtable: &'static TaskVTable) -> Self {
        Self {
            state: State::new(),
            awaiter: None,
            vtable,
            debugging: false,
        }
    }

    #[cfg(feature = "debugging")]
    pub(super) fn new(vtable: &'static TaskVTable) -> Self {
        use crate::utils::thread_id::get_current_thread_id;

        Self {
            state: State::new(),
            awaiter: None,
            vtable,
            debugging: false,
            thread_id: get_current_thread_id()
        }
    }

    #[cfg(feature = "debugging")]
    pub(super) fn thread_id(&self) -> usize {
        self.thread_id
    }

    /// Notifies the awaiter blocked on this task.
    ///
    /// If the awaiter is the same as the current waker, it will not be notified.
    pub(super) fn notify(&mut self, current: Option<&Waker>) {
        let waker = self.awaiter.take();

        if let Some(w) = waker {
            // Need a safeguard against panics because waking can panic
            abort_on_panic(|| match current {
                None => {
                    w.wake()
                }
                Some(c) if !w.will_wake(c) => w.wake(),
                Some(_) => {
                    drop(w);
                }
            })
        }
    }

    /// Registers a new awaiter blocked on this task.
    ///
    /// This method is called when `JoinHandle` is polled and the task has not completed.
    pub(super) fn register(&mut self, waker: &Waker) {
        // Put the waker into the awaiter field.
        abort_on_panic(|| self.awaiter = Some(waker.clone()));
    }

    pub(super) fn set_debugging(&mut self, debug: bool) {
        self.debugging = debug
    }

    pub(super) fn to_compact_string(&self) -> String {
        format!("{}", self.state.to_compact_string())
    }
}
