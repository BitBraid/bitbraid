//! The window's side of the providers' places in the dialogs that clone
//! and make a repository: one place per connected account under the
//! form's own, and what is done when one of them goes ahead.
//!
//! Sources: none (original design).

use gpui_kit::component::WindowExt as _;
use gpui_kit::*;
use serde_json::json;
use std::path::PathBuf;

use super::Shell;
use crate::{
    providers::{self, Site},
    views::{
        provider_clone::{ProviderCloneEvent, ProviderCloneView},
        provider_create::{Creation, ProviderCreateView},
        workspace::form::{FormHost as _, Place},
    },
};

/// The most accounts given a place in a dialog's column.
const MOST_PLACES: usize = 12;

/// The views of the accounts' places of the dialog that is open, and what
/// they say.
#[derive(Default)]
pub(super) struct Places {
    views: Vec<AnyView>,
    subscriptions: Vec<Subscription>,
    /// A repository made here that is to be made at a provider once it
    /// is open: where it is, and `provider.repo_create`'s parameters, what
    /// the status bar says meanwhile, and what is said when it is done.
    after: Option<(PathBuf, serde_json::Value, String, String)>,
}

impl Shell {
    /// The view of the account's place `ix` of the dialog that is open,
    /// when it is a `V`: for the tests.
    pub fn provider_place<V: 'static>(&self, ix: usize) -> Option<Entity<V>> {
        self.provider_places
            .views
            .get(ix)?
            .clone()
            .downcast::<V>()
            .ok()
    }
}

impl Shell {
    /// A place for each connected account in the dialog that clones.
    pub(super) fn clone_places(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Place> {
        self.provider_places = Places::default();
        let parent = self.clone_parent.clone();
        let mut places = Vec::new();
        for account in providers::accounts(cx).into_iter().take(MOST_PLACES) {
            let site = Site::of(&account);
            let label = providers::account_label(&account);
            let key = account.id.clone();
            let parent = parent.clone();
            let view = cx.new(|cx| ProviderCloneView::new(account, parent, window, cx));
            self.provider_places.subscriptions.push(cx.subscribe_in(
                &view,
                window,
                |this, _, event: &ProviderCloneEvent, window, cx| match event {
                    ProviderCloneEvent::Clone { url, path } => {
                        this.clone_from_provider(url.clone(), path.clone(), window, cx)
                    }
                },
            ));
            let view: AnyView = view.into();
            self.provider_places.views.push(view.clone());
            places.push(Place {
                key: key.into(),
                label: label.into(),
                icon: site.icon(),
                view,
            });
        }
        places
    }

    /// A place for each connected account in the dialog that makes a
    /// repository.
    pub(super) fn init_places(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Place> {
        self.provider_places = Places::default();
        let parent = self.clone_parent.clone();
        let mut places = Vec::new();
        for account in providers::accounts(cx).into_iter().take(MOST_PLACES) {
            let site = Site::of(&account);
            let label = providers::account_label(&account);
            let key = account.id.clone();
            let parent = parent.clone();
            let view = cx.new(|cx| ProviderCreateView::new(account, parent, window, cx));
            self.provider_places.subscriptions.push(cx.subscribe_in(
                &view,
                window,
                |this, _, creation: &Creation, window, cx| {
                    this.create_at_provider(creation.clone(), window, cx)
                },
            ));
            let view: AnyView = view.into();
            self.provider_places.views.push(view.clone());
            places.push(Place {
                key: key.into(),
                label: label.into(),
                icon: site.icon(),
                view,
            });
        }
        places
    }

    /// Make a repository here, with a first commit, and once it is open
    /// make it at the provider, as its remote origin, and push there.
    fn create_at_provider(
        &mut self,
        creation: Creation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.creating.is_some() {
            return;
        }
        let Some(account) = providers::account(&creation.account, cx) else {
            return;
        };
        self.forms_mut().close();
        window.close_dialog(cx);
        let site = Site::of(&account).name();
        let owner = creation
            .namespace
            .clone()
            .unwrap_or_else(|| account.login.clone());
        let name = creation.name.clone();
        let mut params = json!({
            "account": creation.account,
            "name": creation.name,
            "description": creation.description,
            "private": creation.private,
            "remote": "origin",
            "push": true,
        });
        if let (Some(namespace), Some(object)) = (&creation.namespace, params.as_object_mut()) {
            object.insert("namespace".to_owned(), namespace.clone().into());
        }
        self.provider_places.after = Some((
            creation.path.clone(),
            params,
            say!("shell-provider-making", owner = &owner, name = &name, site),
            say!("shell-provider-made", owner = &owner, name = &name, site),
        ));
        self.create(
            "repo.init",
            json!({ "path": creation.path, "first_commit": true }),
            say!("shell-creating", name = &name),
            say!("shell-created", name = &name),
            creation.path,
            false,
            window,
            cx,
        );
    }

    /// A repository was made and opened: when it is one to be made at a
    /// provider too, its tab does that now.
    pub(super) fn after_created(
        &mut self,
        path: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let wanted = self
            .provider_places
            .after
            .as_ref()
            .is_some_and(|(after, ..)| after == path);
        if !wanted {
            return;
        }
        let Some((_, params, label, done)) = self.provider_places.after.take() else {
            return;
        };
        let workspace = self
            .tabs
            .iter()
            .find(|tab| tab.path.as_deref() == Some(path))
            .and_then(|tab| match &tab.kind {
                super::TabKind::Repo(workspace) => Some(workspace.clone()),
                super::TabKind::Start(_) => None,
            });
        if let Some(workspace) = workspace {
            workspace.update(cx, |workspace, cx| {
                workspace.publish_to_provider(params, label, done, window, cx)
            });
        }
    }

    /// Clone what an account's place picked, as a clone from an address
    /// is: the dialog goes, and the clone runs in the status bar.
    fn clone_from_provider(
        &mut self,
        url: String,
        path: std::path::PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.creating.is_some() {
            return;
        }
        self.forms_mut().close();
        window.close_dialog(cx);
        let name = super::repo_title(&path);
        self.create(
            "repo.clone",
            serde_json::json!({ "url": url, "path": path }),
            say!("shell-cloning", name = &name),
            say!("shell-cloned", name = &name),
            path,
            true,
            window,
            cx,
        );
    }
}
