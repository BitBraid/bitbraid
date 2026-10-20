//! Issues of GitHub and GitLab in the window: the ISSUES section of the
//! left panel with its repository, search box, query and groups; an
//! issue's card and its view in the middle; a branch made for an issue;
//! and issue keys drawn as links in commit messages and branch names,
//! with the issue's card on them.
//!
//! Everything shown here was read by the provider commands of
//! `bitbraid_integrations` (`provider.links`, `issue.list`,
//! `issue.show`), run off the window's thread as standalone commands with
//! the link the section holds; a branch for an issue is made by
//! `issue.branch` on the repository's queue. The views take the shapes of
//! `bitbraid_integrations::model` as they are. Issues are read only.
//!
//! Sources: none (original design).

pub mod card;
pub mod facts;
pub mod linked;
pub mod section;
pub mod view;

pub use section::{IssuesEvent, IssuesView};
pub use view::{IssueView, IssueViewEvent};
