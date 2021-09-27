use anyhow::{bail, Error};

/// Worker task abstraction
///
/// A worker task is a long running task, which usually logs output into a separate file.
pub trait WorkerTaskContext: Send + Sync {

    /// Test if there was a request to abort the task.
    fn abort_requested(&self) -> bool;

    /// If the task should be aborted, this should fail with a reasonable error message.
    fn check_abort(&self) -> Result<(), Error> {
        if self.abort_requested() {
            bail!("abort requested - aborting task");
        }
        Ok(())
    }

    /// Test if there was a request to shutdown the server.
    fn shutdown_requested(&self) -> bool;


    /// This should fail with a reasonable error message if there was
    /// a request to shutdown the server.
    fn fail_on_shutdown(&self) -> Result<(), Error> {
        if self.shutdown_requested() {
            bail!("Server shutdown requested - aborting task");
        }
        Ok(())
    }

    /// Create a log message for this task.
    fn log(&self, level: log::Level, message: &std::fmt::Arguments);
}

/// Convenience implementation:
impl<T: WorkerTaskContext + ?Sized> WorkerTaskContext for std::sync::Arc<T> {
    fn abort_requested(&self) -> bool {
        <T as WorkerTaskContext>::abort_requested(&*self)
    }

    fn check_abort(&self) -> Result<(), Error> {
        <T as WorkerTaskContext>::check_abort(&*self)
    }

    fn shutdown_requested(&self) -> bool {
        <T as WorkerTaskContext>::shutdown_requested(&*self)
    }

    fn fail_on_shutdown(&self) -> Result<(), Error> {
        <T as WorkerTaskContext>::fail_on_shutdown(&*self)
    }

    fn log(&self, level: log::Level, message: &std::fmt::Arguments) {
        <T as WorkerTaskContext>::log(&*self, level, message)
    }
}

#[macro_export]
macro_rules! task_error {
    ($task:expr, $($fmt:tt)+) => {{
        $crate::task::WorkerTaskContext::log(&*$task, log::Level::Error, &format_args!($($fmt)+))
    }};
}

#[macro_export]
macro_rules! task_warn {
    ($task:expr, $($fmt:tt)+) => {{
        $crate::task::WorkerTaskContext::log(&*$task, log::Level::Warn, &format_args!($($fmt)+))
    }};
}

#[macro_export]
macro_rules! task_log {
    ($task:expr, $($fmt:tt)+) => {{
        $crate::task::WorkerTaskContext::log(&*$task, log::Level::Info, &format_args!($($fmt)+))
    }};
}

#[macro_export]
macro_rules! task_debug {
    ($task:expr, $($fmt:tt)+) => {{
        $crate::task::WorkerTaskContext::log(&*$task, log::Level::Debug, &format_args!($($fmt)+))
    }};
}

#[macro_export]
macro_rules! task_trace {
    ($task:expr, $($fmt:tt)+) => {{
        $crate::task::WorkerTaskContext::log(&*$task, log::Level::Trace, &format_args!($($fmt)+))
    }};
}
