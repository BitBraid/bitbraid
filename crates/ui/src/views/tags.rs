//! Tag requests: the menus of the graph and of the references panel make
//! them, the workspace asks where it has to and runs them.
//!
//! Sources: gpui-kit documentation (popup menu).

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::*;
use serde::Deserialize;

/// One request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum TagRequest {
    /// Ask for a name (and a message, which makes the tag an annotated
    /// one) and create a tag at this commit or reference.
    New(String),
    /// Ask for a name and then a message, and create an annotated tag at
    /// this commit or reference.
    NewAnnotated(String),
    /// Push this tag: to the only remote, or to one that is asked for.
    Push(String),
    /// Push every tag, likewise.
    PushAll,
    /// Ask, then delete this tag here.
    Delete(String),
    /// Ask, then delete this tag on a remote: the only one, or one that
    /// is asked for.
    DeleteRemote(String),
    /// Show who made this tag, when, with what message, and what git makes
    /// of its signature.
    Details(String),
    /// Ask for a message and give it to this lightweight tag.
    Annotate(String),
}

/// The action a menu entry dispatches.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = tag, no_json)]
pub struct RunTag {
    /// What to do.
    pub request: TagRequest,
}

fn entry(request: TagRequest) -> Box<dyn Action> {
    Box::new(RunTag { request })
}

/// The entries that make a tag at `target`: one without a message of its
/// own, and an annotated one.
pub fn new_tag_entry(menu: PopupMenu, target: &str) -> PopupMenu {
    menu.menu(
        say!("tags-new-here"),
        entry(TagRequest::New(target.to_owned())),
    )
    .menu(
        say!("tags-new-annotated-here"),
        entry(TagRequest::NewAnnotated(target.to_owned())),
    )
}

/// The entry that gives a lightweight tag a message.
pub fn annotate_entry(menu: PopupMenu, name: &str) -> PopupMenu {
    menu.menu(
        say!("tags-annotate"),
        entry(TagRequest::Annotate(name.to_owned())),
    )
}

/// The entry of the tags' heading that pushes them all.
pub fn push_all_entry(menu: PopupMenu) -> PopupMenu {
    menu.menu(say!("tags-push-all"), entry(TagRequest::PushAll))
}

/// Menu entries for one tag.
pub fn tag_entries(menu: PopupMenu, name: &str) -> PopupMenu {
    let tag = || name.to_owned();
    menu.menu(say!("tags-details"), entry(TagRequest::Details(tag())))
        .menu(say!("tags-push"), entry(TagRequest::Push(tag())))
        .menu(say!("tags-delete"), entry(TagRequest::Delete(tag())))
        .menu(
            say!("tags-delete-remote"),
            entry(TagRequest::DeleteRemote(tag())),
        )
}

/// What `tag.details` answered, as lines to show: what kind of tag it is
/// and where it points, who made it and when, what git makes of its
/// signature, and its message.
pub fn details_lines(value: &serde_json::Value) -> Vec<String> {
    let text = |value: &serde_json::Value, key: &str| {
        value
            .get(key)
            .and_then(|text| text.as_str())
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let short = |id: String| id.chars().take(7).collect::<String>();
    let annotated = value.get("annotated").and_then(|flag| flag.as_bool()) == Some(true);
    let target = text(value, "target").map(short).unwrap_or_default();
    let kind = text(value, "target_kind").unwrap_or_else(|| "commit".to_owned());
    let mut lines = vec![if annotated {
        say!("tags-details-annotated", kind, target)
    } else {
        say!("tags-details-lightweight", kind, target)
    }];
    if let Some(tagger) = value.get("tagger").filter(|tagger| tagger.is_object()) {
        let name = text(tagger, "name").unwrap_or_default();
        let email = text(tagger, "email").unwrap_or_default();
        let seconds = tagger.get("seconds").and_then(|s| s.as_i64()).unwrap_or(0);
        let offset = tagger
            .get("offset")
            .and_then(|offset| offset.as_i64())
            .and_then(|offset| i32::try_from(offset).ok())
            .unwrap_or(0);
        lines.push(say!(
            "tags-details-tagged-by",
            name,
            email,
            time = crate::views::absolute_time(seconds, offset)
        ));
    }
    if let Some(signature) = value
        .get("signature")
        .and_then(crate::views::signatures::Verification::from_json)
    {
        lines.push(signature.sentence());
    } else if annotated {
        lines.push(say!("tags-details-not-signed").to_owned());
    }
    if let Some(message) = text(value, "message") {
        lines.push(String::new());
        lines.extend(message.trim_end().lines().map(str::to_owned));
    }
    lines
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use serde_json::json;

    use super::details_lines;

    #[test]
    fn a_tags_details_are_told_in_lines() {
        let annotated = json!({
            "name": "v1.0",
            "annotated": true,
            "target": "0123456789abcdef0123456789abcdef01234567",
            "target_kind": "commit",
            "tagger": { "name": "Ada", "email": "ada@fixture.invalid", "seconds": 1_577_836_800, "offset": 3600 },
            "message": "release 1.0\n\nwith notes\n",
            "signature": { "state": "good", "signer": "Ada" },
        });
        assert_eq!(
            details_lines(&annotated),
            [
                "An annotated tag on the commit 0123456",
                "Tagged by Ada <ada@fixture.invalid> on 2020-01-01 01:00",
                "Good signature by Ada",
                "",
                "release 1.0",
                "",
                "with notes",
            ]
        );
        let unsigned =
            json!({ "annotated": true, "target": "abcdef0123", "signature": { "state": "none" } });
        assert_eq!(
            details_lines(&unsigned),
            ["An annotated tag on the commit abcdef0", "Not signed"]
        );
        let light = json!({ "annotated": false, "target": "abcdef0123", "target_kind": "commit", "tagger": null });
        assert_eq!(
            details_lines(&light),
            ["A lightweight tag on the commit abcdef0: a name and nothing more"]
        );
        // Garbage is a tag about which nothing is known.
        assert_eq!(details_lines(&json!("nonsense")).len(), 1);
    }
}
