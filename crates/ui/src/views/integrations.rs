//! The integrations section of the preferences: the providers in a
//! column, and the chosen one's page, with its accounts and, once one is
//! connected, the SSH key that goes to it.
//!
//! The providers stand in a column of their own between the preferences'
//! column and the page (GitHub, GitHub Enterprise Server, GitLab, GitLab
//! Self-Managed); on the page, an account as a card with its picture, its
//! login and a state (not connected in the colour of danger, connecting
//! in amber with a spinner, connected in green with a tick) and its
//! buttons; while a new one is connected, a field for a token and a save
//! button under the card. Several accounts may be connected at one site:
//! the default one is marked, and another can be made the default. A
//! server of one's own asks for its address first, then for a token, with
//! what the token has to be allowed and a button that opens the server's
//! page for making one.
//!
//! A token that is typed is masked, goes to `account.add_token` and is
//! taken out of the field at once, whatever came of it: it is never shown
//! again, and nothing here keeps or writes it.
//!
//! Sources: docs.github.com "Managing your personal access tokens";
//! docs.gitlab.com "Personal access tokens"; gpui-kit documentation
//! (input, button, spinner, dialog).

mod layout;
mod sign_in;
mod ssh_keys;

use std::time::Instant;

use bitbraid_integrations::model::{Account, DeviceCode, DevicePoll};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::json;

pub use sign_in::{Flow, Stage};

use crate::{
    providers::{self, Site},
    zoom::px,
};

/// The size of a provider's name in the column.
const SITE_TEXT: f32 = 15.;
/// A connected account's card: its size, its picture, and its buttons.
const ACCOUNT_CARD_WIDTH: f32 = 500.;
const ACCOUNT_CARD_HEIGHT: f32 = 60.;
const CARD_PICTURE: f32 = 40.;
/// The mark of where an account stands: on the card that connects one,
/// and on a connected account's card.
const STATE_MARK: f32 = 14.;
const STATE_MARK_LARGE: f32 = 16.;
const CARD_BUTTON_HEIGHT: f32 = 23.;
/// The card that connects a new account: at least this wide, this high,
/// and the outline of a person in it.
const NEW_CARD_WIDTH: f32 = 200.;
const NEW_CARD_HEIGHT: f32 = 90.;
const NEW_CARD_PICTURE: f32 = 24.;
/// The code in the dialog.
const CODE_SIZE: f32 = 30.;
/// The dialog's width.
const DIALOG_WIDTH: f32 = 460.;

/// What the section asks of the preferences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntegrationsEvent {
    /// Connect with this private key file from now on (the current
    /// profile's SSH key).
    UseKey(String),
}

/// How a new account is being connected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Way {
    /// By a code typed at the provider's site.
    Code,
    /// By a token pasted into the field.
    Token,
}

/// What the section has on show, for the tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IntegrationsProbe {
    /// The provider whose page is shown.
    pub site: &'static str,
    /// The logins of its accounts, the default one marked with `*`.
    pub accounts: Vec<String>,
    /// How a new account is being connected, if one is.
    pub connecting: Option<Way>,
    /// What is said under the token's field.
    pub token_error: Option<String>,
    /// Where a sign-in by code stands, and the code shown.
    pub sign_in: Option<(&'static str, Option<String>)>,
    /// What the server's address field holds.
    pub host: String,
    /// Whether the token's field holds anything.
    pub token_typed: bool,
    /// The SSH key in use, as its public key file, and its fingerprint.
    pub ssh_key: Option<(String, Option<String>)>,
}

/// The section.
pub struct IntegrationsView {
    focus_handle: FocusHandle,
    /// The provider whose page is shown.
    chosen: Site,
    /// The address of a server of one's own.
    host_input: Entity<InputState>,
    /// The id of the application for signing in by code at a server of
    /// one's own.
    app_input: Entity<InputState>,
    /// The token, masked.
    token_input: Entity<InputState>,
    /// What follows its typing.
    token_subscription: Subscription,
    /// A new account being connected: at which site, and how.
    connecting: Option<(Site, Way)>,
    /// "Add another account" was pressed: the card for a new one shows
    /// beside the accounts there are.
    adding: bool,
    /// Why the token was not taken.
    token_error: Option<String>,
    /// The token is being tried.
    token_busy: bool,
    /// A sign-in by code, while its dialog is open.
    flow: Option<Flow>,
    /// The asking that goes with it; dropped, it stops.
    polling: Option<Task<()>>,
    /// The account an action runs for (made the default, disconnected).
    busy: Option<String>,
    /// What came of the last thing done on the page: whether it worked,
    /// and what there is to say.
    said: Option<(bool, String)>,
    /// The sites that said they offer no sign-in by code: a token only
    /// is offered there from then on.
    token_only: Vec<Site>,
    /// The SSH key rows.
    keys: ssh_keys::Keys,
    /// The view itself, for what its rows do.
    me: WeakEntity<Self>,
    column_scroll: ScrollHandle,
    page_scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<IntegrationsEvent> for IntegrationsView {}

/// A masked field for a token, and what follows its typing: Enter saves,
/// typing takes back what was said of the last one.
fn token_field(
    window: &mut Window,
    cx: &mut Context<IntegrationsView>,
) -> (Entity<InputState>, Subscription) {
    let field = cx.new(|cx| {
        InputState::new(window, cx)
            .masked(true)
            .placeholder(say!("integrations-token-placeholder"))
    });
    let subscription = cx.subscribe_in(
        &field,
        window,
        |this, _, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { .. } => this.save_token(window, cx),
            InputEvent::Change if this.token_error.take().is_some() => cx.notify(),
            _ => {}
        },
    );
    (field, subscription)
}

