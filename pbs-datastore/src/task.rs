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
