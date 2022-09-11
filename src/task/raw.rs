use std::{
    alloc::Layout as StdLayout,
    future::Future,
    mem::{self, ManuallyDrop},
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll, RawWaker, RawWakerVTable, Waker},
};

use super::{
    dbg_context,
    debugging::TaskDebugger,
    header::Header,
    leap,
    task_impl::Task,
    utils::{abort, abort_on_panic, max, Layout},
    Schedule,
};

#[derive(Debug)]
struct TaskLayout {
    layout: StdLayout,
    offset_scheduler: usize,
    offset_future: usize,
    offset_output: usize,
}

pub(crate) struct RawTask<F, R, S> {
    /// The task header
    pub(super) header: *const Header,

    /// The schedule function.
    scheduler: *const S,

    /// The stage of the future.
    future: *mut F,

    /// The stage of the future.
    output: *mut R,
}

pub(super) struct TaskVTable {
    pub(super) schedule: unsafe fn(*const (), bool),
    pub(super) get_output: unsafe fn(*const ()) -> *const (),
    pub(super) drop_future: unsafe fn(*const ()),
    pub(super) run: unsafe fn(*const ()) -> bool,
    pub(super) destroy: unsafe fn(*const ()),
}

impl<F, R, S> Copy for RawTask<F, R, S> {}

impl<F, R, S> Clone for RawTask<F, R, S> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<F, R, S> RawTask<F, R, S> {
    const TASK_LAYOUT: Option<TaskLayout> = Self::eval_task_layout();

    /// Computes the memory layout for a task
    #[inline]
    const fn eval_task_layout() -> Option<TaskLayout> {
        // Compute the layouts for `Header`, `S`, `F`, and `T`.
        let layout_header = Layout::new::<Header>();
        let layout_scheduler = Layout::new::<S>();
        let layout_future = Layout::new::<F>();
        let layout_output = Layout::new::<R>();

        // Compute the layout for `union { F, T }`.
        let size_union = max(layout_future.size(), layout_output.size());
        let align_union = max(layout_future.align(), layout_output.align());
        let layout_union = Layout::from_size_align(size_union, align_union);

        // Compute the layout for `Header` followed `S` and `union { F, T }`.
        let layout = layout_header;
        let (layout, offset_scheduler) = leap!(layout.extend(layout_scheduler));
        let (layout, offset_union) = leap!(layout.extend(layout_union));
        let offset_future = offset_union;
        let offset_output = offset_union;

        Some(TaskLayout {
            layout: unsafe { layout.into_std() },
            offset_scheduler,
            offset_future,
            offset_output,
        })
    }
}