/// The parameters that name a provider: its kind, and the address of a
/// server.
fn site_params(site: Site, web: &str) -> serde_json::Value {
    match site.public_web() {
        Some(_) => json!({ "kind": site.kind_word() }),
        None => json!({ "kind": site.kind_word(), "web": web }),
    }
}

impl IntegrationsView {
    /// Create the section.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let host_input =
            cx.new(|cx| InputState::new(window, cx).placeholder("https://gitlab.example.org"));
        let app_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(say!("integrations-app-placeholder"))
        });
        let (token_input, token_subscription) = token_field(window, cx);
        let subscriptions =
            vec![
                cx.subscribe_in(&host_input, window, |this, _, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.token_error = None;
                        cx.notify();
                    }
                }),
            ];
        let keys = ssh_keys::Keys::new(window, cx);
        Self {
            focus_handle: cx.focus_handle(),
            chosen: Site::Github,
            host_input,
            app_input,
            token_input,
            token_subscription,
            connecting: None,
            adding: false,
            token_error: None,
            token_busy: false,
            flow: None,
            polling: None,
            busy: None,
            said: None,
            token_only: Vec::new(),
            keys,
            me: cx.entity().downgrade(),
            column_scroll: ScrollHandle::new(),
            page_scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }

    /// The provider whose page is shown.
    pub fn site(&self) -> Site {
        self.chosen
    }

    /// Show a provider's page, from its top. What was being connected
    /// elsewhere is given up.
    pub fn choose(&mut self, site: Site, window: &mut Window, cx: &mut Context<Self>) {
        if self.chosen != site {
            self.give_up_connecting(window, cx);
            self.adding = false;
            self.said = None;
            self.host_input
                .update(cx, |input, cx| input.set_value("", window, cx));
            self.app_input
                .update(cx, |input, cx| input.set_value("", window, cx));
        }
        self.chosen = site;
        self.page_scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    /// The private key the current profile connects with; none for the
    /// SSH agent's keys.
    pub fn set_profile_key(&mut self, key: Option<String>, cx: &mut Context<Self>) {
        if self.keys.profile_key != key {
            self.keys.profile_key = key;
            self.keys.public = None;
            cx.notify();
        }
    }

    /// What the section has on show.
    pub fn probe(&self, cx: &App) -> IntegrationsProbe {
        IntegrationsProbe {
            site: self.chosen.key(),
            accounts: providers::accounts_at(self.chosen, cx)
                .into_iter()
                .map(|account| {
                    if account.default {
                        format!("*{}", account.login)
                    } else {
                        account.login
                    }
                })
                .collect(),
            connecting: self
                .connecting
                .filter(|(site, _)| *site == self.chosen)
                .map(|(_, way)| way),
            token_error: self.token_error.clone(),
            sign_in: self.flow.as_ref().map(|flow| {
                (
                    flow.stage_word(),
                    flow.code().map(|code| code.user_code.clone()),
                )
            }),
            host: self.host_input.read(cx).value().to_string(),
            token_typed: !self.token_input.read(cx).value().is_empty(),
            ssh_key: self.keys.probe(),
        }
    }

    /// The address of the site whose page is shown: its own, or what was
    /// typed for a server.
    fn web(&self, cx: &App) -> Result<String, String> {
        match self.chosen.public_web() {
            Some(web) => Ok(web.to_owned()),
            None => providers::site_address(&self.host_input.read(cx).value()),
        }
    }

    /// Stop connecting a new account: the dialog's asking ends and the
    /// token field is emptied.
    fn give_up_connecting(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.connecting = None;
        self.token_error = None;
        self.stop_sign_in();
        self.forget_token(window, cx);
    }

    /// Empty the token's field for good: a field of its own takes its
    /// place, so that not even its undo remembers what was typed.
    fn forget_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (field, subscription) = token_field(window, cx);
        self.token_input = field;
        self.token_subscription = subscription;
    }

    /// End the asking of a sign-in by code.
    fn stop_sign_in(&mut self) {
        self.polling = None;
        self.flow = None;
    }

    /// The dialog of a sign-in by code is gone: the asking ends, and the
    /// card is as it was unless a token is being typed.
    fn sign_in_closed(&mut self, cx: &mut Context<Self>) {
        self.stop_sign_in();
        if self.connecting.is_some_and(|(_, way)| way == Way::Code) {
            self.connecting = None;
        }
        cx.notify();
    }

    /// Connect a new account by token: the field shows under the card.
    pub fn connect_by_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.stop_sign_in();
        self.connecting = Some((self.chosen, Way::Token));
        self.token_error = None;
        let field = self.token_input.clone();
        cx.defer_in(window, move |_, window, cx| {
            field.update(cx, |input, cx| input.focus(window, cx));
        });
        cx.notify();
    }

    /// Connect a new account by a code: ask the provider for one and show
    /// it in a dialog, which asks the provider until the user is done.
    pub fn connect_by_code(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_code(true, window, cx);
    }

    /// Ask for a code, in a new dialog or in the one that is open.
    fn begin_code(&mut self, new_dialog: bool, window: &mut Window, cx: &mut Context<Self>) {
        let web = match self.web(cx) {
            Ok(web) => web,
            Err(problem) => {
                self.token_error = Some(problem);
                cx.notify();
                return;
            }
        };
        self.connecting = Some((self.chosen, Way::Code));
        self.token_error = None;
        self.flow = Some(Flow::new(self.chosen, web.clone()));
        if new_dialog {
            self.open_sign_in_dialog(window, cx);
        }
        window.refresh();
        let mut params = site_params(self.chosen, &web);
        let app = self.app_input.read(cx).value().trim().to_owned();
        if self.chosen.self_hosted()
            && !app.is_empty()
            && let Some(params) = params.as_object_mut()
        {
            params.insert("client_id".to_owned(), app.into());
        }
        let answer = providers::ask::<DeviceCode>("account.sign_in_start", params);
        self.polling = Some(cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let going = this
                .update_in(cx, |this, window, cx| {
                    this.take_start(answer, window, cx);
                    this.flow.as_ref().is_some_and(|flow| !flow.is_over())
                })
                .unwrap_or(false);
            if going {
                Self::keep_asking(this, cx).await;
            }
        }));
        cx.notify();
    }

    /// Ask the provider at the interval the flow says until it is over or
    /// the dialog is closed.
    async fn keep_asking(this: WeakEntity<Self>, cx: &mut AsyncWindowContext) {
        loop {
            let next = this.update_in(cx, |this, window, cx| {
                // A dialog that is gone takes the asking with it.
                if !window.has_active_dialog(cx) {
                    this.stop_sign_in();
                    return None;
                }
                let flow = this.flow.as_mut()?;
                let wait = flow.next_wait(Instant::now());
                if wait.is_none() {
                    // The code ran out here.
                    window.refresh();
                }
                let code = flow.code()?.device_code.clone();
                let mut params = site_params(flow.site, &flow.web);
                if let Some(params) = params.as_object_mut() {
                    params.insert("device_code".to_owned(), code.into());
                    params.insert("interval".to_owned(), flow.interval().into());
                }
                Some((wait?, params))
            });
            let Ok(Some((wait, params))) = next else {
                return;
            };
            cx.background_executor().timer(wait).await;
            let answer = providers::ask::<DevicePoll>("account.sign_in_poll", params).await;
            let going = this.update_in(cx, |this, window, cx| {
                this.take_poll(answer, window, cx);
                this.flow.as_ref().is_some_and(|flow| !flow.is_over())
            });
            if !matches!(going, Ok(true)) {
                return;
            }
        }
    }

    /// Take what `account.sign_in_start` answered.
    pub fn take_start(
        &mut self,
        answer: Result<DeviceCode, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(flow) = self.flow.as_mut() {
            flow.started(answer, Instant::now());
            if matches!(flow.stage, Stage::Unavailable(_)) {
                // A token is what there is: its field shows under the card.
                self.connecting = Some((flow.site, Way::Token));
                // At a server it depends on the application id typed.
                if !flow.site.self_hosted() && !self.token_only.contains(&flow.site) {
                    self.token_only.push(flow.site);
                }
            }
        }
        window.refresh();
        cx.notify();
    }

    /// Take what `account.sign_in_poll` answered.
    pub fn take_poll(
        &mut self,
        answer: Result<DevicePoll, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(flow) = self.flow.as_mut() else {
            return;
        };
        flow.polled(answer);
        if let Stage::Done(account) = &flow.stage {
            let login = account.login.clone();
            let site = flow.site;
            self.connecting = None;
            self.adding = false;
            self.said = Some((
                true,
                say!("integrations-connected-as", site = site.name(), login),
            ));
            providers::reload(cx);
        }
        window.refresh();
        cx.notify();
    }

    /// Show a sign-in by code as it would stand after these answers,
    /// without asking anybody: for the tests and the pictures.
    pub fn show_sign_in(
        &mut self,
        start: Result<DeviceCode, String>,
        polls: Vec<Result<DevicePoll, String>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let web = self
            .chosen
            .public_web()
            .map(str::to_owned)
            .unwrap_or_else(|| self.host_input.read(cx).value().to_string());
        self.connecting = Some((self.chosen, Way::Code));
        self.flow = Some(Flow::new(self.chosen, web));
        self.polling = None;
        if !window.has_active_dialog(cx) {
            self.open_sign_in_dialog(window, cx);
        }
        self.take_start(start, window, cx);
        for poll in polls {
            self.take_poll(poll, window, cx);
        }
    }

    /// The dialog of a sign-in by code: the code large, a button that
    /// copies it, one that opens the page it is typed on, and what stands.
    fn open_sign_in_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        let on_close = view.clone();
        window.open_dialog(cx, move |dialog, window, cx| {
            let flow = view.upgrade().and_then(|view| view.read(cx).flow.clone());
            let title = flow
                .as_ref()
                .map(|flow| say!("integrations-sign-in-to", site = flow.site.name()))
                .unwrap_or_else(|| say!("integrations-sign-in").to_owned());
            let for_close = on_close.clone();
            dialog
                .title(title)
                .width(px(DIALOG_WIDTH))
                .overlay_closable(false)
                .on_close(move |_, _, cx| {
                    let _ = for_close.update(cx, |this, cx| this.sign_in_closed(cx));
                })
                .child(sign_in_body(flow.as_ref(), view.clone(), window, cx))
        });
    }

    /// Try the token in the field: `account.add_token` asks the provider
    /// whose it is and keeps it in the keychain. The field is emptied at
    /// once.
    pub fn save_token(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.token_busy {
            return;
        }
        let token = self.token_input.read(cx).value().trim().to_owned();
        if token.is_empty() {
            self.token_error = Some(say!("integrations-paste-token-first").to_owned());
            cx.notify();
            return;
        }
        let web = match self.web(cx) {
            Ok(web) => web,
            Err(problem) => {
                self.token_error = Some(problem);
                cx.notify();
                return;
            }
        };
        let mut params = site_params(self.chosen, &web);
        if let Some(params) = params.as_object_mut() {
            params.insert("token".to_owned(), token.into());
        }
        self.forget_token(window, cx);
        self.token_busy = true;
        self.token_error = None;
        let site = self.chosen;
        let answer = providers::ask::<Account>("account.add_token", params);
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.token_busy = false;
                this.take_token_answer(site, answer, cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// Take what `account.add_token` answered.
    pub fn take_token_answer(
        &mut self,
        site: Site,
        answer: Result<Account, String>,
        cx: &mut Context<Self>,
    ) {
        match answer {
            Ok(account) => {
                self.connecting = None;
                self.adding = false;
                self.said = Some((
                    true,
                    say!(
                        "integrations-connected-as",
                        site = site.name(),
                        login = account.login.as_str()
                    ),
                ));
                providers::reload(cx);
            }
            Err(problem) => {
                self.connecting = Some((site, Way::Token));
                self.token_error = Some(problem);
            }
        }
        cx.notify();
    }

    /// Make an account the default one of its site.
    fn make_default(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        self.account_action("account.set_default", id, None, window, cx);
    }

    /// Ask, then disconnect an account: its token leaves the keychain.
    fn ask_disconnect(&mut self, account: Account, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        let site = Site::of(&account);
        let id = account.id.clone();
        crate::ask::show(
            crate::ask::Ask::new(say!(
                "integrations-disconnect-ask",
                login = account.login.as_str(),
                site = site.name()
            ))
            .danger(say!("integrations-disconnect"), move |_, window, cx| {
                let id = id.clone();
                let _ = view.update(cx, |this, cx| {
                    this.account_action(
                        "account.remove",
                        id,
                        Some(say!(
                            "integrations-disconnected",
                            login = account.login.as_str()
                        )),
                        window,
                        cx,
                    )
                });
            }),
            window,
            cx,
        );
    }

    /// Run a command on an account, and read the accounts again.
    fn account_action(
        &mut self,
        command: &'static str,
        id: String,
        done: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy.is_some() {
            return;
        }
        self.busy = Some(id.clone());
        self.said = None;
        let answer = providers::ask::<serde_json::Value>(command, json!({ "id": id }));
        cx.spawn_in(window, async move |this, cx| {
            let answer = answer.await;
            let _ = this.update_in(cx, |this, _, cx| {
                this.busy = None;
                match answer {
                    Ok(_) => this.said = done.map(|done| (true, done)),
                    Err(problem) => this.said = Some((false, problem)),
                }
                providers::reload(cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The column of providers: 260 wide, a row 43 high from its top.
    fn column(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let rows = Site::ALL.into_iter().map(|site| {
            let connected = providers::accounts_at(site, cx).len();
            let chosen = site == self.chosen;
            h_flex()
                .id(SharedString::from(format!(
                    "integrations-site-{}",
                    site.key()
                )))
                .w_full()
                .h(px(layout::SITE_HEIGHT))
                .flex_none()
                .pl(px(18.))
                .pr(px(14.))
                .gap(px(10.))
                .items_center()
                .cursor_pointer()
                .text_size(px(SITE_TEXT))
                .when(chosen, |row| row.bg(theme.list_active))
                .when(!chosen, |row| row.hover(|row| row.bg(theme.list_hover)))
                .child(site.icon().size(px(17.)).flex_none())
                .child(div().flex_1().min_w_0().truncate().child(site.name()))
                .when(connected > 0, |row| {
                    row.child(
                        Icon::new(IconName::CircleCheck)
                            .size(px(14.))
                            .text_color(theme.success),
                    )
                })
                .on_click(cx.listener(move |this, _, window, cx| this.choose(site, window, cx)))
                .test_support()
        });
        div()
            .relative()
            .w(px(layout::COLUMN_WIDTH))
            .h_full()
            .flex_none()
            .border_r_1()
            .border_color(theme.border)
            .child(
                v_flex()
                    .id("integrations-column")
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.column_scroll)
                    .pb(px(16.))
                    .children(rows),
            )
            .into_any_element()
    }

    /// What stands under the chosen provider's heading: the accounts as
    /// cards centred in the body (or one that connects a new account); for
    /// a server of one's own, its address and a token as rows of a form,
    /// and the button that connects; and the SSH key rows once an account
    /// is there.
    fn body(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let site = self.chosen;
        let accounts = providers::accounts_at(site, cx);
        let (read, problem) = providers::read_state(cx);
        let view = cx.entity().downgrade();
        let muted = cx.theme().muted_foreground;
        let danger = cx.theme().danger;
        let success = cx.theme().success;
        let connecting = self
            .connecting
            .filter(|(at, _)| *at == site)
            .map(|(_, way)| way);
        let new_card = accounts.is_empty() || self.adding || connecting.is_some();
        let mut body: Vec<AnyElement> = Vec::new();
        if let Some(problem) = problem.filter(|_| read) {
            body.push(
                div()
                    .text_sm()
                    .text_color(danger)
                    .child(say!("integrations-accounts-unread", problem))
                    .into_any_element(),
            );
        }
        let several = accounts.len() > 1;
        for (ix, account) in accounts.into_iter().enumerate() {
            body.push(layout::centred(account_card(
                ix,
                &account,
                several,
                &self.busy,
                view.clone(),
                cx,
            )));
        }
        if site.self_hosted() && new_card {
            // A server's page asks for its address and a token in a form,
            // and connects with the button under them; signing in with a
            // code needs an application of the server's (ours). What the
            // token needs and the button to the server's page for one come
            // once an address is typed.
            let addressed = self.addressed(cx);
            body.push(self.host_row(cx));
            body.push(self.token_row(true, addressed, cx));
            if addressed {
                body.push(self.token_page_row(cx));
            }
            body.push(self.connect_row(cx));
            body.push(self.app_row(cx));
        } else if new_card {
            let busy = self.token_busy;
            let code = !self.token_only.contains(&site);
            body.push(layout::centred(new_account_card(
                site,
                connecting,
                busy,
                code,
                view.clone(),
                cx,
            )));
            if connecting == Some(Way::Token) {
                body.push(self.token_row(false, true, cx));
                body.push(self.token_page_row(cx));
            }
        }
        if !new_card {
            let view = view.clone();
            body.push(layout::centred(
                Button::new("integrations-add-account")
                    .outline()
                    .small()
                    .icon(IconName::Plus)
                    .label(say!("integrations-add-account"))
                    .tooltip(say!("integrations-add-account-tooltip"))
                    .on_click(move |_, _, cx| {
                        let _ = view.update(cx, |this, cx| {
                            this.adding = true;
                            cx.notify();
                        });
                    }),
            ));
        }
        if let Some((worked, text)) = self.said.clone() {
            body.push(layout::centred(
                div()
                    .id("integrations-said")
                    .text_sm()
                    .text_color(if worked { success } else { danger })
                    .child(text)
                    .test_support(),
            ));
        }
        body.extend(self.ssh_rows(cx));
        let _ = muted;
        body
    }

    /// The row of a server's address.
    fn host_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        layout::form_row(
            say!("integrations-server-address"),
            Input::new(&self.host_input)
                .id("integrations-host")
                .min_h(px(layout::FIELD_HEIGHT))
                .w_full()
                .into_any_element(),
            true,
            None,
            muted,
        )
    }

    /// The row of the id of the application a server signs in by code
    /// with, and the button that signs in so.
    fn app_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let muted = theme.muted_foreground;
        let view = self.handle();
        let busy = self.token_busy;
        let why = match self.chosen.kind() {
            bitbraid_integrations::model::ProviderKind::Gitlab => {
                say!("integrations-app-why-gitlab")
            }
            bitbraid_integrations::model::ProviderKind::Github => {
                say!("integrations-app-why-github")
            }
        };
        layout::form_row(
            say!("integrations-app-id"),
            h_flex()
                .w_full()
                .gap(px(8.))
                .child(
                    div().flex_1().min_w_0().child(
                        Input::new(&self.app_input)
                            .id("integrations-app")
                            .min_h(px(layout::FIELD_HEIGHT))
                            .w_full(),
                    ),
                )
                .child(
                    Button::new("integrations-connect")
                        .outline()
                        .small()
                        .h(px(layout::BUTTON_HEIGHT))
                        .label(say!("integrations-sign-in-with-code"))
                        .disabled(busy)
                        .on_click(move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| this.connect_by_code(window, cx));
                        }),
                )
                .into_any_element(),
            true,
            Some(div().child(why).into_any_element()),
            muted,
        )
    }

    /// Whether a server's address that can be used is typed.
    fn addressed(&self, cx: &App) -> bool {
        providers::site_address(&self.host_input.read(cx).value()).is_ok()
    }

    /// The row of the token: the masked field (with the button that saves
    /// beside it, unless the page has a button of its own that connects),
    /// what went wrong, and, when `scopes`, what the token has to be
    /// allowed.
    fn token_row(&self, own_button: bool, scopes: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (danger, muted) = (theme.danger, theme.muted_foreground);
        let busy = self.token_busy;
        let site = self.chosen;
        let view = self.handle();
        let error = self.token_error.clone();
        layout::form_row(
            say!("integrations-token"),
            h_flex()
                .w_full()
                .gap(px(5.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .when(error.is_some(), |field| {
                            field.rounded(px(6.)).border_1().border_color(danger)
                        })
                        .child(
                            Input::new(&self.token_input)
                                .id("integrations-token")
                                .mask_toggle()
                                .disabled(busy)
                                .min_h(px(layout::FIELD_HEIGHT))
                                .w_full(),
                        ),
                )
                .when(!own_button, |line| {
                    line.child(
                        Button::new("integrations-token-save")
                            .primary()
                            .small()
                            .h(px(layout::BUTTON_HEIGHT))
                            .label(say!("integrations-save"))
                            .loading(busy)
                            .disabled(busy)
                            .on_click(move |_, window, cx| {
                                let _ = view.update(cx, |this, cx| this.save_token(window, cx));
                            }),
                    )
                })
                .into_any_element(),
            true,
            (scopes || error.is_some()).then(|| {
                v_flex()
                    .children(error.map(|why| {
                        div()
                            .id("integrations-token-error")
                            .text_color(danger)
                            .child(why)
                            .test_support()
                    }))
                    .when(scopes, |under| {
                        under.child(
                            div()
                                .id("integrations-token-scopes")
                                .child(say!(
                                    "integrations-token-scopes",
                                    scopes = site.scopes().join(", ")
                                ))
                                .test_support(),
                        )
                    })
                    .into_any_element()
            }),
            muted,
        )
    }

    /// The row with the button that opens the provider's page for making
    /// a token.
    fn token_page_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let site = self.chosen;
        let muted = cx.theme().muted_foreground;
        let web = site
            .public_web()
            .map(str::to_owned)
            .or_else(|| providers::site_address(&self.host_input.read(cx).value()).ok());
        let page = web.as_deref().map(|web| site.token_page(web));
        let label = match &web {
            Some(web) => say!("integrations-token-page-at", host = providers::host_of(web)),
            None => say!("integrations-token-page").to_owned(),
        };
        layout::form_row(
            "",
            Button::new("integrations-token-page")
                .outline()
                .small()
                .h(px(layout::BUTTON_HEIGHT))
                .icon(IconName::ExternalLink)
                .label(label)
                .tooltip(say!("integrations-token-page-tooltip"))
                .disabled(page.is_none())
                .on_click(move |_, _, cx| {
                    if let Some(page) = &page {
                        providers::open_link(page, cx);
                    }
                })
                .into_any_element(),
            false,
            None,
            muted,
        )
    }

    /// The row with the button that connects a server's account with the
    /// token typed, under the form.
    fn connect_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let busy = self.token_busy;
        let view = self.handle();
        let cancel = self.adding.then(|| self.handle());
        layout::form_row(
            "",
            h_flex()
                .gap(px(8.))
                .child(
                    Button::new("integrations-token-save")
                        .primary()
                        .small()
                        .h(px(layout::BUTTON_HEIGHT))
                        .label(say!("integrations-connect"))
                        .loading(busy)
                        .disabled(busy)
                        .on_click(move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| this.save_token(window, cx));
                        }),
                )
                .children(cancel.map(|view| {
                    Button::new("integrations-cancel")
                        .ghost()
                        .small()
                        .h(px(layout::BUTTON_HEIGHT))
                        .label(say!("integrations-cancel"))
                        .disabled(busy)
                        .on_click(move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.give_up_connecting(window, cx);
                                this.adding = false;
                                cx.notify();
                            });
                        })
                }))
                .into_any_element(),
            false,
            None,
            muted,
        )
    }

    fn handle(&self) -> WeakEntity<Self> {
        self.me.clone()
    }
}

