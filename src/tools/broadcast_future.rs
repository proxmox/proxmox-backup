use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use anyhow::{format_err, Error};
use futures::future::{FutureExt, TryFutureExt};
use tokio::sync::oneshot;

/// Broadcast results to registered listeners using asnyc oneshot channels
#[derive(Default)]
pub struct BroadcastData<T> {
    result: Option<Result<T, String>>,
    listeners: Vec<oneshot::Sender<Result<T, Error>>>,
}

impl <T: Clone> BroadcastData<T> {

    pub fn new() -> Self {
        Self {
            result: None,
            listeners: vec![],
        }
    }

    pub fn notify_listeners(&mut self, result: Result<T, String>) {

        self.result = Some(result.clone());

        loop {
            match self.listeners.pop() {
                None => { break; },
                Some(ch) => {
                    match &result {
                        Ok(result) => { let _ = ch.send(Ok(result.clone())); },
                        Err(err) => { let _ = ch.send(Err(format_err!("{}", err))); },
                    }
                },
            }
        }
    }

    pub fn listen(&mut self) -> impl Future<Output = Result<T, Error>> {
        use futures::future::{ok, Either};

        match &self.result {
            None => {},
            Some(Ok(result)) => return Either::Left(ok(result.clone())),
            Some(Err(err)) => return Either::Left(futures::future::err(format_err!("{}", err))),
        }

        let (tx, rx) = oneshot::channel::<Result<T, Error>>();

        self.listeners.push(tx);

        Either::Right(rx
            .map(|res| match res {
                Ok(Ok(t)) => Ok(t),
                Ok(Err(e)) => Err(e),
                Err(e) => Err(Error::from(e)),
            })
        )
    }
}

/// Broadcast future results to registered listeners
pub struct BroadcastFuture<T> {
    inner: Arc<
        Mutex<(
            BroadcastData<T>,
            Option<Pin<Box<dyn Future<Output = Result<T, Error>> + Send>>>,
        )>,
    >,
}

impl<T: Clone + Send + 'static> BroadcastFuture<T> {
    /// Create instance for specified source future.
    ///
    /// The result of the future is sent to all registered listeners.
    pub fn new(source: Box<dyn Future<Output = Result<T, Error>> + Send>) -> Self {
        Self { inner: Arc::new(Mutex::new((BroadcastData::new(), Some(Pin::from(source))))) }
    }

    /// Creates a new instance with a oneshot channel as trigger
    pub fn new_oneshot() -> (Self, oneshot::Sender<Result<T, Error>>) {

        let (tx, rx) = oneshot::channel::<Result<T, Error>>();
        let rx = rx
            .map_err(Error::from)
            .and_then(futures::future::ready);

        (Self::new(Box::new(rx)), tx)
    }

    fn notify_listeners(
        inner: Arc<
            Mutex<(
                BroadcastData<T>,
                Option<Pin<Box<dyn Future<Output = Result<T, Error>> + Send>>>,
            )>,
        >,
        result: Result<T, String>,
    ) {
        let mut data = inner.lock().unwrap();
        data.0.notify_listeners(result);
    }

    fn spawn(
        inner: Arc<
            Mutex<(
                BroadcastData<T>,
                Option<Pin<Box<dyn Future<Output = Result<T, Error>> + Send>>>,
            )>,
        >,
    ) -> impl Future<Output = Result<T, Error>> {
        let mut data = inner.lock().unwrap();

        if let Some(source) = data.1.take() {

            let inner1 = inner.clone();

            let task = source.map(move |value| {
                match value {
                    Ok(value) => Self::notify_listeners(inner1, Ok(value.clone())),
                    Err(err) => Self::notify_listeners(inner1, Err(err.to_string())),
                }
            });
            tokio::spawn(task);
        }

        data.0.listen()
    }

    /// Register a listener
    pub fn listen(&self) -> impl Future<Output = Result<T, Error>> {
        let inner2 = self.inner.clone();
        async move { Self::spawn(inner2).await }
    }
}

#[test]
fn test_broadcast_future() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CHECKSUM: AtomicUsize  = AtomicUsize::new(0);

    let (sender, trigger) = BroadcastFuture::new_oneshot();

    let receiver1 = sender.listen()
        .map_ok(|res| {
            CHECKSUM.fetch_add(res, Ordering::SeqCst);
        })
        .map_err(|err| { panic!("got error {}", err); })
        .map(|_| ());

    let receiver2 = sender.listen()
        .map_ok(|res| {
            CHECKSUM.fetch_add(res*2, Ordering::SeqCst);
        })
        .map_err(|err| { panic!("got error {}", err); })
        .map(|_| ());

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(async move {
        let r1 = tokio::spawn(receiver1);
        let r2 = tokio::spawn(receiver2);

        trigger.send(Ok(1)).unwrap();
        let _ = r1.await;
        let _ = r2.await;
    });

    let result = CHECKSUM.load(Ordering::SeqCst);

    assert_eq!(result, 3);
}
