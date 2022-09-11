// impl Context {
//     /// Creates a new single-threaded executor
//     pub(crate) fn new() -> Self {
//         Self {
//             local_queue: LocalQueue::new(),
//         }
//     }
// 
//     fn spawn<T>(&self, future: impl Future<Output = T>) -> (Runnable, JoinHandle<T>) {
//         task_impl::spawn_local(future, LocalScheduler)
// 
//     }
// 
//     pub(crate) fn spawn_and_run<T>(&self, future: impl Future<Output = T>) -> Task<T> {
//         let (runnable, handle) = self.spawn(future);
//         runnable.run_right_away();
//         Task(Some(handle))
//     }
// 
// 
//     pub(crate) fn get_task(&self) -> Option<Runnable> {
//         self.local_queue.pop()
//     }
// 
//     pub(crate) fn is_active(&self) -> bool {
//         !self.local_queue.is_empty()
//     }
// 
//     pub(crate) fn run_task_queue(&self) {
//         let mut ran = false;
//     }
// 
// }