/// The picture of an account: the provider's, when pictures are asked
/// for and it is in; else the pattern made from its login.
fn account_picture(account: &Account, size: f32, cx: &App) -> AnyElement {
    crate::views::avatar::picture_at(
        account.avatar_url.as_deref(),
        &account.login,
        px(size),
        cx.theme().mode.is_dark(),
        cx,
    )
}

/// A connected account's card: 500 wide and 60 high, centred; the
/// picture, the name over the login (the login alone when there is no
/// name; a server's host after the login), where it stands in the card's
/// middle in larger type, and the buttons at the right.
fn account_card(
    ix: usize,
    account: &Account,
    several: bool,
    busy: &Option<String>,
    view: WeakEntity<IntegrationsView>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let site = Site::of(account);
    let busy_here = busy.as_deref() == Some(account.id.as_str());
    let host = providers::host_of(&account.web);
    let login = account.login.clone();
    let (top, under) = match (account.name.clone(), site.self_hosted()) {
        (Some(name), true) => (name, format!("{login} · {host}")),
        (Some(name), false) => (name, login),
        (None, true) => (login, host.to_owned()),
        (None, false) => (login, String::new()),
    };
    let (for_default, for_remove) = (view.clone(), view);
    let (id, for_ask) = (account.id.clone(), account.clone());
    let default_mark = (several && account.default).then(|| {
        div()
            .id(("integrations-default-mark", ix))
            .flex_none()
            .px(px(5.))
            .rounded(px(4.))
            .border_1()
            .border_color(theme.border)
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(say!("integrations-default"))
            .test_support()
    });
    let words = v_flex()
        .flex_1()
        .min_w_0()
        .text_sm()
        .child(div().truncate().child(top))
        .when(!under.is_empty() || default_mark.is_some(), |column| {
            column.child(
                h_flex()
                    .gap(px(6.))
                    .items_center()
                    .min_w_0()
                    .child(
                        div()
                            .min_w_0()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(under.clone()),
                    )
                    .children(default_mark),
            )
        });
    let buttons = h_flex()
        .flex_none()
        .gap(px(5.))
        .when(several && !account.default, |buttons| {
            buttons.child(
                Button::new(("integrations-default", ix))
                    .outline()
                    .xsmall()
                    .h(px(CARD_BUTTON_HEIGHT))
                    .label(say!("integrations-make-default"))
                    .disabled(busy.is_some())
                    .on_click(move |_, window, cx| {
                        let id = id.clone();
                        let _ =
                            for_default.update(cx, |this, cx| this.make_default(id, window, cx));
                    }),
            )
        })
        .child(
            Button::new(("integrations-disconnect", ix))
                .outline()
                .xsmall()
                .h(px(CARD_BUTTON_HEIGHT))
                .label(say!("integrations-disconnect"))
                .loading(busy_here)
                .disabled(busy.is_some())
                .on_click(move |_, window, cx| {
                    let account = for_ask.clone();
                    let _ =
                        for_remove.update(cx, |this, cx| this.ask_disconnect(account, window, cx));
                }),
        );
    // The picture with the words, and the buttons, share what is left on
    // either side of where it stands, so that it is in the card's middle
    // while the buttons fit.
    card(("integrations-account", ix), cx)
        .w(px(ACCOUNT_CARD_WIDTH))
        .h(px(ACCOUNT_CARD_HEIGHT))
        .px(px(10.))
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(px(10.))
                .items_center()
                .child(account_picture(account, CARD_PICTURE, cx))
                .child(words),
        )
        .child(
            div()
                .flex_none()
                .child(state_line(State::Connected, true, cx)),
        )
        .child(h_flex().flex_1().justify_end().child(buttons))
        .into_any_element()
}

