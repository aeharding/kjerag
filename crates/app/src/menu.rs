//! The menu bar: `File`, `Playback` and `View`, in `header_start`.
//!
//! Built with `responsive_menu_bar` rather than the older `MenuBar::new` that
//! cosmic-player still uses, because it collapses to a single button on a
//! narrow window and a narrow window is a normal size for a video player
//! (cosmic-edit `src/menu.rs:229-240`).
//!
//! Accelerators are drawn from the key-bind map, so an item only names its
//! action. An item whose capability does not exist yet is `ButtonDisabled`,
//! which is how the menu can be complete before the app is; that is
//! cosmic-player's own pattern for its frame-step items.

use std::collections::HashMap;
use std::sync::LazyLock;

use cosmic::app::Core;
use cosmic::widget::Id;
use cosmic::widget::menu::key_bind::KeyBind;
use cosmic::widget::menu::{Item, ItemHeight, ItemWidth};
use cosmic::widget::responsive_menu_bar;
use cosmic::{Element, theme};

use crate::app::Message;
use crate::config::ConfigState;
use crate::key_bind::Action;
use crate::strings;

/// The bar's id, which is how libcosmic remembers the width it collapses at.
static MENU_ID: LazyLock<Id> = LazyLock::new(|| Id::new("kyerag-menu-bar"));

pub fn menu_bar<'a>(
    core: &Core,
    state: &ConfigState,
    key_binds: &HashMap<KeyBind, Action>,
    has_file: bool,
    horizon_locked: bool,
) -> Element<'a, Message> {
    responsive_menu_bar()
        .item_height(ItemHeight::Dynamic(40))
        .item_width(ItemWidth::Uniform(320))
        .spacing(theme::active().cosmic().spacing.space_xxxs.into())
        .into_element(
            core,
            key_binds,
            MENU_ID.clone(),
            Message::Surface,
            vec![
                (
                    strings::FILE.to_owned(),
                    vec![
                        Item::Button(strings::OPEN_VIDEO.to_owned(), None, Action::FileOpen),
                        Item::Folder(strings::OPEN_RECENT.to_owned(), recent(state)),
                        enabled(has_file, strings::CLOSE_VIDEO, Action::FileClose),
                        Item::Divider,
                        // Issue #15 gave these two something to do, and there
                        // is nothing to take a still of without a file.
                        enabled(has_file, strings::SAVE_FRAME, Action::SaveFrame),
                        enabled(has_file, strings::COPY_FRAME, Action::CopyFrame),
                        Item::Divider,
                        Item::Button(strings::QUIT.to_owned(), None, Action::Quit),
                    ],
                ),
                (
                    strings::PLAYBACK.to_owned(),
                    vec![
                        enabled(has_file, strings::PLAY_PAUSE, Action::PlayPause),
                        enabled(has_file, strings::BACK_10, Action::SeekBackward),
                        enabled(has_file, strings::FORWARD_10, Action::SeekForward),
                        Item::Divider,
                        enabled(has_file, strings::PREVIOUS_FRAME, Action::PreviousFrame),
                        enabled(has_file, strings::NEXT_FRAME, Action::NextFrame),
                    ],
                ),
                (
                    strings::VIEW.to_owned(),
                    vec![
                        enabled(has_file, strings::ZOOM_IN, Action::ZoomIn),
                        enabled(has_file, strings::DEFAULT_VIEW, Action::DefaultView),
                        enabled(has_file, strings::ZOOM_OUT, Action::ZoomOut),
                        Item::Divider,
                        // A checkbox rather than a pair of items, which is
                        // what cosmic-files does for every setting that is a
                        // state (`src/menu.rs`, "Show hidden files").
                        Item::CheckBox(
                            strings::LOCK_HORIZON.to_owned(),
                            None,
                            horizon_locked,
                            Action::LockHorizon,
                        ),
                        Item::Divider,
                        Item::Button(strings::FULLSCREEN.to_owned(), None, Action::Fullscreen),
                        Item::Divider,
                        Item::Button(strings::SETTINGS.to_owned(), None, Action::Settings),
                        Item::Button(strings::about_item(), None, Action::About),
                    ],
                ),
            ],
        )
}

/// `File > Open recent`. The divider and `Clear recent list` only appear once
/// there is something to clear (cosmic-player `src/menu.rs:49-55`).
fn recent(state: &ConfigState) -> Vec<Item<Action, String>> {
    let mut items: Vec<_> = state
        .recent_files
        .iter()
        .enumerate()
        .map(|(index, path)| {
            Item::Button(strings::recent(path), None, Action::FileOpenRecent(index))
        })
        .collect();
    if !items.is_empty() {
        items.push(Item::Divider);
        items.push(Item::Button(
            strings::CLEAR_RECENT.to_owned(),
            None,
            Action::FileClearRecents,
        ));
    }
    items
}

fn enabled(yes: bool, label: &str, action: Action) -> Item<Action, String> {
    match yes {
        true => Item::Button(label.to_owned(), None, action),
        false => Item::ButtonDisabled(label.to_owned(), None, action),
    }
}
