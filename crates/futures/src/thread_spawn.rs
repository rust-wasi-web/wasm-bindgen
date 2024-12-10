//! Spawning tasks on threads.

use std::any::Any;
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::{pin, Pin};
use std::task::{ready, Context, Poll};

use futures_channel::oneshot;
use futures_util::{select_biased, FutureExt};

use crate::spawn_local;
use crate::sync_wrapper::SyncWrapper;
use crate::thread::{thread_hold, thread_release};

/// Task failed to execute to completion.
pub struct JoinError(Repr);

enum Repr {
    /// Aborted.
    Aborted,
    /// Panicked.
    Panicked(SyncWrapper<Box<dyn Any + Send + 'static>>),
    /// Thread failed.
    Failed,
}

fn panic_payload_as_str(payload: &SyncWrapper<Box<dyn Any + Send>>) -> Option<&str> {
    if let Some(s) = payload.downcast_ref_sync::<String>() {
        return Some(s);
    }

    if let Some(s) = payload.downcast_ref_sync::<&'static str>() {
        return Some(s);
    }

    None
}

impl fmt::Debug for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.0 {
            Repr::Aborted => write!(f, "Aborted"),
            Repr::Panicked(p) => f
                .debug_tuple("Panicked")
                .field(&panic_payload_as_str(p).unwrap_or("..."))
                .finish(),
            Repr::Failed => write!(f, "Failed"),
        }
    }
}

impl fmt::Display for JoinError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match &self.0 {
            Repr::Aborted => write!(f, "task was cancelled"),
            Repr::Panicked(p) => match panic_payload_as_str(p) {
                Some(msg) => write!(f, "task panicked with message {msg}"),
                None => write!(f, "task panicked"),
            },
            Repr::Failed => write!(f, "task failed"),
        }
    }
}

impl Error for JoinError {}

impl JoinError {
    /// Returns true if the error was caused by the task being cancelled.
    pub fn is_cancelled(&self) -> bool {
        matches!(&self.0, Repr::Aborted)
    }

    /// Returns true if the error was caused by thread failure.
    pub fn is_failed(&self) -> bool {
        matches!(&self.0, Repr::Failed)
    }

    /// Returns true if the error was caused by the task panicking.
    pub fn is_panic(&self) -> bool {
        matches!(&self.0, Repr::Panicked(_))
    }

    /// Consumes the join error, returning the object with which the task panicked.
    #[track_caller]
    pub fn into_panic(self) -> Box<dyn Any + Send + 'static> {
        self.try_into_panic()
            .expect("`JoinError` reason is not a panic.")
    }

    /// Consumes the join error, returning the object with which the task
    /// panicked if the task terminated due to a panic. Otherwise, `self` is
    /// returned.
    pub fn try_into_panic(self) -> Result<Box<dyn Any + Send + 'static>, JoinError> {
        match self.0 {
            Repr::Panicked(p) => Ok(p.into_inner()),
            _ => Err(self),
        }
    }
}

/// An owned permission to join on a task.
pub struct JoinHandle<T> {
    result_rx: oneshot::Receiver<Result<T, JoinError>>,
    abort_tx: Option<oneshot::Sender<()>>,
}

impl<T> fmt::Debug for JoinHandle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_tuple("JoinHandle").finish()
    }
}

impl<T> Future for JoinHandle<T> {
    type Output = Result<T, JoinError>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let result_rx = unsafe { self.map_unchecked_mut(|this| &mut this.result_rx) };
        let result = ready!(result_rx.poll(cx)).unwrap_or(Err(JoinError(Repr::Failed)));
        Poll::Ready(result)
    }
}

impl<T> JoinHandle<T> {
    /// Abort the task associated with the handle.
    pub fn abort(&mut self) {
        if let Some(abort_tx) = self.abort_tx.take() {
            let _ = abort_tx.send(());
        }
    }
}

/// Spawns a `Future` on a new thread.
pub fn spawn_thread<T, F, U>(f: F) -> JoinHandle<T>
where
    F: FnOnce() -> U + Send + 'static,
    U: Future<Output = T> + 'static,
    T: Send + 'static,
{
    let (result_tx, result_rx) = oneshot::channel();
    let (abort_tx, mut abort_rx) = oneshot::channel();

    std::thread::spawn(move || {
        unsafe { thread_hold() };

        spawn_local(async move {
            let future = pin!(AssertUnwindSafe(f()).catch_unwind());
            let mut future = future.fuse();

            let res = loop {
                select_biased! {
                    res = &mut future => {
                        break res.map_err(|err| JoinError(Repr::Panicked(SyncWrapper::new(err))));
                    }
                    res = abort_rx => {
                        if res.is_ok() {
                            break Err(JoinError(Repr::Aborted));
                        }
                    }
                }
            };

            let _ = result_tx.send(res);
            unsafe { thread_release() };
        });
    });

    JoinHandle {
        result_rx,
        abort_tx: Some(abort_tx),
    }
}
