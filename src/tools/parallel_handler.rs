use std::thread::{JoinHandle};
use std::sync::{Arc, Mutex};
use crossbeam_channel::{bounded, Sender};
use anyhow::{format_err, Error};

/// A handle to send data toö the worker thread (implements clone)
pub struct SendHandle<I> {
    input: Sender<I>,
    abort: Arc<Mutex<Option<String>>>,
}

/// A thread pool which run the supplied closure
///
/// The send command sends data to the worker threads. If one handler
/// returns an error, we mark the channel as failed and it is no
/// longer possible to send data.
///
/// When done, the 'complete()' method needs to be called to check for
/// outstanding errors.
pub struct ParallelHandler<I> {
    handles: Vec<JoinHandle<()>>,
    name: String,
    input: SendHandle<I>,
}

impl <I: Send + Sync +'static> SendHandle<I> {

    /// Returns the first error happened, if any
    pub fn check_abort(&self) -> Result<(), Error> {
        let guard = self.abort.lock().unwrap();
        if let Some(err_msg) = &*guard {
            return Err(format_err!("{}", err_msg));
        }
        Ok(())
    }

    /// Send data to the worker threads
    pub fn send(&self, input: I) -> Result<(), Error> {
        self.check_abort()?;
        self.input.send(input)?;
        Ok(())
    }
}

impl <I> Clone for SendHandle<I> {
    fn clone(&self) -> Self {
        Self { input: self.input.clone(), abort: self.abort.clone() }
    }
}

impl <I: Send + Sync + 'static> ParallelHandler<I> {

    /// Create a new thread pool, each thread processing incoming data
    /// with 'handler_fn'.
    pub fn new<F>(
        name: &str,
        threads: usize,
        handler_fn: F,
    ) -> Self
        where F: Fn(I) -> Result<(), Error> + Send + Sync + Clone + 'static,
    {
        let mut handles = Vec::new();
        let (input_tx, input_rx) = bounded::<I>(threads);

        let abort = Arc::new(Mutex::new(None));

        for i in 0..threads {
            let input_rx = input_rx.clone();
            let abort = abort.clone();
            let handler_fn = handler_fn.clone();
            handles.push(
                std::thread::Builder::new()
                    .name(format!("{} ({})", name, i))
                    .spawn(move || {
                        loop {
                            let data = match input_rx.recv() {
                                Ok(data) => data,
                                Err(_) => return,
                            };
                            match (handler_fn)(data) {
                                Ok(()) => {},
                                Err(err) => {
                                    let mut guard = abort.lock().unwrap();
                                    if guard.is_none() {
                                        *guard = Some(err.to_string());
                                    }
                                }
                            }
                        }
                    })
                    .unwrap()
            );
        }
        Self {
            handles,
            name: name.to_string(),
            input: SendHandle {
                input: input_tx,
                abort,
            },
        }
    }

    /// Returns a cloneable channel to send data to the worker threads
    pub fn channel(&self) -> SendHandle<I> {
        self.input.clone()
    }

    /// Send data to the worker threads
    pub fn send(&self, input: I) -> Result<(), Error> {
        self.input.send(input)?;
        Ok(())
    }

    /// Wait for worker threads to complete and check for errors
    pub fn complete(self) -> Result<(), Error> {
        self.input.check_abort()?;
        drop(self.input);
        let mut msg = Vec::new();
        for (i, handle) in self.handles.into_iter().enumerate() {
            if let Err(panic) = handle.join() {
                match panic.downcast::<&str>() {
                    Ok(panic_msg) => msg.push(format!("thread {} ({}) paniced: {}", self.name, i, panic_msg)),
                    Err(_) => msg.push(format!("thread {} ({}) paniced", self.name, i)),
                }
            }
        }
        if msg.is_empty() {
            return Ok(());
        }
        Err(format_err!("{}", msg.join("\n")))
    }
}
