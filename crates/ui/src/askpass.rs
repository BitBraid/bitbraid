//! Git's questions in the application: the askpass server of
//! `bitbraid-git` is started here, and what it is asked travels to the
//! shell, which shows a dialog. A user name is typed in the open, a password
//! or passphrase is not shown, and nothing typed is stored or logged.
//!
//! Sources: gitcredentials(7); ssh(1) (`SSH_ASKPASS`).

use bitbraid_git::askpass::{Event, Server};
use futures::channel::mpsc::{UnboundedReceiver, unbounded};
use gpui_kit::*;

/// The running server and the questions it has for the shell.
pub struct Askpass {
    /// Taken by the shell that shows the questions.
    pub events: Option<UnboundedReceiver<Event>>,
    /// Kept so that the socket lives as long as the application.
    _server: Option<Server>,
}

impl Global for Askpass {}

/// Start the askpass server, with this executable as the program git
/// children run to ask. Without one (another system, no place for the
/// socket) git fails with its own message where it would have asked.
pub fn init(cx: &mut App) {
    let (sender, receiver) = unbounded();
    let server = std::env::current_exe().ok().and_then(|program| {
        Server::start(program, move |event| {
            // The shell is gone when this fails; the asker is declined when
            // the question is dropped.
            let _ = sender.unbounded_send(event);
        })
        .ok()
    });
    crate::session::debug(|| {
        format!(
            "askpass server {}",
            if server.is_some() {
                "running"
            } else {
                "not available"
            }
        )
    });
    cx.set_global(Askpass {
        events: Some(receiver),
        _server: server,
    });
}
