use std::{
    borrow::BorrowMut,
    future::Future,
    io,
    pin::Pin,
    task::{ready, Context, Poll, Waker},
};

use io_uring::squeue;
use slotmap::{new_key_type, SlotMap};

use crate::driver::{self, uring::lifecycle};

use super::InnerHandle;

// In-flight operation
pub(crate) struct Op<T: 'static> {
    // Driver running the operation
    pub(super) driver: InnerHandle,

    // Operation key in the slab
    pub(super) key: OpsKey,

    // Per-operation data
    pub(super) data: Option<T>,
}

new_key_type! {
    /// Key for ops
    pub(crate) struct OpsKey;
}

pub(super) struct Ops {
    slot: SlotMap<OpsKey, Lifecycle>,
}

#[derive(Debug)]
pub(crate) struct Completion<T> {
    pub(crate) data: T,
    pub(crate) result: io::Result<u32>,
}

impl<T> Op<T> {
    /// Create a new operation
    pub(super) fn new(data: T, inner: &mut driver::UringInner, inner_rc: &InnerHandle) -> Op<T> {
        Op {
            driver: inner_rc.clone(),
            key: inner.ops.insert(),
            data: Some(data),
        }
    }

    /// Submit an operation to uring
    ///
    /// `state` is stored during the operation tracking any state submitted to the kernel.
    pub(super) fn submit_with<F>(data: T, f: F) -> io::Result<Op<T>>
    where
        F: FnOnce(&mut T) -> squeue::Entry,
    {
        super::CURRENT.with(|inner_rc| {
            let inner_ref = unsafe { &mut *inner_rc.get() };
            inner_ref.submit_with(inner_rc, data, f)
        })
    }
}

impl<T> Future for Op<T>
where
    T: Unpin + 'static,
{
    type Output = Completion<T>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> std::task::Poll<Self::Output> {
        use std::mem;

        let me = &mut *self;
        let data_mut = me.data.as_mut().expect("unexpected operation state");
        let inner = unsafe { &mut *me.driver.get() };
        let lifecycle = inner
            .ops
            .slot
            .get_mut(me.key)
            .expect("invalid internal state");
        match mem::replace(lifecycle, Lifecycle::Submitted) {
            Lifecycle::Submitted => {
                *lifecycle = Lifecycle::Waiting(cx.waker().clone());
                Poll::Pending
            }
            Lifecycle::Waiting(waker) if !waker.will_wake(cx.waker()) => {
                *lifecycle = Lifecycle::Waiting(cx.waker().clone());
                Poll::Pending
            }
            Lifecycle::Waiting(waker) => {
                *lifecycle = Lifecycle::Waiting(waker);
                Poll::Pending
            }
            Lifecycle::Ignored(..) => unreachable!(),
            Lifecycle::Completed(result, flags) => {
                inner.ops.remove(me.key);
                Poll::Ready(Completion {
                    data: me.data.take().expect("unexpected operation state"),,
                    result,
                    flags,
                })
            }
        }
    }
}

impl Ops {
    pub(super) fn new() -> Self {
        Ops {
            slot: SlotMap::with_capacity_and_key(64),
        }
    }

    pub(super) fn insert(&mut self) -> OpsKey {
        self.slot.insert(Lifecycle::Submitted)
    }

    pub(super) fn remove(&mut self, key: OpsKey) {
        self.slot.remove(key);
    }

    pub(super) fn len(&self) -> usize {
        self.slot.len()
    }

    pub(super) fn complete(&mut self, key: OpsKey, result: io::Result<u32>, flags: u32) {
        if self.slot[key].complete(result, flags) {
            self.slot.remove(key);
        }
    }
}

impl Drop for Ops {
    fn drop(&mut self) {
        assert!(self.slot.is_empty())
    }
}

pub(crate) enum Lifecycle {
    /// The operation has been submitted to uring and is currently in-flight
    Submitted,

    /// The submitter is waiting for the completion of the operation
    Waiting(Waker),

    /// The submitter no longer has interest in the operation result. The state must be passed to
    /// the driver and held until the operation completes
    Ignored(Box<dyn std::any::Any>),

    /// The operation has completed
    Completed(io::Result<u32>, u32),
}

impl Lifecycle {
    pub(super) fn complete(&mut self, result: io::Result<u32>, flags: u32) -> bool {
        match std::mem::replace(self, Lifecycle::Submitted) {
            Lifecycle::Submitted => {
                *self = Lifecycle::Completed(result, flags);
                false
            }
            Lifecycle::Waiting(waker) => {
                *self = Lifecycle::Completed(result, flags);
                waker.wake();
                false
            }
            Lifecycle::Ignored(..) => true,
            Lifecycle::Completed(..) => unreachable!("invalid operation state"),
        }
    }

    fn poll_op<T: 'static>(&mut self, cx: &mut Context<'_>) -> Poll<Completion<T>> {}
}
