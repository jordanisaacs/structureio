use std::{cell::RefCell, collections::HashMap, time::Instant};

use super::header::Header;

thread_local! {
    static DEBUGGER: RefCell<Option<TaskDebugger>> = RefCell::new(None)
}

pub struct TaskDebugger {
    label: Option<&'static str>,
    registry: HashMap<*const (), TaskInfo>,
    filter: fn(Option<&'static str>) -> bool,
    task_count: usize,
    current_task: Option<*const ()>,
    context: Vec<&'static str>,
}

#[derive(Debug)]
struct TaskInfo {
    ptr: *const (),
    label: Option<&'static str>,
    ts: Instant,
}

fn has_label(label: Option<&'static str>) -> bool {
    label.is_some()
}

impl TaskInfo {
    fn new(ptr: *const (), label: Option<&'static str>) -> Self {
        Self {
            ptr,
            label,
            ts: Instant::now(),
        }
    }
}

impl TaskDebugger {
    fn with<F, R>(f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        DEBUGGER.with(|dbg| {
            let mut dbg = dbg.borrow_mut();
            if dbg.is_none() {
                *dbg = Some(TaskDebugger {
                    label: None,
                    registry: HashMap::new(),
                    filter: has_label,
                    task_count: 0,
                    current_task: None,
                    context: Vec::new(),
                });
            }
            f(dbg.as_mut().unwrap())
        })
    }

    /// Set label for the next glommio task to be spawned. Labels are used for filtering tasks to
    /// inspect
    pub fn set_label(label: &'static str) {
        Self::with(|dbg| {
            dbg.label = Some(label);
        })
    }

    pub fn set_filter(filter: fn(Option<&str>) -> bool) {
        Self::with(|dbg| {
            dbg.filter = filter;
        });
    }

    pub fn debug_aged_taskes(older_than: std::time::Duration) {
        Self::with(|dbg| {
            let mut count = 0;
            for (_, v) in dbg.registry.iter() {
                let age = v.ts.elapsed();
                if age > older_than {
                    count += 1;
                    dbg.debug_task_info(v, format!("age: {:?}", age).as_str())
                }
            }
            if count > 0 {
                log::debug!("found {} tasks older than {:?}", count, older_than)
            }
        })
    }

    pub fn task_count() -> usize {
        Self::with(|dbg| dbg.task_count)
    }

    pub(super) fn register(ptr: *const ()) -> bool {
        Self::with(|dbg| {
            dbg.task_count += 1;
            let label = dbg.label.take();
            if (dbg.filter)(label) {
                dbg.registry.insert(ptr, TaskInfo::new(ptr, label));
                unsafe { (*ptr.cast::<Header>().cast_mut()).set_debugging(true) };
                true
            } else {
                false
            }
        })
    }

    pub(super) fn unregister(ptr: *const ()) {
        Self::with(|dbg| {
            dbg.task_count -= 1;
            dbg.registry.remove(&ptr).is_some()
        });
    }

    pub(super) fn leave() {
        Self::with(|dbg| {
            dbg.context.pop();
        });
    }

    pub(super) fn set_current_task(ptr: *const ()) {
        Self::with(|dbg| {
            dbg.current_task = Some(ptr);
        });
    }

    pub(super) fn enter(ptr: *const (), ctx: &'static str) -> bool {
        Self::with(|dbg| {
            if let Some(info) = dbg.registry.get(&ptr) {
                dbg.context.push(ctx);
                dbg.debug_task(info, "");
                true
            } else {
                false
            }
        })
    }

    fn debug_task(&self, info: &TaskInfo, msg: &str) {
        self.debug_task_info(info, msg);
    }

    fn debug_task_info(&self, info: &TaskInfo, msg: &str) {
        let header = unsafe { &*(info.ptr as *const Header) };
        log::debug!(
            "[{:?}][{}][label:{}][{}]{}",
            info.ptr,
            header.to_compact_string(),
            info.label.unwrap_or(""),
            self.context.join("|"),
            msg
        )
    }
}
