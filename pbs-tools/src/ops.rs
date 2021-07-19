//! std::ops extensions

/// Modeled after the nightly `std::ops::ControlFlow`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ControlFlow<B, C = ()> {
    Continue(C),
    Break(B),
}

impl<B> ControlFlow<B> {
    pub const CONTINUE: ControlFlow<B, ()> = ControlFlow::Continue(());
}
