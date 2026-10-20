//! What the inbox asks of a repository's tab: check a pull request out,
//! as its row in the pull requests section does, and open the pane that
//! adds a worktree at a pull request's head, fetched first from where it
//! comes from (`pr.head`).
//!
//! Sources: none (original design).

use bitbraid_integrations::model::{PullHead, RepoLink};
use gpui_kit::*;
use serde_json::json;

use super::{Workspace, docked::Adds};
use crate::{
    notices::Notice,
    session::Reply,
    views::pulls::{calls, facts::number_text},
};

impl Workspace {
    /// Check out a pull request of this repository (`pr.checkout`), from
    /// a fork after asking.
    pub fn checkout_pull_request(
        &mut self,
        link: RepoLink,
        number: u64,
        fork: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.checkout_pull(link, number, fork, window, cx);
    }

    /// Open the pane that adds a worktree, starting from `start` (a
    /// remote's branch, a commit).
    pub fn add_worktree_from(
        &mut self,
        start: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dock(Adds::Worktree(Some(start)), window, cx);
    }

    /// Open the pane that adds a worktree for a pull request, starting at
    /// its head: fetched first (`pr.head`), as a checkout fetches it, so
    /// that a fork's pull request starts at the fork's branch when the fork
    /// is a remote here, else at the head's commit; never at this remote's
    /// branch of the same name, which only the fork has.
    pub fn add_pull_worktree(
        &mut self,
        link: RepoLink,
        number: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let params = json!({
            "account": link.account,
            "owner": link.owner,
            "name": link.name,
            "number": number,
            "remote": link.remote,
        });
        let named = number_text(link.kind, number);
        let reply = calls::run_queued(&self.session, "pr.head", params);
        self.operation_began(
            &say!("workspace-pull-head-fetching", named = named.as_str()),
            None,
        );
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let reply = reply.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.operation_ended();
                match reply {
                    Ok(Reply::Command(value)) => match serde_json::from_value::<PullHead>(value) {
                        Ok(head) => this.add_worktree_from(head.start, window, cx),
                        Err(_) => crate::notices::push(
                            window,
                            cx,
                            Notice::error(say!(
                                "workspace-pull-head-unreadable",
                                named = named.as_str()
                            )),
                        ),
                    },
                    Ok(Reply::Error(err)) => {
                        let notice = this.failed_notice(&err, cx);
                        crate::notices::push(window, cx, notice);
                    }
                    _ => {}
                }
                // What was fetched is in the references now.
                this.reload(cx);
            });
        })
        .detach();
    }
}
