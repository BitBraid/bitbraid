//! How the views of pull requests run the provider commands: each on a
//! thread of its own, off the window's (`session::run_standalone`), so
//! that a provider that is slow to answer never holds the window or the
//! repository's queue; and the commands that read the repository (its
//! templates, a prediction of conflicts) on that queue. What the
//! off-screen tests want answered in place of a provider is given here
//! beforehand ([`stand_in`]): the views then get those answers as they
//! would get the provider's, and what they asked is kept to be looked at
//! ([`asked`]).
//!
//! Only the window's thread asks, so the stand-ins are kept for it alone.
//!
//! Sources: none (original design).

use std::{cell::RefCell, future::Future, rc::Rc};

use futures::channel::oneshot;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::session::{Reply, Request, SessionHandle, run_standalone, run_standalone_unasked};

/// What answers in place of the providers: for a command and its
/// parameters, an answer, or none to run it for real.
type Answer = Rc<dyn Fn(&str, &Value) -> Option<Result<Value, String>>>;

#[derive(Default)]
struct StandIn {
    answer: Option<Answer>,
    asked: Vec<(String, Value)>,
    /// The commands whose answers wait for [`release`], as a slow
    /// provider's would, and the answers that wait.
    held: Vec<String>,
    waiting: Vec<(Result<Value, String>, oneshot::Sender<Reply>)>,
}

thread_local! {
    static STAND_IN: RefCell<StandIn> = RefCell::new(StandIn::default());
}

/// Answer the provider commands with `answer` from now on, where it gives
/// one: for the off-screen tests. What was asked before is forgotten.
pub fn stand_in(answer: impl Fn(&str, &Value) -> Option<Result<Value, String>> + 'static) {
    STAND_IN.with(|stand_in| {
        *stand_in.borrow_mut() = StandIn {
            answer: Some(Rc::new(answer)),
            ..StandIn::default()
        };
    });
}

/// From now on the stand-in's answers to the command `id` wait until
/// [`release`], as the answers of a provider that is slow do: for the
/// off-screen tests of what a view shows meanwhile.
pub fn hold(id: &str) {
    STAND_IN.with(|stand_in| stand_in.borrow_mut().held.push(id.to_owned()));
}

/// Give the answers that wait, and hold nothing any longer.
pub fn release() {
    let waiting = STAND_IN.with(|stand_in| {
        let mut stand_in = stand_in.borrow_mut();
        stand_in.held.clear();
        std::mem::take(&mut stand_in.waiting)
    });
    for (given, sender) in waiting {
        let _ = sender.send(reply_of(given));
    }
}

/// Run the provider commands for real again.
pub fn stand_down() {
    STAND_IN.with(|stand_in| *stand_in.borrow_mut() = StandIn::default());
}

/// What was asked while a stand-in answered: each command with its
/// parameters, in the order they were asked.
pub fn asked() -> Vec<(String, Value)> {
    STAND_IN.with(|stand_in| stand_in.borrow().asked.clone())
}

/// What a stand-in answers for a command, when one stands in and has an
/// answer for it; the command is kept among those asked either way.
pub fn answered(id: &str, params: &Value) -> Option<Result<Value, String>> {
    let answer = STAND_IN.with(|stand_in| {
        let mut stand_in = stand_in.borrow_mut();
        let answer = stand_in.answer.clone()?;
        stand_in.asked.push((id.to_owned(), params.clone()));
        Some(answer)
    });
    answer.and_then(|answer| answer(id, params))
}

fn reply_of(given: Result<Value, String>) -> Reply {
    match given {
        Ok(value) => Reply::Command(value),
        Err(error) => Reply::Error(error),
    }
}

/// A reply that is there already.
fn ready(given: Result<Value, String>) -> oneshot::Receiver<Reply> {
    let (sender, receiver) = oneshot::channel();
    let _ = sender.send(reply_of(given));
    receiver
}

/// A stand-in's reply to `id`: there already, or waiting for [`release`]
/// while `id` is held.
fn given(id: &str, given: Result<Value, String>) -> oneshot::Receiver<Reply> {
    STAND_IN.with(|stand_in| {
        let mut stand_in = stand_in.borrow_mut();
        if !stand_in.held.iter().any(|held| held == id) {
            return ready(given);
        }
        let (sender, receiver) = oneshot::channel();
        stand_in.waiting.push((given, sender));
        receiver
    })
}

/// Run a provider command: its reply comes on the receiver.
pub fn run(id: &str, params: Value) -> oneshot::Receiver<Reply> {
    match answered(id, &params) {
        Some(answer) => given(id, answer),
        None => run_standalone(id, params).reply,
    }
}

/// Run a command on the open repository's queue, behind what it is
/// doing: one that reads the repository or its working tree (the
/// templates, a prediction of conflicts, commits).
pub fn run_queued(session: &SessionHandle, id: &str, params: Value) -> oneshot::Receiver<Reply> {
    match answered(id, &params) {
        Some(answer) => given(id, answer),
        None => session.send(Request::Command {
            id: id.to_owned(),
            params,
        }),
    }
}

/// Read a reply as a `T`. An error is the sentence to show.
async fn read<T: DeserializeOwned>(reply: oneshot::Receiver<Reply>) -> Result<T, String> {
    match reply.await {
        Ok(Reply::Command(value)) => {
            serde_json::from_value(value).map_err(|_| say!("pulls-answer-unreadable").to_owned())
        }
        Ok(Reply::Error(error)) => Err(crate::providers::said(&error)),
        Ok(_) | Err(_) => Err(say!("pulls-no-answer").to_owned()),
    }
}

/// Run a provider command and read its answer as a `T`. An error is the
/// sentence to show.
pub fn ask<T: DeserializeOwned + 'static>(
    id: &'static str,
    params: Value,
) -> impl Future<Output = Result<T, String>> + 'static {
    read(run(id, params))
}

/// [`ask`] for what nobody asked for: the inbox read again on its timer,
/// the workspaces listed beside it. What git is run for on the way (the
/// look at each repository) stays on this machine
/// (`session::run_standalone_unasked`); the providers are asked as ever.
pub fn ask_in_background<T: DeserializeOwned + 'static>(
    id: &'static str,
    params: Value,
) -> impl Future<Output = Result<T, String>> + 'static {
    read(match answered(id, &params) {
        Some(answer) => given(id, answer),
        None => run_standalone_unasked(id, params).reply,
    })
}

/// [`ask`], on the open repository's queue ([`run_queued`]).
pub fn ask_queued<T: DeserializeOwned + 'static>(
    session: &SessionHandle,
    id: &'static str,
    params: Value,
) -> impl Future<Output = Result<T, String>> + 'static {
    read(run_queued(session, id, params))
}
