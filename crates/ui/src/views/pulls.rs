//! Pull requests (merge requests at GitLab) in the window: the PULL
//! REQUESTS section of the left panel with its groups, search box, custom
//! filters and hover cards; a pull request's view in the middle; the
//! checks of commits in the graph and the mark of a branch that has an
//! open pull request; and when lists are read again, with the mark of an
//! answer kept from before because the provider could not be reached.
//!
//! Everything shown here was read by the provider commands of
//! `bitbraid_integrations` (`provider.links`, `account.list`, `pr.list`,
//! `pr.show`, `ci.status`), run off the window's thread: the reads of a
//! provider as standalone commands with the link the section holds, so
//! that they never wait behind the repository's queue. The views take the
//! shapes of `bitbraid_integrations::model` as they are.
//!
//! Sources: none (original design).

pub mod calls;
pub mod card;
pub mod checks;
pub mod create;
pub mod facts;
pub mod filter;
pub mod markdown;
pub mod patch;
pub mod section;
pub mod view;

pub use create::{NewPullEvent, NewPullView};
pub use section::{PullsEvent, PullsView};
pub use view::{PullView, PullViewEvent};
