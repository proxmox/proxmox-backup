use failure::*;
use std::sync::{Mutex, Arc};

use futures::*;
use tokio::sync::oneshot;

struct BroadcastData<T> {
    result: Option<Result<T, String>>,
    listeners: Vec<oneshot::Sender<Result<T, Error>>>,
    source: Option<Box<Future<Item=T, Error=Error> + Send >>,
}

/// Broadcast future results to registered listeners
pub struct BroadcastFuture<T> {
    inner: Arc<Mutex<BroadcastData<T>>>,
}

impl <T: Clone + Send + 'static> BroadcastFuture<T> {

    /// Create instance for specified source future.
    ///
    /// The result of the future is sent to all registered listeners.
    pub fn new(source: Box<Future<Item=T, Error=Error> + Send>) -> Self {
        let data = BroadcastData {
            result: None,
            listeners: vec![],
            source: Some(source),
        };
        Self { inner: Arc::new(Mutex::new(data)) }
    }

    /// Creates a new instance with a oneshot channel as trigger
    pub fn new_oneshot() -> (Self, oneshot::Sender<Result<T, Error>>) {

        let (tx, rx) = oneshot::channel::<Result<T, Error>>();
        let rx = rx.map_err(Error::from).flatten();
        let test = Box::new(rx);

        (Self::new(test), tx)
    }

    fn update(inner: Arc<Mutex<BroadcastData<T>>>, result: Result<T, String>) {
        let mut data = inner.lock().unwrap();

        data.result = Some(result.clone());

        loop {
            match data.listeners.pop() {
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

    fn spawn(inner: Arc<Mutex<BroadcastData<T>>>) -> impl Future<Item=T, Error=Error> {

        let mut data = inner.lock().unwrap();

        match &data.result {
            None => {},
            Some(Ok(result)) => return futures::future::Either::A(futures::future::ok(result.clone())),
            Some(Err(err)) => return futures::future::Either::A(futures::future::err(format_err!("{}", err))),
        }

        let (tx, rx) = oneshot::channel::<Result<T, Error>>();

        data.listeners.push(tx);

        if let Some(source) = data.source.take() {

            let inner1 = inner.clone();

            let task = source.then(move |value| {
                match value {
                    Ok(value) => Self::update(inner1, Ok(value.clone())),
                    Err(err) => Self::update(inner1, Err(err.to_string())),
                }
                Ok(())
            });
            tokio::spawn(task);
        }

        futures::future::Either::B(rx.map_err(Error::from).and_then(|result| { result }))
    }

    /// Register a listener
    pub fn listen(&self) -> impl Future<Item=T, Error=Error> {
        let inner2 = self.inner.clone();
        futures::future::lazy(move || { Self::spawn(inner2) })
    }
}

#[test]
fn test_broadcast_future() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    static CHECKSUM: AtomicUsize  = AtomicUsize::new(0);

    let (sender, trigger) = BroadcastFuture::new_oneshot();

    let receiver1 = sender.listen()
        .and_then(|res| {
            CHECKSUM.fetch_add(res, Ordering::SeqCst);
            Ok(())
        })
        .map_err(|err| { panic!("got errror {}", err); });

    let receiver2 = sender.listen()
        .and_then(|res| {
            CHECKSUM.fetch_add(res*2, Ordering::SeqCst);
            Ok(())
        })
        .map_err(|err| { panic!("got errror {}", err); });

    tokio::run(futures::future::lazy(move || {

        tokio::spawn(receiver1);
        tokio::spawn(receiver2);

        trigger.send(Ok(1)).unwrap();

        Ok(())
    }));

    let result = CHECKSUM.load(Ordering::SeqCst);

    assert!(result == 3);
}
