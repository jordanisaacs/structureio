use std::sync::atomic::{AtomicUsize, Ordering};

use once_cell::sync::Lazy;

static ID_GEN: Lazy<AtomicUsize> = Lazy::new(|| AtomicUsize::new(1));

/// Generate thread id.
pub(crate) fn gen_id() -> usize {
    ID_GEN.fetch_add(1, Ordering::AcqRel)
}

pub(crate) fn get_current_thread_id() -> usize {
    crate::executor::LOCAL_EX.with(|ex| ex.thread_id())
}


