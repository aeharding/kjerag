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
static MENU_ID: LazyLock<Id> = LazyLock::new(|| Id::new("kjerag-menu-bar"));

pub(crate) struct MenuState {
    pub has_file: bool,
    pub can_transport: bool,
    pub can_go_to_view: bool,
    pub horizon_locked: bool,
    pub can_lock: bool,
    pub flow: Option<bool>,
}

pub fn menu_bar<'a>(
    core: &Core,
    state: &ConfigState,
    key_binds: &HashMap<KeyBind, Action>,
    menu: MenuState,
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
                        enabled(menu.has_file, strings::CLOSE_VIDEO, Action::FileClose),
                        Item::Divider,
                        // Issue #15 gave these two something to do, and there
                        // is nothing to take a still of without a file.
                        enabled(menu.has_file, strings::SAVE_FRAME, Action::SaveFrame),
                        enabled(menu.has_file, strings::COPY_FRAME, Action::CopyFrame),
                        // Under the two picture items rather than in `View`,
                        // which holds the things that move the view rather
                        // than the things that take something away from it.
                        enabled(menu.has_file, strings::COPY_VIEW, Action::CopyView),
                        // The one item in this menu that is never drawn
                        // disabled. A reference carrying a whole path opens
                        // the video it names, so it has something to do with
                        // nothing open; and what the clipboard holds cannot
                        // be known here anyway, because reading it is a task
                        // whose answer arrives later and this runs on every
                        // redraw.
                        enabled(menu.can_go_to_view, strings::GO_TO_VIEW, Action::GoToView),
                        Item::Divider,
                        Item::Button(strings::QUIT.to_owned(), None, Action::Quit),
                    ],
                ),
                (
                    strings::PLAYBACK.to_owned(),
                    vec![
                        enabled(menu.can_transport, strings::PLAY_PAUSE, Action::PlayPause),
                        enabled(menu.can_transport, strings::BACK_10, Action::SeekBackward),
                        enabled(menu.can_transport, strings::FORWARD_10, Action::SeekForward),
                        Item::Divider,
                        enabled(
                            menu.can_transport,
                            strings::PREVIOUS_FRAME,
                            Action::PreviousFrame,
                        ),
                        enabled(menu.can_transport, strings::NEXT_FRAME, Action::NextFrame),
                    ],
                ),
                (
                    strings::VIEW.to_owned(),
                    vec![
                        enabled(menu.has_file, strings::ZOOM_IN, Action::ZoomIn),
                        enabled(menu.has_file, strings::DEFAULT_VIEW, Action::DefaultView),
                        enabled(menu.has_file, strings::ZOOM_OUT, Action::ZoomOut),
                        Item::Divider,
                        // A checkbox rather than a pair of items, which is
                        // what cosmic-files does for every setting that is a
                        // state (`src/menu.rs`, "Show hidden files").
                        //
                        // Disabled on a capture that carries no orientation
                        // record, because a checkbox that ticks and does
                        // nothing is worse than one that cannot be reached: a
                        // DJI Osmo 360 `.OSV` is exactly that file today
                        // (`kjerag_meta::osmo`, "No IMU"). `ButtonDisabled`
                        // rather than a disabled checkbox because libcosmic
                        // has no such variant, and this module's own rule is
                        // that a capability which is not there is a disabled
                        // button.
                        match menu.can_lock {
                            true => Item::CheckBox(
                                strings::LOCK_HORIZON.to_owned(),
                                None,
                                menu.horizon_locked,
                                Action::LockHorizon,
                            ),
                            false => Item::ButtonDisabled(
                                strings::LOCK_HORIZON.to_owned(),
                                None,
                                Action::LockHorizon,
                            ),
                        },
                        // The selected ONE X2 route runs automatically and must
                        // not fall through the legacy route. The preference
                        // remains editable with no file open or with a camera
                        // whose legacy route is supported.
                        optical_flow(menu.flow),
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

fn optical_flow(flow: Option<bool>) -> Item<Action, String> {
    match flow {
        Some(on) => Item::CheckBox(
            strings::OPTICAL_FLOW.to_owned(),
            None,
            on,
            Action::OpticalFlow,
        ),
        None => Item::ButtonDisabled(strings::OPTICAL_FLOW.to_owned(), None, Action::OpticalFlow),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_optical_flow_is_a_disabled_button_not_a_checkbox() {
        assert!(matches!(
            optical_flow(None),
            Item::ButtonDisabled(_, _, Action::OpticalFlow)
        ));
        assert!(matches!(
            optical_flow(Some(false)),
            Item::CheckBox(_, _, false, Action::OpticalFlow)
        ));
        assert!(matches!(
            optical_flow(Some(true)),
            Item::CheckBox(_, _, true, Action::OpticalFlow)
        ));
    }
}
