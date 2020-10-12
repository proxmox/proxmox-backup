use anyhow::Error;

/// `WorkerTask` methods commonly used from contexts otherwise not related to the API server.
pub trait TaskState {
    /// If the task should be aborted, this should fail with a reasonable error message.
    fn check_abort(&self) -> Result<(), Error>;

    /// Create a log message for this task.
    fn log(&self, level: log::Level, message: &std::fmt::Arguments);
}

/// Convenience implementation:
impl<T: TaskState + ?Sized> TaskState for std::sync::Arc<T> {
    fn check_abort(&self) -> Result<(), Error> {
        <T as TaskState>::check_abort(&*self)
    }

    fn log(&self, level: log::Level, message: &std::fmt::Arguments) {
        <T as TaskState>::log(&*self, level, message)
    }
}

#[macro_export]
macro_rules! task_error {
    ($task:expr, $($fmt:tt)+) => {{
        $crate::task::TaskState::log(&*$task, log::Level::Error, &format_args!($($fmt)+))
    }};
}

#[macro_export]
macro_rules! task_warn {
    ($task:expr, $($fmt:tt)+) => {{
        $crate::task::TaskState::log(&*$task, log::Level::Warn, &format_args!($($fmt)+))
    }};
}

#[macro_export]
macro_rules! task_log {
    ($task:expr, $($fmt:tt)+) => {{
        $crate::task::TaskState::log(&*$task, log::Level::Info, &format_args!($($fmt)+))
    }};
}

#[macro_export]
macro_rules! task_debug {
    ($task:expr, $($fmt:tt)+) => {{
        $crate::task::TaskState::log(&*$task, log::Level::Debug, &format_args!($($fmt)+))
    }};
}

#[macro_export]
macro_rules! task_trace {
    ($task:expr, $($fmt:tt)+) => {{
        $crate::task::TaskState::log(&*$task, log::Level::Trace, &format_args!($($fmt)+))
    }};
}