impl<F, R, S> RawTask<F, R, S>
where
    F: Future<Output = R>,
    S: Schedule,
{
    const RAW_WAKER_VTABLE: RawWakerVTable = RawWakerVTable::new(
        Self::clone_waker,
        Self::wake_by_val,
        Self::wake_by_ref,
        Self::drop_waker,
    );

    /// Clone the waker
    ///
    /// # Safety
    ///
    /// ptr must be NonNull and point to
    unsafe fn clone_waker(ptr: *const ()) -> RawWaker {
        dbg_context!(ptr, "clone_waker", {
            #[cfg(feature = "debugging")]
            {
                Self::check_waker_thread(ptr);
            }
            let raw = Self::from_ptr(ptr);
            (*raw.header.cast_mut()).state.incr_ref();
            RawWaker::new(ptr, &Self::RAW_WAKER_VTABLE)
        })
    }

    unsafe fn do_wake(ptr: *const ()) {
        use super::state::TransitionToScheduled;

        let raw = Self::from_ptr(ptr);

        match (*raw.header.cast_mut()).state.transition_to_scheduled() {
            TransitionToScheduled::Submit => {
                Self::schedule(ptr, false);
            }
            TransitionToScheduled::DoNothing => {}
        }
    }

    /// This function checks if the the current
    ///
    /// If it is not, then abort because we are in an unrecoverable state. Wakers are reference
    /// counted, not atomically reference counted. Therefore they should not be sync. But to due to
    /// the design of context/wakers, they are. Thus, all we can do is abort if the waker was
    /// synced to another thread.
    #[cfg(feature = "debugging")]
    unsafe fn check_waker_thread(ptr: *const ()) {
        use crate::utils::thread_id::get_current_thread_id;

        abort_on_panic(|| {
            let raw = Self::from_ptr(ptr);
            let current_id = get_current_thread_id();
            let waker_id = (*raw.header).thread_id();

            if current_id != waker_id {
                panic!("Waker is not on the same thread as the executor")
            }
        })

    }

    /// Wakes a waker.
    unsafe fn wake_by_val(ptr: *const ()) {
        dbg_context!(ptr, "wake_by_val", {
            #[cfg(feature = "debugging")]
            {
                Self::check_waker_thread(ptr);
            }
            Self::do_wake(ptr);
            Self::drop_waker(ptr);
        })
    }

    /// Wakes a waker by reference
    unsafe fn wake_by_ref(ptr: *const ()) {
        dbg_context!(ptr, "wake_by_ref", {
            #[cfg(feature = "debugging")]
            {
                Self::check_waker_thread(ptr);
            }
            Self::do_wake(ptr);
        })
    }

    /// Drops a waker
    ///
    /// This function will decrement the reference count. if it drops to zero, the associated join
    /// handle has been dropped too, and the task has not been completed, then it will get
    /// scheduled one more time so that its future gets dropped by the executor.
    #[inline]
    unsafe fn drop_waker(ptr: *const ()) {
        dbg_context!(ptr, "drop_waker", {
            use super::state::DropWaker;

            #[cfg(feature = "debugging")]
            {
                Self::check_waker_thread(ptr);
            }
            let raw = Self::from_ptr(ptr);

            match (*raw.header.cast_mut()).state.drop_waker() {
                DropWaker::Destroy => Self::destroy(ptr),
                DropWaker::Schedule => Self::schedule(ptr, false),
                DropWaker::DoNothing => {}
            }
        })
    }

    /// Create a harness from a pointer to header
    ///
    /// # Safety
    ///
    /// Cannot be null
    pub(super) unsafe fn from_ptr(ptr: *const ()) -> Self {
        let task_layout = Self::task_layout();
        let p = ptr as *const u8;

        Self {
            header: p.cast(),
            scheduler: p.add(task_layout.offset_scheduler).cast(),
            future: p.add(task_layout.offset_future).cast::<F>().cast_mut(),
            output: p.add(task_layout.offset_output).cast::<R>().cast_mut(),
        }
    }

    /// Allocate a new `Harness<F, R, S>` and return a pointer to it
    pub(super) fn allocate(future: F, scheduler: S) -> NonNull<()> {
        let task_layout = abort_on_panic(Self::task_layout);

        unsafe {
            // Allocate enough space for the entire task
            let raw_task = match NonNull::new(alloc::alloc::alloc(task_layout.layout) as *mut ()) {
                None => abort(),
                Some(p) => p,
            };

            let raw = Self::from_ptr(raw_task.as_ptr());

            let header = Header::new(Self::vtable());
            (raw.header as *mut Header).write(header);
            raw.scheduler.cast_mut().write(scheduler);
            raw.future.write(future);

            if TaskDebugger::register(raw_task.as_ptr()) {
                dbg_context!(raw_task.as_ptr(), "allocate", {});
            }

            raw_task
        }
    }

    #[inline]
    fn task_layout() -> TaskLayout {
        match Self::TASK_LAYOUT {
            Some(tl) => tl,
            None => abort(),
        }
    }

    fn vtable() -> &'static TaskVTable {
        &TaskVTable {
            /// Schedules the task
            schedule: Self::schedule,
            run: Self::run,
            drop_future: Self::drop_future,
            get_output: Self::get_output,
            destroy: Self::destroy,
        }
    }

    unsafe fn schedule(ptr: *const (), yield_now: bool) {
        dbg_context!(ptr, "schedule", {
            let raw = Self::from_ptr(ptr);
            (*raw.header.cast_mut()).state.incr_ref();

            // If the schedule function has captured variables, create a temporary waker that prevents
            // the task from getting deallocated while the function is being invoked
            let guard = if mem::size_of::<S>() > 0 {
                Some(Waker::from_raw(Self::clone_waker(ptr)))
            } else {
                None
            };

            let task = Task::from_raw(NonNull::new_unchecked(ptr.cast_mut()));

            if yield_now {
                (*raw.scheduler).yield_now(task);
            } else {
                (*raw.scheduler).schedule(task)
            }

            drop(guard)
        })
    }

    unsafe fn run(ptr: *const ()) -> bool {
        struct Guard<F, R, S>(RawTask<F, R, S>)
        where
            F: Future<Output = R>,
            S: Schedule;

        impl<F, R, S> Drop for Guard<F, R, S>
        where
            F: Future<Output = R>,
            S: Schedule,
        {
            fn drop(&mut self) {
                let raw = self.0;
                let ptr = raw.header.cast();

                unsafe {
                    // Transition to closed
                    (*raw.header.cast_mut()).state.set_closed();

                    // Drop tasks future, and the task reference.
                    // The thread that closed the task didn't drop the future because it was
                    // running, so now it's our responsiblity to do so.
                    RawTask::<F, R, S>::drop_future(ptr);

                    // Notify the await that the future has been dropped.
                    (*raw.header.cast_mut()).notify(None);

                    RawTask::<F, R, S>::drop_task(ptr);
                }
            }
        }

        use super::state::TransitionToRunning;

        let raw = Self::from_ptr(ptr);
        match (*raw.header.cast_mut()).state.transition_to_running() {
            TransitionToRunning::Closed => {
                // Drop the future
                Self::drop_future(ptr);

                // Notify the awaiter that the future has been dropped.
                (*raw.header.cast_mut()).notify(None);

                // Drop the task reference
                Self::drop_task(ptr);

                return false;
            }
            TransitionToRunning::Running => {
                // Poll the future. If the future completes the output is written to the stage field.
                let waker_ref =
                    ManuallyDrop::new(Waker::from_raw(RawWaker::new(ptr, &Self::RAW_WAKER_VTABLE)));
                let cx = &mut Context::from_waker(&waker_ref);

                // Polls the inner future, but surround it with a guard that closes the task in
                // case polling panics
                let guard = Guard(raw);
                let poll = <F as Future>::poll(Pin::new_unchecked(&mut *raw.future), cx);
                mem::forget(guard);

                let mut ret = false;
                match poll {
                    Poll::Ready(out) => {
                        use super::state::TransitionToCompleted;

                        Self::drop_future(ptr);
                        raw.output.write(out);

                        // A place where the output will be stored in case it needs to be dropped.
                        let mut output = None;

                        match (*raw.header.cast_mut()).state.transition_to_completed() {
                            TransitionToCompleted::Closed => {
                                // The handle is dropped or the task was closed while running so do
                                // not care about the output. Drop it
                                output = Some(raw.output.read());
                            }
                            TransitionToCompleted::DoNothing => {}
                        }

                        // We have an associated waker so wake
                        (*raw.header.cast_mut()).notify(None);

                        drop(output)
                    }
                    Poll::Pending => {
                        // The task is still not completed
                        use super::state::TransitionToIdle;

                        match (*raw.header.cast_mut()).state.transition_to_idle() {
                            TransitionToIdle::Closed => {
                                // The task was closed while running, notify the awaiter
                                Self::drop_future(ptr);
                                (*raw.header.cast_mut()).notify(None);
                            }
                            TransitionToIdle::Schedule => {
                                // Was woken up while running, need to reschedule it
                                Self::schedule(ptr, false);
                                ret = true;
                            }
                            TransitionToIdle::DoNothing => {}
                        }
                    }
                }
                Self::drop_task(ptr);
                ret
            }
        }
    }

    /// Drops a task.
    ///
    /// This function will decrement the reference count. If it drops to zero and the associated
    /// join handle has been dropped too, then the task gets destroyed
    unsafe fn drop_task(ptr: *const ()) {
        dbg_context!(ptr, "drop_task", {
            let raw = Self::from_ptr(ptr);

            // If this was the last reference to the task and the `JoinHandle` has been dropped too,
            // then destroy the task
            if (*raw.header.cast_mut()).state.decr_ref() {
                Self::destroy(ptr);
            }
        })
    }

    /// Drops the future inside a task.
    #[inline]
    unsafe fn drop_future(ptr: *const ()) {
        let raw = Self::from_ptr(ptr);

        // We need a safeguard against panics because the destructor can panic
        abort_on_panic(|| {
            raw.future.drop_in_place();
        })
    }

    // Returns a pointer to the output inside a task.
    unsafe fn get_output(ptr: *const ()) -> *const () {
        let raw = Self::from_ptr(ptr);

        raw.output.cast()
    }

    // Drops a task.
    //
    // This function will decerement the reference count. If it drops to zero and the associated
    // join handle has been dropped too, then the task gets destroyed.

    #[inline]
    unsafe fn destroy(ptr: *const ()) {
        dbg_context!(ptr, "destroy", {
            TaskDebugger::unregister(ptr);

            let raw = Self::from_ptr(ptr);
            let task_layout = Self::task_layout();

            // Need a safeguard against panics because destructors can panic.
            abort_on_panic(|| {
                // Drop the schedule function
                (raw.scheduler.cast_mut()).drop_in_place();
            });

            alloc::alloc::dealloc(ptr.cast::<u8>().cast_mut(), task_layout.layout);
        })
    }
}
