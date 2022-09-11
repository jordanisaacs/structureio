//! Task abstraction for building executors
//!
//! # Spawning
//!
//! To spawn a future onto the Blizzard executor, we first need to allocate it on the heap and keep
//! some state alongside it. The state indicates whether the future is ready for polling, waiting
//! to be woken up, or completed. Such a future is called a *task*.
//!
//! When a task is run, its future gets polled. If polling does not complete the task, that means
//! it's waiting for another future and needs to go to sleep. When woken up, its schedule function
//! will be invoked, pushing it back into the queue so that it can be run again.
//!
//! Paired with a task there is usually a [`JoinHandle`] that can be used to wait for the task's
//! completion.
//!
//! # Cancellation
//!
//! Both [`Task`] and [`JoinHandle`] have methods that cancel the task. When cancelled, the task's
//! future will not be polled again and will get dropped instead.
//!
//! If canceled by the [Task] instance, the task is destroyed immediately. If canceled  by the
//! [`JoinHandle`] instance, it will be scheduled one more time and the next attempt to run it will
//! destroy it.
//!
//! The `JoinHandle` future will then evaluate to `None`, but only after the task's future is
//! dropped
//!
//! # Performance
//!
//! Task construction incurs a single allocation that holds its state, the schedule functions, and
//! the future or the result of the future if completed.
//!
//! # Waking
//!
//! Wakers are reference counted. They cannot be passed between threads. Doing so will result in
//! undefined behavior.
use core::mem;
use std::{future::Future, ptr::NonNull};

use super::{
    dbg_context, debugging::TaskDebugger, header::Header, raw::RawTask, JoinHandle, Schedule,
};

/// Creates a new local task.
///
/// This constructor returns a [`Task`] reference that runs the future and a [`JoinHandle`] that
/// awaits its result.
///
/// When run initially, the task will run immediately polling the `future`. When woken up it gets
/// scheduled for running by the `schedule implementation`.

pub(crate) fn spawn_local<F, R, S>(future: F, scheduler: S) -> JoinHandle<R>
where
    F: Future<Output = R>,
    S: Schedule,
{
    let raw_task = RawTask::<_, R, S>::allocate(future, scheduler);
    // Safety: Guaranteed to point to a RawTask as pointer comes from allocate
    let handle = unsafe { JoinHandle::from_raw(raw_task) };
    handle
}

/// A task reference that runs its future. Not sendable
///
/// At any moment in time, there is at most one [`Task`] reference associated with a particular
/// task. Running consumes the [`Task`] reference and polls its internal future. If the future is
/// still pending after getting polled, the [`Task`] reference won't exist until a [`Waker]
/// notifies the task. If the future completes, its result becomes available to the [`JoinHandle`].
///
/// When a task is woken up, its [`Task`] reference is recreated and passed to the schedule
/// function.
///
/// If the [`Task`] reference is dropped without getting run, the task is automatically cancelled.
/// When cancelled, the task won't be scheduled again even if a [`Waker`] wakes it. It is possible
/// for the [`JoinHandle`] to cancel while the [`Task`] reference exists, in which case an attempt
/// to run the task won't do anything
pub(crate) struct Task {
    raw_task: NonNull<()>,
}

impl Task {
    pub(crate) unsafe fn from_raw(ptr: NonNull<()>) -> Self {
        Task { raw_task: ptr }
    }

    /// Runs the task.
    ///
    /// Returns `true` if the task was woken while running, in which case it gets rescheduled at the
    /// end of this method invocation.
    ///
    /// This method polls the task's future. If the future completes, its result will become
    /// available to the [`JoinHandle`]. and if the future is still pending, the task will have to
    /// be woken up in order to be rescheduled and run again.
    ///
    /// If the task was cancelled by a [`JoinHandle`] before it gets run, then this method won't do
    /// anything.
    ///
    /// It is possible that polling the future panics, in which case the panic will be propagated
    /// into the caller. It is advised that invocations of this method are wrapped inside a
    /// [`catch_unwind`]. If a panic occurs, the task is automatically canceled.
    pub(crate) fn run(self) -> bool {
        let ptr = self.raw_task.as_ptr();
        dbg_context!(ptr, "run", {
            let header = ptr.cast::<Header>();
            mem::forget(self);
            TaskDebugger::set_current_task(ptr);

            unsafe { ((*header).vtable.run)(ptr) }
        })
    }

    pub(crate) fn run_right_away(self) -> bool {
        let ptr = self.raw_task.as_ptr();
        dbg_context!(ptr, "run_right_away", {
            let header = ptr.cast::<Header>();
            mem::forget(self);
            TaskDebugger::set_current_task(ptr);

            unsafe {
                (*header).state.incr_ref();
                ((*header).vtable.run)(ptr)
            }
        })
    }

    pub(crate) fn schedule(self) {
        let ptr = self.raw_task.as_ptr();
        dbg_context!(ptr, "schedule", {
            let header = ptr.cast::<Header>();
            mem::forget(self);
            TaskDebugger::set_current_task(ptr);

            unsafe { ((*header).vtable.schedule)(ptr, true) }
        })
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        // Decrement the ref count
        let ptr = self.raw_task.as_ptr();
        let header = ptr.cast::<Header>();

        unsafe {
            // Cancel the task.
            (*header).state.drop_task();

            ((*header).vtable.drop_future)(ptr);

            // Notify the awaiter that the future has been dropped
            (*header).notify(None);

            // Cancel the task
            (*header).state.drop_task();
        }
    }
}
