//! Bridges blocking work (registry reads/writes, `verify_login`'s external
//! process calls) into `iced::Task`'s async world without pulling in a full
//! `tokio` runtime -- `iced` already re-exports `futures` and ships its own
//! executor (`docs.rs/iced`: default features include a native `thread-pool`
//! executor), so a `std::thread::spawn` + oneshot channel is all we need.

use iced::Task;
use iced::futures::channel::oneshot;

/// Runs `work` on its own OS thread and resolves once it's done, wrapped as
/// an `iced::Task` that sends `to_message(result)` back into `update`.
///
/// `work` must be `'static` -- callers `move` in whatever they need
/// (registry handles, form field values already cloned out of `State`).
pub fn perform<T, M>(work: impl FnOnce() -> T + Send + 'static, to_message: impl Fn(T) -> M + Send + 'static) -> Task<M>
where
    T: Send + 'static,
    M: Send + 'static,
{
    let (tx, rx) = oneshot::channel();
    std::thread::spawn(move || {
        // The only way this send fails is if the receiving future was
        // dropped (e.g. the application shut down mid-task) -- nothing
        // to report to, so silently drop the result.
        let _ = tx.send(work());
    });
    Task::perform(rx, move |result| {
        // `rx.await` only errs if the sender was dropped without sending,
        // which the spawn above never does (it always sends, even if the
        // closure itself panics the thread would abort before reaching
        // here -- an existing-code panic bug, not something to paper over
        // with a fabricated fallback value).
        to_message(result.expect("perform: worker thread dropped its sender without sending"))
    })
}

// No unit tests here: `Task` only actually runs under a live `iced`
// event loop (there's no headless harness to drive one outside a real
// window), so `perform`'s wiring is exercised implicitly by every screen
// that uses it and verified by manually running the app -- not something
// a fake test that never drives the `Task` to completion could honestly
// claim to cover.
