use alloc::rc::Rc;
use core::cell::UnsafeCell;
use slotmap::{Key, KeyData};

use std::{io, mem::ManuallyDrop, time::Duration};

use io_uring::{cqueue, opcode, squeue, types::Timespec, Builder, IoUring};
use lifecycle::Lifecycle;

use self::op::{Op, Ops, OpsKey};

use super::util::timespec;

mod lifecycle;
mod op;

scoped_tls::scoped_thread_local!(pub(super) static CURRENT: InnerHandle);

pub(crate) const CANCEL_USERDATA: u64 = u64::MAX;
pub(crate) const TIMEOUT_USERDATA: u64 = u64::MAX - 1;
pub(crate) const MIN_REVERSED_USERDATA: u64 = u64::MAX - 2;
pub(crate) struct UringInner {
    ops: Ops,
    uring: ManuallyDrop<IoUring>,
}

impl UringInner {
    fn tick(&mut self) {
        let mut cq = self.uring.completion();
        cq.sync();

        for cqe in cq {
            if cqe.user_data() <= MIN_REVERSED_USERDATA {
                // Result of a cancellation or timeout action. There isn't anything we
                // need to do here. We must wait for the CQE for the operation that was canceled.
                continue;
            }

            let key = OpsKey::from(KeyData::from_ffi(cqe.user_data()));
            self.ops.complete(key, resultify(&cqe), cqe.flags())
        }
    }

    fn submit(&mut self) -> io::Result<()> {
        loop {
            match self.uring.submit() {
                Ok(_) => {
                    self.uring.submission().sync();
                    return Ok(());
                }
                Err(ref e)
                    if e.kind() == io::ErrorKind::Other
                        || e.kind() == io::ErrorKind::ResourceBusy =>
                {
                    self.tick();
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub(super) fn submit_with<T, F>(
        &mut self,
        this: &InnerHandle,
        data: T,
        f: F,
    ) -> io::Result<Op<T>>
    where
        F: FnOnce(&mut T) -> squeue::Entry,
    {
        // If the submission queue is full, flush it to the kernel
        if self.uring.submission().is_full() {
            self.submit()?;
        }

        let mut op = Op::new(data, self, this);

        let data_mut = op.data.as_mut().unwrap();
        let sqe = f(data_mut).user_data(op.key.data().as_ffi());

        {
            let mut sq = self.uring.submission();

            // Push the new operation
            if unsafe { sq.push(&sqe).is_err() } {
                unimplemented!("when is this hit?");
            }
        }

        Ok(op)
    }
}

impl Drop for UringInner {
    fn drop(&mut self) {
        unsafe { ManuallyDrop::drop(&mut self.uring) }
    }
}

pub struct IoUringDriver {
    inner: InnerHandle,
    timespec: *mut Timespec,
}

pub(super) type InnerHandle = Rc<UnsafeCell<UringInner>>;

impl IoUringDriver {
    pub(crate) fn new(opts: Option<Builder>) -> io::Result<Self> {
        let uring = if let Some(opts) = opts {
            ManuallyDrop::new(opts.build(256)?)
        } else {
            ManuallyDrop::new(IoUring::new(256)?)
        };

        let inner = Rc::new(UnsafeCell::new(UringInner {
            ops: Ops::new(),
            uring,
        }));

        Ok(Self {
            inner,
            timespec: Box::leak(Box::new(Timespec::new())) as *mut Timespec,
        })
    }

    pub(crate) fn submit(&self) -> io::Result<()> {
        let inner = unsafe { &mut *self.inner.get() };
        inner.submit()?;
        inner.tick();
        Ok(())
    }

    pub(crate) fn park(&self) -> io::Result<()> {
        self.inner_park(None)
    }

    pub(crate) fn park_timeout(&self, duration: Duration) -> io::Result<()> {
        self.inner_park(Some(duration))
    }

    pub(crate) fn with<R>(&self, f: impl FnOnce() -> R) -> R {
        CURRENT.set(&self.inner, f)
    }

    fn wait(&self) -> io::Result<usize> {
        let inner = self.inner.get();
        unsafe { (*inner).uring.submit_and_wait(1) }
    }

    fn num_operations(&self) -> usize {
        let inner = self.inner.get();
        unsafe { (*inner).ops.len() }
    }

    fn flush_space(inner: &mut UringInner, need: usize) -> io::Result<()> {
        let sq = inner.uring.submission();
        debug_assert!(sq.capacity() >= need);
        if sq.len() + need > sq.capacity() {
            drop(sq);
            inner.submit()?;
        }
        Ok(())
    }

    fn install_timeout(&self, inner: &mut UringInner, duration: Duration) {
        let timespec = timespec(duration);
        unsafe {
            std::ptr::replace(self.timespec, timespec);
        }
        let entry = opcode::Timeout::new(self.timespec as *const Timespec)
            .build()
            .user_data(TIMEOUT_USERDATA);

        let mut sq = inner.uring.submission();
        let _ = unsafe { sq.push(&entry) };
    }

    fn inner_park(&self, timeout: Option<Duration>) -> io::Result<()> {
        let inner = unsafe { &mut *self.inner.get() };

        if let Some(duration) = timeout {
            // Alloc space
            Self::flush_space(inner, 1)?;
            self.install_timeout(inner, duration);

            // Submit and wait
            inner.uring.submit_and_wait(1)?;
        }

        // Process CQ
        inner.tick();
        Ok(())
    }
}

fn resultify(cqe: &cqueue::Entry) -> io::Result<u32> {
    let res = cqe.result();

    if res >= 0 {
        Ok(res as u32)
    } else {
        Err(io::Error::from_raw_os_error(-res))
    }
}
