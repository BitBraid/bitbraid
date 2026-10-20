//! Sparse checkout in the lists of files: a folder's menu checks out only
//! that folder, or adds it to what is checked out; while only part of the
//! files is checked out, a bar over the changes says so and turns it off.
//!
//! The workspace asks the engine how the checkout stands (`sparse.status`)
//! and hands that to the lists, which only draw it; what is picked goes
//! back to the workspace as a file action, and the bar's button is the
//! action that turns sparse checkout off.
//!
//! Sources: git-sparse-checkout(1) (what a cone takes: everything under
//! its folders, and the files directly in the folders above them).

use gpui_kit::component::{
    ActiveTheme, Icon, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    menu::PopupMenu,
    tooltip::Tooltip,
};
use gpui_kit::*;
use serde_json::Value;

use crate::{
    views::file_actions::{FileAction, RunFileAction},
    zoom::px,
};

/// How the checkout stands, as the lists need it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SparseMark {
    /// Only part of the files is checked out.
    pub enabled: bool,
    /// The part is chosen by folder.
    pub cone: bool,
    /// The folders of a cone, or the patterns.
    pub rules: Vec<String>,
    /// How many tracked files are left out.
    pub outside: u64,
}

impl SparseMark {
    /// Read what `sparse.status` answered.
    pub fn from_json(value: &Value) -> Self {
        let flag = |key: &str| value.get(key).and_then(Value::as_bool) == Some(true);
        Self {
            enabled: flag("enabled"),
            cone: flag("cone"),
            rules: value
                .get("rules")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|rule| rule.as_str().map(str::to_owned))
                .take(1000)
                .collect(),
            outside: value.get("outside").and_then(Value::as_u64).unwrap_or(0),
        }
    }

    /// Whether everything below `folder` is checked out: in a cone, it is
    /// one of its folders or lies below one. Of a checkout by patterns
    /// that cannot be told, and the answer is no.
    pub fn takes_all_of(&self, folder: &str) -> bool {
        if !self.enabled {
            return true;
        }
        if !self.cone {
            return false;
        }
        let folder = folder.trim_end_matches('/');
        self.rules.iter().any(|rule| {
            let rule = rule.trim_end_matches('/');
            folder == rule
                || folder
                    .strip_prefix(rule)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
    }

    /// What the bar says.
    pub fn said(&self) -> String {
        let what = if !self.cone {
            say!("sparse-by-patterns", count = self.rules.len())
        } else if self.rules.is_empty() {
            say!("sparse-top-only").to_owned()
        } else {
            let mut named: Vec<&str> = self.rules.iter().map(String::as_str).take(3).collect();
            let more = self.rules.len().saturating_sub(named.len());
            named.sort_unstable();
            let folders = named.join(", ");
            if more > 0 {
                say!("sparse-folders-and-more", folders, more)
            } else {
                folders
            }
        };
        match self.outside {
            0 => say!("sparse-bar-text", what),
            outside => say!("sparse-bar-text-left-out", what, outside),
        }
    }
}

/// The entries of a folder's menu: check out only it, and while the
/// checkout is sparse, add it or check out everything again. None without
/// a worktree.
pub fn folder_entries(
    menu: PopupMenu,
    folder: &str,
    mark: Option<&SparseMark>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let Some(mark) = mark.cloned() else {
        return menu;
    };
    let folder = folder.trim_end_matches('/').to_owned();
    menu.separator()
        .submenu(say!("sparse-menu"), window, cx, move |menu, _, _| {
            let mut menu = menu.menu(
                say!("sparse-only-this-folder"),
                Box::new(RunFileAction {
                    action: FileAction::SparseOnly {
                        path: folder.clone(),
                    },
                }),
            );
            if mark.enabled && !mark.takes_all_of(&folder) {
                menu = menu.menu(
                    say!("sparse-add-this-folder"),
                    Box::new(RunFileAction {
                        action: FileAction::SparseAdd {
                            path: folder.clone(),
                        },
                    }),
                );
            }
            if mark.enabled {
                menu = menu.separator().menu(
                    say!("sparse-check-out-everything"),
                    Box::new(crate::keymap::CheckOutEverything),
                );
            }
            menu
        })
}

/// The bar over the changes while the checkout is sparse: what is checked
/// out, and a button that checks out everything again.
pub fn bar(mark: &SparseMark, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let rules = if mark.rules.is_empty() {
        say!("sparse-top-files").to_owned()
    } else {
        mark.rules.join("\n")
    };
    h_flex()
        .id("sparse-bar")
        .w_full()
        .h(px(28.))
        .px_2()
        .gap_2()
        .items_center()
        .border_b_1()
        .border_color(theme.border)
        .bg(theme.secondary)
        .text_xs()
        .tooltip(move |window, cx| Tooltip::new(rules.clone()).build(window, cx))
        .child(
            Icon::new(gpui_kit::assets::IconName::FolderTree)
                .size_3p5()
                .flex_none()
                .text_color(theme.muted_foreground),
        )
        .child(div().flex_1().min_w_0().truncate().child(mark.said()))
        .child(
            Button::new("sparse-everything")
                .ghost()
                .xsmall()
                .label(say!("sparse-check-out-everything"))
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(crate::keymap::CheckOutEverything), cx)
                }),
        )
        // Asked for in the tests.
        .test_support()
        .into_any_element()
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use serde_json::json;

    use super::SparseMark;

    #[test]
    fn the_mark_is_read_and_said() {
        let cone = SparseMark::from_json(&json!({
            "enabled": true, "cone": true, "rules": ["src/ui", "docs", 5, "tools", "assets"],
            "outside": 12,
        }));
        assert_eq!(cone.rules, ["src/ui", "docs", "tools", "assets"]);
        assert_eq!(
            cone.said(),
            "Sparse checkout: docs, src/ui, tools and 1 more; 12 files left out"
        );
        assert!(cone.takes_all_of("src/ui"));
        assert!(cone.takes_all_of("src/ui/deeper/"));
        assert!(!cone.takes_all_of("src"));
        assert!(!cone.takes_all_of("src/uix"));
        let top = SparseMark::from_json(&json!({ "enabled": true, "cone": true, "outside": 1 }));
        assert_eq!(
            top.said(),
            "Sparse checkout: only the files at the top; 1 file left out"
        );
        let patterns =
            SparseMark::from_json(&json!({ "enabled": true, "cone": false, "rules": ["/*"] }));
        assert_eq!(patterns.said(), "Sparse checkout: by 1 pattern");
        assert!(!patterns.takes_all_of("anything"));
        let off = SparseMark::from_json(&json!("nonsense"));
        assert!(!off.enabled && off.takes_all_of("anything"));
    }
}
