use crate::{executor::executor::LOCAL_EX, task::Schedule};

use super::Runnable;

pub(crate) struct LocalScheduler;

impl Schedule for LocalScheduler {
    fn schedule(&self, runnable: Runnable) {
        LOCAL_EX.with(|ex| ex.local_queue.push(runnable));
    }

    fn yield_now(&self, runnable: Runnable) {
        LOCAL_EX.with(|ex| ex.local_queue.push_front(runnable))
    }
}