/// Where an account stands, as its card says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    NotConnected,
    Connecting,
    Connected,
}

/// The mark and the words of a state, in its colour; larger on a
/// connected account's card.
fn state_line(state: State, large: bool, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let mark = if large { STATE_MARK_LARGE } else { STATE_MARK };
    let (colour, words) = match state {
        State::NotConnected => (theme.danger, say!("integrations-not-connected")),
        State::Connecting => (theme.warning, say!("integrations-connecting")),
        State::Connected => (theme.success, say!("integrations-connected")),
    };
    let mark = match state {
        State::Connecting => Spinner::new().xsmall().color(colour).into_any_element(),
        State::NotConnected => Icon::new(IconName::CircleX)
            .size(px(mark))
            .text_color(colour)
            .into_any_element(),
        State::Connected => Icon::new(IconName::CircleCheck)
            .size(px(mark))
            .text_color(colour)
            .into_any_element(),
    };
    h_flex()
        .gap(px(6.))
        .items_center()
        .map(|line| {
            if large {
                line.text_base()
            } else {
                line.text_sm()
            }
        })
        .text_color(colour)
        .child(mark)
        .child(words)
        .into_any_element()
}

/// The frame of a card: a tinted box, without a line round it.
fn card(id: impl Into<ElementId>, cx: &App) -> Stateful<Div> {
    let theme = cx.theme();
    h_flex()
        .id(id)
        .flex_none()
        .gap(px(10.))
        .items_center()
        .rounded(theme.radius)
        .bg(theme.sidebar)
}

