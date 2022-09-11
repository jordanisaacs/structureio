use crate::task::task_impl;

pub mod executor;
mod queue;
mod scheduler;
pub mod task;

/// When a  future is internally spawned using `task::spawn()` or `task::spawn_local()` we get back
/// two values:
///
/// 1. a `task::Task<()>`, which we refer to as a `Runnable`
/// 2. a `task::JoinHandle<T, ()>` which is wrapped inside a `Task<T>`
///
/// Once a `Runnable` is run, it "vanishes" and only reappears when its future is woken. When it's
/// woken up, its schedule function is called, which means the `Runnable` gets pushed into a task
/// queue in an executor.
pub(super) type Runnable = task_impl::Task;

pub(crate) use executor::LOCAL_EX;
