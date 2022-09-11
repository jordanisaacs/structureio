use std::{
    io,
    ops::ControlFlow,
    task::{Context, Poll, Waker},
};

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
