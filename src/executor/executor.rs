use std::{
    future::Future,
    io,
    panic::{RefUnwindSafe, UnwindSafe},
    task::Poll,
};

use crate::{
    driver::IoUringDriver,
    pin,
    task::{
        task_impl,
        waker_fn::{dummy_waker, set_poll, should_poll},
        JoinHandle,
    },
};

use super::{queue::LocalQueue, scheduler::LocalScheduler, task::Task};

scoped_tls::scoped_thread_local!(pub(crate) static LOCAL_EX: LocalExecutor);

pub struct ExecutorProxy {}

pub struct LocalExecutor {
    pub(super) local_queue: LocalQueue,
    driver: IoUringDriver,

    #[cfg(feature = "debugging")]
    thread_id: usize,
}

impl ExecutorProxy {
    pub fn spawn_local<T>(&self, future: impl Future<Output = T> + 'static) -> Task<T>
    where
        T: 'static,
    {
        return LOCAL_EX.with(|local_ex| Task::<T>(local_ex.spawn_await(future)));
    }
}

pub fn executor() -> ExecutorProxy {
    ExecutorProxy {}
}

impl UnwindSafe for LocalExecutor {}

impl RefUnwindSafe for LocalExecutor {}

impl LocalExecutor {
    pub fn new() -> io::Result<Self> {
        let driver = IoUringDriver::new(None)?;
        let local_queue = LocalQueue::new();
        #[cfg(feature = "debugging")]
        let thread_id = crate::utils::thread_id::gen_id();

        Ok(Self {
            local_queue,
            driver,
            #[cfg(feature = "debugging")]
            thread_id,
        })
    }

    #[cfg(feature = "debugging")]
    pub(crate) fn thread_id(&self) -> usize {
        self.thread_id
    }

    fn spawn_await<T>(&self, future: impl Future<Output = T>) -> JoinHandle<T> {
        let handle = task_impl::spawn_local(future, LocalScheduler);
        handle
    }

    pub fn run<T>(&mut self, future: impl Future<Output = T>) -> T {
        let run = |this: &Self| {
            let waker = dummy_waker();
            let cx = &mut std::task::Context::from_waker(&waker);

            //
            let join = future;
            pin!(join);
            set_poll();
            loop {
                loop {
                    // Consume all tasks(with max round to prevent io starvation)
                    let mut max_round = this.local_queue.len() * 2;
                    while let Some(t) = this.local_queue.pop() {
                        t.run();
                        if max_round == 0 {
                            // maybe there's a looping task
                            break;
                        } else {
                            max_round -= 1;
                        }
                    }

                    if should_poll() {
                        if let Poll::Ready(t) = join.as_mut().poll(cx) {
                            return t;
                        }
                    }

                    if this.local_queue.is_empty() {
                        // No task to execute, we should wait for io blockingly
                        // Hot path
                        break;
                    }

                    // Cold path
                    let _ = this.driver.submit();
                }

                let _ = this.driver.park();
            }
        };

        assert!(
            !LOCAL_EX.is_set(),
            "There is already a LocalExecutor running on this thread."
        );

        LOCAL_EX.set(self, || run(self))
    }
}

#[cfg(test)]
mod test {
    use std::{
        future::{self, Future, IntoFuture},
        pin::Pin,
        task::{Context, Poll, Waker},
        time::Duration,
    };

    use futures_concurrency::{Join, Race};

    use crate::{
        executor::task::FutureExt,
        task::{debugging::TaskDebugger, waker_fn::dummy_waker, yield_now::yield_now},
    };

    use super::LocalExecutor;
    fn init() {
        let _ = env_logger::builder()
            .format_timestamp(None)
            .filter_level(log::LevelFilter::Debug)
            .is_test(true)
            .try_init();
    }

    #[test]
    fn test_executor() {
        init();

        let mut ex = LocalExecutor::new().unwrap();

        let test = ex.run(async { 2 + 2 });
        assert_eq!(test, 4)
    }

    async fn testing() -> usize {
        TaskDebugger::set_label("subtask-1");
        let task4 = async { 6 + 6 }.spawn();
        TaskDebugger::set_label("subtask-2");
        let task3 = async { 3 + 3 }.spawn();
        yield_now().await;
        println!("testing1");
        task4.await;
        println!("testing2");
        task3.await;
        println!("testing3");

        3 + 9
    }

    #[test]
    fn test_task() {
        use std::future::Future;
        init();

        let mut ex = LocalExecutor::new().unwrap();
        let test1 = ex.run(async {
            TaskDebugger::set_label("task");
            let test = async {
                TaskDebugger::set_label("subtask");
                let test3 = async {
                    3 + 3;
                    yield_now();
                    println!("waiting");
                }
                .spawn()
                .await;
            }
            .spawn()
            .await;
        });
    }
}
