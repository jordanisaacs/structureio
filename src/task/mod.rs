pub mod debugging;
mod header;
mod join;
pub mod raw;
mod state;
pub mod task_impl;
mod utils;
pub mod waker_fn;
pub mod yield_now;

use self::task_impl::Task;

pub(crate) use join::JoinHandle;

pub(crate) trait Schedule: Sized + 'static {
    /// Schedule the task
    fn schedule(&self, task: Task);

    /// Schedule the task to run in the near future, yielding the thread to other tasks.
    fn yield_now(&self, task: Task) {
        self.schedule(task);
    }
}

/// We can't use `?` in const contexts yet, so this macro acts
/// as a workaround.
macro_rules! leap {
    ($x: expr) => {{
        match ($x) {
            Some(val) => val,
            None => return None,
        }
    }};
}

/// Mark context for task operation
macro_rules! dbg_context {
    ($ptr:expr, $name:tt, $($body:tt)*) => {{
        let entered = super::debugging::TaskDebugger::enter($ptr, $name);

        defer! {
            if entered {
                super::debugging::TaskDebugger::leave();
            }
        }

        $($body)*
    }};
}

pub(super) use dbg_context;
pub(super) use leap;
