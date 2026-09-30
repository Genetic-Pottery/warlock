use std::ops::ControlFlow;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) enum Workers {
    /// On a thread of its own, which is what keeps the panel drawing while the
    /// work blocks. The `JoinHandle` is dropped on purpose: joining is waiting,
    /// and the thread exists so nobody waits for it.
    #[default]
    Threaded,
    /// To completion on the caller's thread before the spawn returns, so a test
    /// drains the result on the very next round with no clock involved. A panic
    /// is caught and closes the channel with nothing on it, exactly as a panic on
    /// a thread does, so it still reads as [`Lost`].
    #[cfg(test)]
    Inline,
}

/// A worker whose channel closed without its final word: every worker sends on
/// every path it takes, so this is a panic the hook has already printed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Lost;

#[derive(Debug)]
pub(crate) struct Once<T>(Receiver<T>);

#[derive(Debug)]
pub(crate) struct Stream<E>(Receiver<E>);

/// A failed send is ignored: a receiver that has gone away is an application
/// that is quitting, which is also what cancels the work.
#[derive(Debug)]
pub(crate) struct Port<E>(Sender<E>);

impl<E> Clone for Port<E> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<E> Port<E> {
    pub(crate) fn send(&self, event: E) {
        let _ = self.0.send(event);
    }
}

impl Workers {
    pub(crate) fn once<T: Send + 'static>(
        self,
        work: impl FnOnce() -> T + Send + 'static,
    ) -> Once<T> {
        let (sender, received) = mpsc::channel();
        let port = Port(sender);
        self.run(move || port.send(work()));
        Once(received)
    }

    /// `start` runs on the caller's thread with the port, so anything that must
    /// be wired to it before the work begins is wired there, and returns the work
    /// itself. The stream is over when the work drops the last port.
    pub(crate) fn stream<E, W>(self, start: impl FnOnce(Port<E>) -> W) -> Stream<E>
    where
        E: Send + 'static,
        W: FnOnce() + Send + 'static,
    {
        let (sender, received) = mpsc::channel();
        self.run(start(Port(sender)));
        Stream(received)
    }

    fn run(self, job: impl FnOnce() + Send + 'static) {
        match self {
            Self::Threaded => drop(thread::spawn(job)),
            #[cfg(test)]
            Self::Inline => drop(std::panic::catch_unwind(std::panic::AssertUnwindSafe(job))),
        }
    }
}

impl<T> Once<T> {
    /// Never blocks: the loop draws between rounds, and a blocking receive would
    /// freeze the frame for as long as the work took.
    pub(crate) fn landed(&self) -> Option<Result<T, Lost>> {
        match self.0.try_recv() {
            Ok(said) => Some(Ok(said)),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(Lost)),
        }
    }
}

impl<E> Stream<E> {
    /// Drained to the end of the batch rather than one event a round: a worker
    /// reports faster than the loop polls, and a queue emptied one event per
    /// frame would draw the work minutes behind itself.
    pub(crate) fn drained<End>(
        &self,
        mut each: impl FnMut(E) -> ControlFlow<End>,
    ) -> Option<Result<End, Lost>> {
        loop {
            match self.0.try_recv() {
                Ok(event) => {
                    if let ControlFlow::Break(end) = each(event) {
                        return Some(Ok(end));
                    }
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => return Some(Err(Lost)),
            }
        }
    }
}

/// Taken before the caller says anything, so the round that reports an ending is
/// already a round on which the next one is allowed. A caller that spoke first
/// and cleared the slot after would refuse a keystroke answered by a line that
/// said the work was over.
pub(crate) fn settled<S, T>(
    slot: &mut Option<S>,
    landed: impl FnOnce(&mut S) -> Option<T>,
) -> Option<(S, T)> {
    let said = landed(slot.as_mut()?)?;
    slot.take().map(|held| (held, said))
}

#[cfg(test)]
impl<T> From<Receiver<T>> for Once<T> {
    fn from(received: Receiver<T>) -> Self {
        Self(received)
    }
}

#[cfg(test)]
impl<E> From<Receiver<E>> for Stream<E> {
    fn from(received: Receiver<E>) -> Self {
        Self(received)
    }
}

#[cfg(test)]
impl<E> From<Sender<E>> for Port<E> {
    fn from(sender: Sender<E>) -> Self {
        Self(sender)
    }
}

#[cfg(test)]
impl<E> Stream<E> {
    pub(crate) fn into_receiver(self) -> Receiver<E> {
        self.0
    }
}

#[cfg(test)]
#[path = "tests/inflight.rs"]
mod tests;
