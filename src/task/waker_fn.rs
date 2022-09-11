use std::{
    cell::Cell,
    task::{RawWaker, RawWakerVTable, Waker},
};

pub(crate) fn dummy_waker() -> Waker {
    fn vtable() -> &'static RawWakerVTable {
        &RawWakerVTable::new(|_| raw_waker(), |_| set_poll(), |_| set_poll(), |_| {})
    }

    fn raw_waker() -> RawWaker {
        // the pointer is never derefenced so null is ok
        RawWaker::new(std::ptr::null::<()>(), vtable())
    }

    unsafe { Waker::from_raw(raw_waker()) }
}

thread_local! {pub(crate) static SHOULD_POLL: Cell<bool> = Cell::new(true)}

#[inline]
pub(crate) fn should_poll() -> bool {
    SHOULD_POLL.with(|b| b.replace(false))
}

pub(crate) fn set_poll() {
    SHOULD_POLL.with(|b| b.set(true))
}
