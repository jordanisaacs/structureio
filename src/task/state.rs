use core::fmt;

use super::utils::abort;

/// Set if the task is running.
///
/// A task is in the running state while its future is being polled.
///
/// This flag can't be set when the task is completed. However, it can be in the scheduled state
/// while it is running, in which case it will be rescheduled as soon as polling finishes.
const RUNNING: usize = 1 << 0;

/// Set if the task is complete.
///
/// This flag is set when polling returns `Poll::Ready`. THe output of the future is then stored
/// inside the task until it becomes closed. In face [`JoinHandle`] picks up the output by marking
/// the task as closed.
///
/// Once set can't be unset. Can't be set when the task is scheduled or running.
const COMPLETED: usize = 1 << 1;

/// Set if the task is closed
///
/// If a task is closed, that means it's either canceled or its output has been consumed by the
/// [`JoinHandle]. A task becomes closed when:
///
/// 1. It gets canceled by [`Task::cancel()`], [`Task::drop()`], or [`JoinHandle::cancel()`].
/// 2. Its output gets awaited by the [`JoinHandle`].
/// 3. It panics while polling the future.
/// 4. It is completed and the [`JoinHandle`] gets dropped.
const CLOSED: usize = 1 << 2;

/// Extracts the task's lifecycle value from the state
const LIFECYCLE_MASK: usize = RUNNING | COMPLETED | CLOSED;

/// Set if the task is scheduled for running.
///
/// A task is considered to be scheduled whenever its [`Task`] reference exists. It therefore also
/// begins in a scheduled state at the moment of creation.
///
/// This flag can't be set when the task is completed. However, it can be set while the task is
/// running, in which case it will be rescheduled as soon as polling finishes.
const SCHEDULED: usize = 1 << 3;

const NEED_START: usize = 1 << 4;

/// All bits
const STATE_MASK: usize = LIFECYCLE_MASK | SCHEDULED | NEED_START;

/// Bits used by the refcount portion of the state
const REF_COUNT_MASK: usize = !STATE_MASK;

/// Number of positions to shift the ref count
const REF_COUNT_SHIFT: usize = REF_COUNT_MASK.count_zeros() as usize;

const REF_COUNT_MAX: usize = usize::MAX >> REF_COUNT_SHIFT;

/// One ref count
const REF_ONE: usize = 1 << REF_COUNT_SHIFT;

/// State a task is initialized with
///
/// As the task starts with a `JoinHandle`, it has a reference. When the `JoinHandle` is dropped,
/// the task is cancelled. No need to track the join interest.
const INITIAL_STATE: usize = REF_ONE | NEED_START;

#[must_use]
pub(super) enum TransitionToIdle {
    DoNothing,
    Closed,
    Schedule,
}

#[must_use]
pub(super) enum TransitionToScheduled {
    DoNothing,
    Submit,
}

#[must_use]
pub(super) enum TransitionToClosed {
    DoNothing,
    Closed(bool),
}

#[must_use]
pub(super) enum TransitionToRunning {
    Closed,
    Running,
}

#[must_use]
pub(super) enum TransitionToCompleted {
    Closed,
    DoNothing,
}

#[must_use]
pub(super) enum DropWaker {
    Destroy,
    Schedule,
    DoNothing,
}

#[must_use]
pub(super) enum DropJoinHandle {
    Destroy,
    DropOutput(bool),
    Schedule,
    DoNothing,
    Close,
}

#[must_use]
pub(super) enum PollJoinHandle {
    Start,
    Register,
    Complete(bool),
}

/// Current state value
#[derive(Copy, Clone)]
pub(crate) struct Snapshot(usize);

impl Snapshot {
    fn xor(&mut self, other: usize) {
        self.0 ^= other;
    }

    fn add(&mut self, other: usize) {
        self.0 += other
    }

    fn sub(&mut self, other: usize) {
        self.0 -= other
    }

