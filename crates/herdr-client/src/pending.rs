//! `Pending<R>`: the answer to one call, waited on by a worker thread (`wait`), polled from a UI
//! frame (`try_take`), or awaited on whatever executor the caller has (`Future`).

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use crate::error::Error;

type Outcome<R> = Result<R, Error>;

struct Slot<R> {
    state: Mutex<State<R>>,
    ready: Condvar,
}

struct State<R> {
    value: Option<Outcome<R>>,
    done: bool,
    waker: Option<Waker>,
}

impl<R> Slot<R> {
    fn lock(&self) -> MutexGuard<'_, State<R>> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// A call's answer, once it comes. Dropping it discards the answer (an attach stream in it
/// detaches).
#[must_use = "a call's answer is only seen through its Pending"]
pub struct Pending<R> {
    slot: Arc<Slot<R>>,
}

/// The connection's side of a `Pending`. Dropping it unanswered answers `Lost`.
pub(crate) struct Completer<R> {
    slot: Option<Arc<Slot<R>>>,
}

pub(crate) fn pending<R>() -> (Pending<R>, Completer<R>) {
    let slot = Arc::new(Slot {
        state: Mutex::new(State {
            value: None,
            done: false,
            waker: None,
        }),
        ready: Condvar::new(),
    });
    (
        Pending { slot: slot.clone() },
        Completer { slot: Some(slot) },
    )
}

impl<R> Pending<R> {
    /// One that is already answered.
    pub fn ready(outcome: Result<R, Error>) -> Self {
        let (pending, completer) = pending();
        completer.complete(outcome);
        pending
    }

    /// Whether the answer is in (and not yet taken).
    pub fn is_ready(&self) -> bool {
        self.slot.lock().value.is_some()
    }

    /// The answer if it is in. Each answer is handed out once.
    pub fn try_take(&mut self) -> Option<Result<R, Error>> {
        self.slot.lock().value.take()
    }

    /// Blocks until the answer is in. Never on the UI thread. Calls time out on their own (10 s
    /// by default), so this ends.
    pub fn wait(self) -> Result<R, Error> {
        let mut state = self.slot.lock();
        loop {
            if let Some(value) = state.value.take() {
                return value;
            }
            if state.done {
                return Err(Error::Lost);
            }
            state = self
                .slot
                .ready
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
    }

    /// Blocks up to `timeout`; `None` if the answer isn't in by then.
    pub fn wait_timeout(&mut self, timeout: Duration) -> Option<Result<R, Error>> {
        let deadline = Instant::now() + timeout;
        let mut state = self.slot.lock();
        loop {
            if let Some(value) = state.value.take() {
                return Some(value);
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            if state.done || left.is_zero() {
                return None;
            }
            state = self
                .slot
                .ready
                .wait_timeout(state, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }
}

impl<R> Future for Pending<R> {
    type Output = Result<R, Error>;

    /// Panics if polled again after it returned `Ready`.
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.slot.lock();
        if let Some(value) = state.value.take() {
            return Poll::Ready(value);
        }
        assert!(!state.done, "Pending polled after it was ready");
        state.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

impl<R> Completer<R> {
    pub(crate) fn complete(mut self, outcome: Result<R, Error>) {
        self.fill(outcome);
    }

    fn fill(&mut self, outcome: Result<R, Error>) {
        let Some(slot) = self.slot.take() else { return };
        let waker = {
            let mut state = slot.lock();
            state.value = Some(outcome);
            state.done = true;
            state.waker.take()
        };
        slot.ready.notify_all();
        if let Some(waker) = waker {
            waker.wake();
        }
    }
}

impl<R> Drop for Completer<R> {
    fn drop(&mut self) {
        self.fill(Err(Error::Lost));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::Wake;

    #[test]
    fn wait_gets_an_answer_from_another_thread() {
        let (pending, completer) = pending::<u32>();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(20));
            completer.complete(Ok(7));
        });
        assert_eq!(pending.wait().unwrap(), 7);
    }

    #[test]
    fn try_take_hands_the_answer_out_once() {
        let (mut pending, completer) = pending::<u32>();
        assert!(!pending.is_ready());
        assert!(pending.try_take().is_none());
        completer.complete(Err(Error::Closed));
        assert!(pending.is_ready());
        assert!(matches!(pending.try_take(), Some(Err(Error::Closed))));
        assert!(pending.try_take().is_none());
    }

    #[test]
    fn an_abandoned_completer_answers_lost() {
        let (pending, completer) = pending::<u32>();
        drop(completer);
        assert!(matches!(pending.wait(), Err(Error::Lost)));
    }

    #[test]
    fn wait_timeout_gives_up() {
        let (mut pending, _completer) = pending::<u32>();
        let start = Instant::now();
        assert!(pending.wait_timeout(Duration::from_millis(30)).is_none());
        assert!(start.elapsed() >= Duration::from_millis(30));
        assert!(matches!(
            Pending::ready(Ok(1u8)).wait_timeout(Duration::ZERO),
            Some(Ok(1))
        ));
    }

    struct Count(AtomicUsize);
    impl Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn as_a_future_it_wakes_its_task() {
        let count = Arc::new(Count(AtomicUsize::new(0)));
        let waker = Waker::from(count.clone());
        let mut cx = Context::from_waker(&waker);
        let (mut pending, completer) = pending::<&str>();
        assert!(Pin::new(&mut pending).poll(&mut cx).is_pending());
        completer.complete(Ok("hi"));
        assert_eq!(count.0.load(Ordering::SeqCst), 1);
        assert!(matches!(
            Pin::new(&mut pending).poll(&mut cx),
            Poll::Ready(Ok("hi"))
        ));
    }
}