/// The card of an account that is not connected yet: small and centred,
/// a person's outline at the left, where it stands and under that the
/// buttons that connect one (or the one that gives up while it is being
/// connected).
fn new_account_card(
    _site: Site,
    connecting: Option<Way>,
    busy: bool,
    code: bool,
    view: WeakEntity<IntegrationsView>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let (for_code, for_token, for_cancel) = (view.clone(), view.clone(), view);
    let state = if connecting.is_some() || busy {
        State::Connecting
    } else {
        State::NotConnected
    };
    card("integrations-new-account", cx)
        .min_w(px(NEW_CARD_WIDTH))
        .h(px(NEW_CARD_HEIGHT))
        .pl(px(16.))
        .pr(px(20.))
        .gap(px(14.))
        .child(
            Icon::new(IconName::CircleUser)
                .size(px(NEW_CARD_PICTURE))
                .flex_none()
                .text_color(theme.muted_foreground),
        )
        .child(
            v_flex()
                .gap(px(8.))
                .child(state_line(state, false, cx))
                .child(
                    h_flex()
                        .gap(px(6.))
                        .when(connecting.is_none(), |buttons| {
                            buttons
                                .when(code, |buttons| {
                                    buttons.child(
                                        Button::new("integrations-connect")
                                            .primary()
                                            .xsmall()
                                            .h(px(CARD_BUTTON_HEIGHT))
                                            .label(say!("integrations-connect"))
                                            .tooltip(say!("integrations-connect-tooltip"))
                                            .on_click(move |_, window, cx| {
                                                let _ = for_code.update(cx, |this, cx| {
                                                    this.connect_by_code(window, cx)
                                                });
                                            }),
                                    )
                                })
                                .child(
                                    Button::new("integrations-use-token")
                                        .when(code, |button| button.outline())
                                        .when(!code, |button| button.primary())
                                        .xsmall()
                                        .h(px(CARD_BUTTON_HEIGHT))
                                        .label(if code {
                                            say!("integrations-use-token")
                                        } else {
                                            say!("integrations-connect-with-token")
                                        })
                                        .on_click(move |_, window, cx| {
                                            let _ = for_token.update(cx, |this, cx| {
                                                this.connect_by_token(window, cx)
                                            });
                                        }),
                                )
                        })
                        .when(connecting.is_some(), |buttons| {
                            buttons.child(
                                Button::new("integrations-cancel")
                                    .outline()
                                    .xsmall()
                                    .h(px(CARD_BUTTON_HEIGHT))
                                    .label(say!("integrations-cancel"))
                                    .disabled(busy)
                                    .on_click(move |_, window, cx| {
                                        let _ = for_cancel.update(cx, |this, cx| {
                                            this.give_up_connecting(window, cx);
                                            this.adding = false;
                                            cx.notify();
                                        });
                                    }),
                            )
                        }),
                ),
        )
        .into_any_element()
}

