use std::{
    pin::Pin,
    sync::{
        mpsc::{self, Receiver, Sender},
        Arc, Mutex, TryLockError,
    },
};

use atomic::{Atomic, Ordering};
use futures::{
    future::Future,
    task::{Context, Poll, Waker},
};
use pin_project::pin_project;

type WrappedWaker = Arc<Mutex<Option<Waker>>>;

/// The error type returned when the setter is dropped.
#[derive(Debug, thiserror::Error, PartialEq)]
#[error("Setter dropped without setting the flag")]
pub struct SetterDropped;

#[derive(Copy, Clone, PartialEq)]
enum State {
    NotSet,
    Set,
    Dropped,
}

struct SetterInner {
    f: Arc<Atomic<State>>,
    waiters: Receiver<WrappedWaker>,
}

/// The setting half of the flag.  Setting the flag will wake all `Waiter`'s.
pub struct Setter {
    i: Option<SetterInner>,
}

/// A cloneable waiter implementing `Future` with an `Output` type of `Result<(), SetterDropped>`
/// that will become ready when the associated `Setter` is set or dropped.
#[pin_project]
pub struct Waiter {
    f: Arc<Atomic<State>>,
    wait_sender: Sender<WrappedWaker>,
    waiter: Option<WrappedWaker>,
}

/// Create a `Setter`, `Waiter` pair.  The `Waiter` can be cloned any number of times.
pub fn flag() -> (Setter, Waiter) {
    let f = Arc::new(Atomic::new(State::NotSet));
    let (wait_sender, waiters) = mpsc::channel();
    (
        Setter {
            i: Some(SetterInner {
                f: f.clone(),
                waiters,
            }),
        },
        Waiter {
            f,
            wait_sender,
            waiter: None,
        },
    )
}

impl State {
    fn to_poll(self) -> Poll<Result<(), SetterDropped>> {
        match self {
            State::NotSet => Poll::Pending,
            State::Set => Poll::Ready(Ok(())),
            State::Dropped => Poll::Ready(Err(SetterDropped {})),
        }
    }
}

impl Setter {
    /// Set the flag and wake all `Waiter`s that are waiting on it.
    pub fn set(mut self) {
        self.i
            .take()
            .expect("Inner missing, should be impossible")
            .set_state(State::Set)
    }
}

impl SetterInner {
    fn set_state(self, state: State) {
        self.f.store(state, Ordering::Release);
        for waiter in self.waiters.try_iter() {
            match waiter.try_lock() {
                Ok(mut w) => w.take().expect("Empty option, should be impossible").wake(),
                Err(TryLockError::WouldBlock) => (), // They'll check state again before returning
                Err(TryLockError::Poisoned(_)) => panic!("Lock was poisoned, should be impossible"),
            }
        }
    }
}

impl Drop for Setter {
    fn drop(&mut self) {
        if let Some(i) = self.i.take() {
            i.set_state(State::Dropped)
        }
    }
}

impl Future for Waiter {
    type Output = Result<(), SetterDropped>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
        let this = self.project();
        match this.f.load(Ordering::Acquire).to_poll() {
            Poll::Ready(r) => return Poll::Ready(r),
            Poll::Pending => (),
        }

        if let Some(waiter) = this.waiter {
            match waiter.try_lock() {
                Ok(mut w) => *w = Some(cx.waker().clone()),
                Err(TryLockError::WouldBlock) => (), // We've raced with the Setter, check the state again.
                Err(TryLockError::Poisoned(_)) => panic!("Lock was poisoned, should be impossible"),
            }
        } else {
            let waiter = Arc::new(Mutex::new(Some(cx.waker().clone())));
            *this.waiter = Some(waiter.clone());
            let _ = this.wait_sender.send(waiter);
        }

        this.f.load(Ordering::Acquire).to_poll()
    }
}

impl Clone for Waiter {
    fn clone(&self) -> Self {
        Waiter {
            f: self.f.clone(),
            wait_sender: self.wait_sender.clone(),
            waiter: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::time::Duration;

    use futures::pin_mut;

    #[tokio::test(core_threads = 4)]
    async fn test_simple() {
        let (set, wait) = flag();

        set.set();

        assert_eq!(wait.await, Ok(()));
    }

    #[tokio::test(core_threads = 4)]
    async fn test_dropped() {
        let (set, wait) = flag();

        drop(set);

        assert_eq!(wait.await, Err(SetterDropped {}));
    }

    #[tokio::test(core_threads = 4)]
    async fn test_multiple() {
        let (set, wait) = flag();

        let handles: Vec<_> = (0..10)
            .map(|_| {
                let w = wait.clone();
                tokio::spawn(async move { w.await.unwrap() })
            })
            .collect();

        tokio::time::delay_for(Duration::from_millis(100)).await;

        set.set();

        for h in handles.into_iter() {
            pin_mut!(h);
            h.await.unwrap()
        }

        assert_eq!(wait.await, Ok(()));
    }

    #[pin_project]
    struct AlwaysWake<T> {
        #[pin]
        t: T,
    }

    impl<T: Future> Future for AlwaysWake<T> {
        type Output = T::Output;
        fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Self::Output> {
            let this = self.project();
            let r = this.t.poll(cx);
            if r.is_pending() {
                cx.waker().wake_by_ref();
            }
            r
        }
    }

    #[tokio::test(core_threads = 4)]
    async fn test_racing() {
        for _ in 0..10 {
            let (set, wait) = flag();

            let handles: Vec<_> = (0..50)
                .map(|_| {
                    let w = wait.clone();
                    tokio::spawn(AlwaysWake {
                        t: async move { w.await.unwrap() },
                    })
                })
                .collect();

            tokio::time::delay_for(Duration::from_millis(10)).await;

            set.set();

            for h in handles.into_iter() {
                pin_mut!(h);
                h.await.unwrap()
            }

            assert_eq!(wait.await, Ok(()));
        }
    }
}
