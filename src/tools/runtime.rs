//! Helpers for quirks of the current tokio runtime.

use std::future::Future;

pub fn main<F, T>(fut: F) -> T
where
    F: Future<Output = T> + Send + 'static,
    T: std::fmt::Debug + Send + 'static,
{
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async {
        let (tx, rx) = tokio::sync::oneshot::channel();

        tokio::spawn(async move {
            tx.send(fut.await).unwrap()
        });

        rx.await.unwrap()
    })
}
