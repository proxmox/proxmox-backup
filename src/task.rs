pub use pbs_datastore::task::TaskState;

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