    fn incr_ref(&mut self) {
        if self.ref_count() == REF_COUNT_MAX {
            abort()
        }

        self.add(REF_ONE);
    }

    fn decr_ref(&mut self) {
        debug_assert!(self.ref_count() >= 1);

        self.sub(REF_ONE);
    }

    // Returns `true` if the task is in an idle state
    // fn is_idle(self) -> bool {
    //     (self.0 & (RUNNING | COMPLETED | CLOSED)) == 0
    // }

    fn is_scheduled(self) -> bool {
        (self.0 & SCHEDULED) == SCHEDULED
    }

    fn unset_scheduled(&mut self) {
        self.0 &= !SCHEDULED
    }

    fn set_scheduled(&mut self) {
        self.0 |= SCHEDULED
    }

    fn is_running(self) -> bool {
        self.0 & RUNNING == RUNNING
    }

    fn set_running(&mut self) {
        self.0 |= RUNNING;
    }

    fn unset_running(&mut self) {
        self.0 &= !RUNNING;
    }

    /// Returns `true` if the task's future has completed execution.
    fn is_completed(self) -> bool {
        self.0 & COMPLETED == COMPLETED
    }

    fn need_start(self) -> bool {
        self.0 & NEED_START == NEED_START
    }

    fn unset_need_start(&mut self) {
        self.0 &= !NEED_START;
    }

    fn set_completed(&mut self) {
        self.0 |= COMPLETED
    }

    fn is_closed(self) -> bool {
        self.0 & CLOSED == CLOSED
    }

    fn set_closed(&mut self) {
        self.0 |= CLOSED;
    }

    fn ref_count(self) -> usize {
        (self.0 & REF_COUNT_MASK) >> REF_COUNT_SHIFT
    }

    // Sets the state but keeps the ref count
    fn set_state(&mut self, state: usize) {
        self.0 = (self.0 & REF_COUNT_MAX) & state
    }

    pub fn to_compact_string(&self) -> String {
        format!(
            "b:{}|s:{}|r:{}|c:{}|x:{}|refs:{}",
            self.need_start() as i32,
            self.is_scheduled() as i32,
            self.is_running() as i32,
            self.is_completed() as i32,
            self.is_closed() as i32,
            self.ref_count() as i32
        )
    }
}

pub(crate) struct State(usize);

