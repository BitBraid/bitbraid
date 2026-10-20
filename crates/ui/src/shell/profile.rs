//! The profile control of the tab bar, and the panel it opens.
//!
//! The control is who is committed as: the current profile's picture, its
//! name and a caret, in a box of its own, with the measures of the
//! constants below. There is always a current profile
//! (`views/profiles.rs`); until the profiles are read the control is a
//! person.
//!
//! A click opens a panel 330 wide under it, at the window's right: the
//! current profile in a list to switch to another from, who that commits
//! as, and the way to the section of the preferences that keeps them.
//! Switching is the window's (`Shell::switch_profile`).
//!
//! Sources: gpui-kit documentation (button, popover, popup menu).

use gpui_kit::component::{
    ActiveTheme, Icon, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    menu::DropdownMenu,
    popover::Popover,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;

use super::SwitchProfile;
use crate::{
    keymap::ShowProfiles,
    views::profiles::{self, Profiles},
};

/// The control's height.
pub(super) const HEIGHT: f32 = 28.;
/// The picture, each way.
pub(super) const PICTURE: f32 = 20.;
/// From the control's left and right edge to what is in it.
const EDGE: f32 = 5.;
/// From the picture to the name.
const GAP: f32 = 6.;
/// The widest the name gets before it is cut: the tabs need the room.
const NAME_WIDTH: f32 = 160.;
/// The panel's width.
const PANEL_WIDTH: f32 = 330.;
/// From the panel's edges to what is in it.
const PANEL_INSET: f32 = 20.;

/// The control: the picture of the profile that is current, its name and
/// a caret. Before the profiles are read, a person.
pub(super) fn control(profiles: &Profiles, cx: &App) -> Button {
    let Some(profile) = profiles.current_profile() else {
        return Button::new("profiles")
            .accessibility_label(say!("shell-profiles"))
            .ghost()
            .small()
            .icon(Icon::new(gpui_kit::assets::IconName::UserRound))
            .tooltip(say!("shell-profiles-tooltip"));
    };
    Button::new("profiles")
        .accessibility_label(say!("shell-profile", name = &profile.name))
        .ghost()
        .small()
        .h(px(HEIGHT))
        .px(px(EDGE))
        .tooltip(say!("shell-profile", name = &profile.name))
        .child(
            h_flex()
                .gap(px(GAP))
                .child(profiles::picture(profile.picture_key(), PICTURE, cx))
                .child(
                    div()
                        .max_w(px(NAME_WIDTH))
                        .text_sm()
                        .truncate()
                        .child(profile.name.clone()),
                ),
        )
        .dropdown_caret(true)
}

/// The control and the panel it opens.
pub(super) fn panel(profiles: &Profiles, cx: &App) -> impl IntoElement {
    let trigger = control(profiles, cx);
    let profiles = profiles.clone();
    // The panel is drawn here, to its own measures, not in the toolkit's
    // padded frame.
    Popover::new("profile-panel")
        .anchor(Anchor::TopRight)
        .appearance(false)
        .trigger(trigger)
        .content(move |_, _, cx| panel_content(&profiles, cx))
}

/// What the panel holds.
fn panel_content(
    profiles: &Profiles,
    cx: &mut Context<gpui_kit::component::popover::PopoverState>,
) -> AnyElement {
    let theme = cx.theme().clone();
    let current = profiles.current_profile().cloned().unwrap_or_default();
    let listed: Vec<(String, String, bool)> = profiles
        .current_first()
        .into_iter()
        .map(|profile| {
            (
                profile.name.clone(),
                profile.picture_key().to_owned(),
                profile.name == current.name,
            )
        })
        .collect();
    let several = listed.len() > 1;
    let chooser = Button::new("panel-profile")
        .accessibility_label(say!("shell-profile", name = &current.name))
        .outline()
        .w_full()
        .h(px(42.))
        .px(px(10.))
        .child(
            h_flex()
                .w_full()
                .gap(px(14.))
                .child(profiles::picture(current.picture_key(), 28., cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .child(current.name.clone()),
                ),
        )
        .when(several, |button| button.dropdown_caret(true));
    // With several there is a list to switch from; with one, the profile.
    let chooser: AnyElement = if several {
        chooser
            .dropdown_menu_with_anchor(Anchor::TopLeft, move |menu, _, _| {
                listed.iter().fold(menu, |menu, (name, key, now)| {
                    let (shown, key) = (name.clone(), key.clone());
                    menu.menu_element_with_check(
                        *now,
                        Box::new(SwitchProfile { name: name.clone() }),
                        move |_, cx| {
                            h_flex()
                                .h(px(37.))
                                .gap(px(10.))
                                .items_center()
                                .child(profiles::picture(&key, 28., cx))
                                .child(shown.clone())
                        },
                    )
                })
            })
            .into_any_element()
    } else {
        chooser.into_any_element()
    };
    let (name, address) = profiles.identity.clone().unwrap_or_default();
    let caption = |text: &'static str| {
        div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(text)
    };
    v_flex()
        .w(px(PANEL_WIDTH))
        .mt(px(2.))
        .rounded(theme.radius)
        .border_1()
        .border_color(theme.border)
        .bg(theme.popover)
        .shadow_lg()
        .child(
            div()
                .px(px(PANEL_INSET))
                .py(px(10.))
                .border_b_1()
                .border_color(theme.border)
                .font_semibold()
                .child(say!("shell-profile-current")),
        )
        .child(
            v_flex()
                .px(px(PANEL_INSET))
                .pt(px(20.))
                .pb(px(12.))
                .gap(px(10.))
                .child(chooser)
                .child(
                    v_flex()
                        .gap_1()
                        .child(caption(say!("shell-profile-commits-as")))
                        .child(
                            v_flex()
                                .pl(px(10.))
                                .child(div().text_sm().child(name))
                                .child(div().text_xs().child(address)),
                        ),
                )
                .child(
                    h_flex()
                        .id("panel-manage")
                        .h(px(40.))
                        .gap(px(10.))
                        .items_center()
                        .cursor_pointer()
                        .text_sm()
                        .hover(|this| this.text_color(theme.primary))
                        .child(Icon::new(gpui_kit::assets::IconName::Users).size(px(18.)))
                        .child(say!("shell-profile-manage"))
                        .on_click(cx.listener(|state, _, window, cx| {
                            state.dismiss(window, cx);
                            window.dispatch_action(Box::new(ShowProfiles), cx);
                        }))
                        .test_support(),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use crate::views::profiles::picture_key;

    #[test]
    fn a_picture_is_made_from_the_address_and_from_the_name_without_one() {
        assert_eq!(
            picture_key("Work", Some("ada@work.invalid")),
            "ada@work.invalid"
        );
        assert_eq!(
            picture_key("Work", Some(" ada@work.invalid ")),
            "ada@work.invalid"
        );
        assert_eq!(picture_key("Work", Some("  ")), "Work");
        assert_eq!(picture_key("Work", None), "Work");
    }
}
