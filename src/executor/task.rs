use std::{
    future::{Future, IntoFuture},
    hash::BuildHasher,
    pin::Pin,
    task::{Context, Poll},
};

use crate::task::JoinHandle;

use super::executor::executor;

#[must_use = "tasks get don't get scheduled until awaited"]
pub struct Task<T>(pub(super) JoinHandle<T>);

impl<T> Task<T> {
    /// Cancels the task and waits for it to stop running
    pub async fn cancel(self) -> Option<T> {
        self.0.cancel();
        self.0.await
    }
}

impl<T> Future for Task<T> {
    type Output = T;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Pin::new(&mut self.0)
            .poll(cx)
            .map(|output| output.expect("task has failed"))
    }
}

pub trait FutureExt: Future + Sized + 'static {
    /// Spawns a task onto the current single-threaded executor.
    ///
    /// If called from a [`LocalExecutor`], the task is spawned onto it.
    /// Otherwise, this method panics.
    fn spawn(self) -> Task<<Self as Future>::Output>
where {
        spawn_local(self)
    }
}

impl<F> FutureExt for F where F: Future + 'static {}

/// Spawns a task onto the current single-threaded executor.
///
/// If called from a [`LocalExecutor`], the task is spawned onto it.
/// Otherwise, this method panics.
///
/// Proxy to [`ExecutorProxy::spawn_local`]
pub(crate) fn spawn_local<T>(future: impl Future<Output = T> + 'static) -> Task<T>
where
    T: 'static,
{
    executor().spawn_local(future)
}