impl std::ops::Deref for Snapshot {
    type Target = usize;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl State {
    pub(crate) fn new() -> Self {
        State(INITIAL_STATE)
    }

    // Loads the current state into a new Snapshot
    pub(crate) fn load(&self) -> Snapshot {
        Snapshot(self.0)
    }

    // Sets the current state to a Snapshot
    pub(crate) fn store(&mut self, val: Snapshot) {
        self.0 = *val
    }

    /// Transitions the lifecycle to `Running`.
    /// This sets the scheduled bit to false so notifications during the poll can be detected
    pub(super) fn transition_to_running(&mut self) -> TransitionToRunning {
        let mut snapshot = self.load();
        debug_assert!(!snapshot.need_start());
        snapshot.unset_scheduled();
        let action = if snapshot.is_closed() {
            TransitionToRunning::Closed
        } else {
            snapshot.set_running();
            TransitionToRunning::Running
        };
        self.store(snapshot);
        action
    }

    /// Transitions the task from `RUNNING` -> `IDLE`
    pub(super) fn transition_to_idle(&mut self) -> TransitionToIdle {
        let mut snapshot = self.load();
        debug_assert!(snapshot.is_running());
        snapshot.unset_running();
        let action = if snapshot.is_closed() {
            snapshot.unset_scheduled();
            TransitionToIdle::Closed
        } else if snapshot.is_scheduled() {
            // Thread that woke up the task didn't reschedule it because it was running, now must
            // do it
            TransitionToIdle::Schedule
        } else {
            TransitionToIdle::DoNothing
        };
        self.store(snapshot);
        action
    }

    /// Transitions the state to `SCHEDULED`.
    pub(super) fn transition_to_scheduled(&mut self) -> TransitionToScheduled {
        let mut snapshot = self.load();
        let action = if snapshot.is_completed() || snapshot.is_closed() || snapshot.is_scheduled() {
            // If task is already scheduled don't need to schedule it again
            // If task is completed or closed it can't be woken up, don't schedule it
            TransitionToScheduled::DoNothing
        } else {
            // Mark the task as scheduled
            snapshot.set_scheduled();
            if snapshot.is_running() {
                // Already running so do not need to schedule it
                TransitionToScheduled::DoNothing
            } else {
                // If it is not yet running, schedule
                TransitionToScheduled::Submit
            }
        };
        self.store(snapshot);
        action
    }

    /// Transitions the state to `COMPLETED`.
    pub(super) fn transition_to_completed(&mut self) -> TransitionToCompleted {
        let mut snapshot = self.load();

        snapshot.unset_running();
        snapshot.unset_scheduled();
        snapshot.set_completed();
        let action = if snapshot.ref_count() == 0 || snapshot.is_closed() {
            snapshot.set_closed();
            // It was closed while running
            TransitionToCompleted::Closed
        } else {
            // Do not need to do anything
            TransitionToCompleted::DoNothing
        };
        self.store(snapshot);

        action
    }

    pub(super) fn transition_to_closed(&mut self) -> TransitionToClosed {
        let mut snapshot = self.load();

        let action = if snapshot.need_start() {
            // If still needs to start, then nothing has ran so nothing needs to be done
            snapshot.set_closed();
            TransitionToClosed::DoNothing
        } else if snapshot.is_completed() || snapshot.is_closed() {
            // If completed or closed don't need to do anything
            TransitionToClosed::DoNothing
        } else if !snapshot.is_scheduled() && !snapshot.is_running() {
            // If it isn't running or schedule
            snapshot.set_scheduled();
            snapshot.set_closed();
            snapshot.incr_ref();
            TransitionToClosed::Closed(true)
        } else {
            snapshot.set_closed();
            TransitionToClosed::Closed(false)
        };
        self.store(snapshot);

        action
    }

    pub(super) fn drop_join_handle(&mut self) -> DropJoinHandle {
        let mut snapshot = self.load();

        // If it is the initial state, then can just immediately destroy it
        if snapshot.0 == INITIAL_STATE {
            return DropJoinHandle::Destroy;
        }

        snapshot.decr_ref();
        let action = match (
            snapshot.ref_count() == 0,
            snapshot.is_completed(),
            snapshot.is_closed(),
        ) {
            (destroy, true, false) => {
                // Task has been completed but not yet closed. Output must be dropped
                debug_assert!(!snapshot.is_scheduled() && !snapshot.is_running());
                snapshot.set_closed();
                DropJoinHandle::DropOutput(destroy)
            }
            (true, false, false) => {
                // It is the last reference to the task, and it is not closed or completed.
                // Thus need to schedule it one more time so that its future gets dropped
                debug_assert!(!snapshot.is_scheduled() && !snapshot.is_running());
                snapshot.set_state(SCHEDULED | CLOSED);
                snapshot.incr_ref();
                DropJoinHandle::Schedule
            }
            (true, _, true) => {
                // If it is closed and the last reference we can destroy.
                // Must not be scheduled/running as last reference.
                debug_assert!(!snapshot.is_scheduled() && !snapshot.is_running());
                DropJoinHandle::Destroy
            }
            (false, false, false) => {
                // It isn't the last reference, it isn't completed, and it isn't closed. It must be
                // either scheduled or running. Set as closed and do nothing
                debug_assert!(snapshot.is_scheduled() || snapshot.is_running());
                snapshot.set_closed();
                DropJoinHandle::DoNothing
            }
            (_, _, _) => {
                // (false, false, true) => will never
                // (false, true, true) => will never have more references, when it is already
                // completed and closed. This is because it won't be scheduled or have wakers
                unreachable!("unreachable state");
            }
        };

        self.store(snapshot);
        action
    }

    /// Poll
    pub(super) fn poll_join_handle(&mut self) -> PollJoinHandle {
        let mut snapshot = self.load();

        let action = if snapshot.is_closed() {
            if snapshot.is_scheduled() || snapshot.is_running() {
                // Need to wait until its future is dropped
                debug_assert!(!snapshot.is_completed());
                debug_assert!(!snapshot.need_start());
                PollJoinHandle::Register
            } else {
                // If the task has been closed, notify the awaiter and there is no output
                debug_assert!(snapshot.is_completed() || snapshot.need_start());
                snapshot.unset_need_start();
                PollJoinHandle::Complete(true)
            }
        } else if snapshot.need_start() {
            snapshot.unset_need_start();
            PollJoinHandle::Start
        } else if !snapshot.is_completed() {
            debug_assert!(!snapshot.need_start());
            // If the task is not completed, register the current task to go again
            PollJoinHandle::Register
        } else {
            // Notify the awaiter, and get the output as it wasn't closed.
            // Even though the awaiter is most likely the current task, it
            // could also be another task
            debug_assert!(!snapshot.need_start());
            debug_assert!(snapshot.is_completed());
            snapshot.set_closed();
            PollJoinHandle::Complete(false)
        };

        self.store(snapshot);
        action
    }

    /// Sets the state to `CLOSED`. Only used during the run guard
    pub(super) fn set_closed(&mut self) {
        let mut val = self.load();
        val.unset_running();
        val.unset_scheduled();
        val.set_closed();
        self.store(val);
    }

    pub(crate) fn incr_ref(&mut self) {
        let mut val = self.load();
        let prev = *val;

        val.add(REF_ONE);
        self.store(val);

        // If the reference count overflowed, abort.
        if prev > isize::MAX as usize {
            abort()
        }
    }

    pub(super) fn cancel(&mut self) {
        let mut snapshot = self.load();
        if snapshot.is_completed() || snapshot.is_closed() {
            return;
        }

        snapshot.set_closed();
        snapshot.set_scheduled();
        self.store(snapshot);
    }

    pub(super) fn drop_waker(&mut self) -> DropWaker {
        let mut snapshot = self.load();

        snapshot.decr_ref();
        let action = if snapshot.ref_count() == 0 {
            // Last reference to task, need to decide how to estroy the task.
            if !snapshot.is_completed() && !snapshot.is_closed() {
                snapshot.set_state(SCHEDULED | CLOSED);
                if !snapshot.is_scheduled() {
                    // Task was not completed nor closed, close it and schedule one more time so
                    // that its future gets dropped by the executor.
                    DropWaker::Schedule
                } else {
                    DropWaker::DoNothing
                }
            } else {
                DropWaker::Destroy
            }
        } else {
            DropWaker::DoNothing
        };

        self.store(snapshot);
        action
    }

    pub(super) fn drop_task(&mut self) {
        let mut snapshot = self.load();

        snapshot.unset_scheduled();
        if !snapshot.is_completed() || !snapshot.is_closed() {
            snapshot.set_closed();
        }

        self.store(snapshot)
    }

    pub(super) fn decr_ref(&mut self) -> bool {
        let mut val = self.load();

        // Previous state
        debug_assert!(val.ref_count() >= 1);

        // New state
        val.sub(REF_ONE);
        self.store(val);

        val.ref_count() == 0
    }

    pub(super) fn to_compact_string(&self) -> String {
        let state = self.load();
        state.to_compact_string()
    }
}

impl fmt::Debug for State {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        let snapshot = self.load();
        snapshot.fmt(fmt)
    }
}

impl fmt::Debug for Snapshot {
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt.debug_struct("Snapshot")
            .field("is_running", &self.is_running())
            .field("is_complete", &self.is_completed())
            .field("is_scheduled", &self.is_scheduled())
            .field("ref_count", &self.ref_count())
            .finish()
    }
}
