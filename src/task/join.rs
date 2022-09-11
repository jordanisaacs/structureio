use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

use super::{dbg_context, header::Header, state::PollJoinHandle, task_impl::Task};

pub struct JoinHandle<R> {
    /// A raw task pointer.
    raw_task: NonNull<()>,

    /// A marker capturing generic types `R`.
    _marker: PhantomData<R>,
}

impl<R> Unpin for JoinHandle<R> {}

impl<R> JoinHandle<R> {}

impl<R> JoinHandle<R> {
    pub(super) unsafe fn from_raw(raw_task: NonNull<()>) -> Self {
        JoinHandle {
            raw_task,
            _marker: PhantomData,
        }
    }

    pub(crate) fn cancel(&self) {
        let ptr = self.raw_task.as_ptr();
        dbg_context!(ptr, "cancel", {
            let header = self.raw_task.as_ptr().cast::<Header>();
            unsafe {
                use super::state::TransitionToClosed;
                match (*header).state.transition_to_closed() {
                    TransitionToClosed::DoNothing => {}
                    TransitionToClosed::Closed(schedule) => {
                        if schedule {
                            ((*header).vtable.schedule)(ptr, false)
                        }

                        // Notify the awaiter that the task has been closed
                        (*header).notify(None);
                    }
                }
            }
        })
    }

    unsafe fn poll_inner(ptr: NonNull<()>, cx: &mut Context<'_>) -> Poll<Option<R>> {
        dbg_context!(ptr.as_ptr(), "poll_join_handle", {
            let header = ptr.as_ptr().cast::<Header>();
            match (*header).state.poll_join_handle() {
                PollJoinHandle::Start => {
                    let runnable = Task::from_raw(ptr);
                    (*header).register(cx.waker());
                    runnable.schedule();
                    Poll::Pending
                }
                PollJoinHandle::Register => {
                    (*header).register(cx.waker());
                    Poll::Pending
                }
                PollJoinHandle::Complete(closed) => {
                    // Notify the awaiter. Even though the awaiter is most likely the current task, it
                    // could also be another task
                    (*header).notify(Some(cx.waker()));
                    if closed {
                        // Task was closed so return `None`
                        Poll::Ready(None)
                    } else {
                        // Take the output from the task
                        let output = ((*header).vtable.get_output)(ptr.as_ptr()).cast::<R>();
                        Poll::Ready(Some(output.read()))
                    }
                }
            }
        })
    }
}

impl<R> Future for JoinHandle<R> {
    type Output = Option<R>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { Self::poll_inner(self.raw_task, cx) }
    }
}

impl<R> Drop for JoinHandle<R> {
    fn drop(&mut self) {
        let ptr = self.raw_task.as_ptr();
        dbg_context!(ptr, "drop_join_handle", {
            let header = ptr as *mut Header;

            // A place where the output will be stored in case it needs to be dropped.
            let mut output = None;

            unsafe {
                use super::state::DropJoinHandle;
                match (*header).state.drop_join_handle() {
                    DropJoinHandle::Close => {
                        // Notify the awaiter that the task has been closed
                        (*header).notify(None);
                    }
                    DropJoinHandle::DoNothing => {}
                    DropJoinHandle::Destroy => ((*header).vtable.destroy)(ptr),
                    DropJoinHandle::Schedule => ((*header).vtable.schedule)(ptr, false),
                    DropJoinHandle::DropOutput(destroy) => {
                        output = Some(((*header).vtable.get_output)(ptr).cast::<R>().read());
                        if destroy {
                            ((*header).vtable.destroy)(ptr)
                        }
                    }
                }
            }
            drop(output);
        })
    }
}