/// What the dialog of a sign-in by code shows.
fn sign_in_body(
    flow: Option<&Flow>,
    view: WeakEntity<IntegrationsView>,
    _: &mut Window,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let Some(flow) = flow else {
        return div().into_any_element();
    };
    let words = div()
        .id("sign-in-words")
        .text_sm()
        .whitespace_normal()
        .text_color(match flow.stage {
            Stage::Denied | Stage::Expired | Stage::Failed(_) => theme.danger,
            Stage::Done(_) => theme.success,
            _ => theme.foreground,
        })
        .child(flow.words())
        .test_support();
    let mut body = v_flex().gap(px(14.)).child(words);
    match (&flow.stage, flow.code()) {
        (Stage::Waiting, Some(code)) => {
            let (copied, page) = (code.user_code.clone(), code.verification_uri.clone());
            let page_shown = providers::shown(&page);
            body = body
                .child(
                    h_flex()
                        .gap(px(10.))
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .id("sign-in-code")
                                .px(px(16.))
                                .py(px(8.))
                                .rounded(theme.radius_lg)
                                .bg(theme.muted)
                                .font_family(theme.mono_font_family.clone())
                                .text_size(px(CODE_SIZE))
                                .font_semibold()
                                .child(code.user_code.clone())
                                .test_support(),
                        )
                        .child(
                            Button::new("sign-in-copy")
                                .accessibility_label(say!("integrations-copy-code"))
                                .outline()
                                .small()
                                .icon(IconName::Copy)
                                .tooltip(say!("integrations-copy-code"))
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copied.clone()))
                                }),
                        ),
                )
                .child(
                    h_flex()
                        .gap(px(10.))
                        .items_center()
                        .child(
                            Button::new("sign-in-open")
                                .primary()
                                .small()
                                .icon(IconName::ExternalLink)
                                .label(say!("integrations-open-page"))
                                .disabled(!providers::openable(&page))
                                .on_click(move |_, _, cx| {
                                    providers::open_link(&page, cx);
                                }),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(page_shown),
                        ),
                )
                .child(
                    h_flex()
                        .gap(px(8.))
                        .items_center()
                        .text_sm()
                        .text_color(theme.warning)
                        .child(Spinner::new().xsmall().color(theme.warning))
                        .child(say!(
                            "integrations-waiting",
                            site = flow.site.name(),
                            interval = flow.interval()
                        )),
                );
        }
        (Stage::Starting, _) => {
            body = body.child(
                h_flex()
                    .justify_center()
                    .child(Spinner::new().small().color(theme.warning)),
            );
        }
        _ => {}
    }
    let again = matches!(
        flow.stage,
        Stage::Denied | Stage::Expired | Stage::Failed(_)
    );
    let token = matches!(flow.stage, Stage::Unavailable(_) | Stage::Failed(_));
    let done = matches!(flow.stage, Stage::Done(_));
    let (for_again, for_token, for_close) = (view.clone(), view.clone(), view);
    body.child(
        h_flex()
            .gap(px(8.))
            .justify_end()
            .when(again, |buttons| {
                buttons.child(
                    Button::new("sign-in-again")
                        .primary()
                        .small()
                        .label(say!("integrations-try-again"))
                        .on_click(move |_, window, cx| {
                            let _ = for_again.update(cx, |this, cx| {
                                this.stop_sign_in();
                                this.begin_code(false, window, cx);
                            });
                        }),
                )
            })
            .when(token, |buttons| {
                buttons.child(
                    Button::new("sign-in-token")
                        .outline()
                        .small()
                        .label(say!("integrations-use-token"))
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            let _ =
                                for_token.update(cx, |this, cx| this.connect_by_token(window, cx));
                        }),
                )
            })
            .child(
                Button::new("sign-in-close")
                    .when(done, |button| button.primary())
                    .when(!done, |button| button.ghost())
                    .small()
                    .label(if done {
                        say!("integrations-done")
                    } else {
                        say!("integrations-close")
                    })
                    .on_click(move |_, window, cx| {
                        let _ = for_close.update(cx, |this, cx| this.sign_in_closed(cx));
                        window.close_dialog(cx);
                    }),
            ),
    )
    .into_any_element()
}

impl Focusable for IntegrationsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for IntegrationsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A dialog closed by Escape or its cross takes its sign-in along.
        if self.flow.is_some() && !window.has_active_dialog(cx) {
            self.sign_in_closed(cx);
        }
        self.look_at_key(cx);
        let column = self.column(cx);
        let body = self.body(cx);
        h_flex()
            .id("integrations")
            .track_focus(&self.focus_handle)
            .size_full()
            .child(column)
            .child(layout::page(
                SharedString::from(self.chosen.name()),
                body,
                &self.page_scroll,
            ))
    }
}
