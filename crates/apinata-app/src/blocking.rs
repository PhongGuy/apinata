//! Blocking git work on its own thread, with the results handed back over a channel.

use async_channel::{Receiver, Sender};

/// Runs `work` on a dedicated thread and returns the events it sends.
///
/// Not `cx.background_spawn`: the test executor polls background tasks on the test thread,
/// where a stalled git would freeze the cancel path under test.
pub fn off_thread_events<E: Send + 'static>(
    work: impl FnOnce(&Sender<E>) + Send + 'static,
) -> Receiver<E> {
    let (tx, rx) = async_channel::unbounded();
    std::thread::spawn(move || work(&tx));
    rx
}

/// Runs `work` on a dedicated thread and delivers its one result.
pub fn off_thread<T: Send + 'static>(work: impl FnOnce() -> T + Send + 'static) -> Receiver<T> {
    off_thread_events(move |tx| {
        let _ = tx.try_send(work());
    })
}
