use std::{cell::UnsafeCell, collections::VecDeque};

use super::Runnable;

#[derive(Debug)]
pub(super) struct LocalQueue {
    /// Local queue
    queue: UnsafeCell<VecDeque<Runnable>>,
}

impl Default for LocalQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl LocalQueue {
    pub(crate) fn new() -> Self {
        const DEFAULT_TASK_QUEUE_SIZE: usize = 4096;
        Self::new_with_capacity(DEFAULT_TASK_QUEUE_SIZE)
    }

    pub(crate) fn new_with_capacity(capacity: usize) -> Self {
        Self {
            queue: UnsafeCell::new(VecDeque::with_capacity(capacity)),
        }
    }

    pub(crate) fn len(&self) -> usize {
        unsafe { (*self.queue.get()).len() }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn push(&self, runnable: Runnable) {
        unsafe { (*self.queue.get()).push_back(runnable) }
    }

    pub(crate) fn push_front(&self, runnable: Runnable) {
        unsafe { (*self.queue.get()).push_front(runnable) }
    }

    pub(crate) fn pop(&self) -> Option<Runnable> {
        unsafe { (*self.queue.get()).pop_front() }
    }
}
