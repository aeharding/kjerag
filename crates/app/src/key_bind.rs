//! The keyboard, as one map from key to action.
//!
//! The map is cosmic-player's, extended with the standard application keys
//! cosmic-edit and cosmic-files agree on; docs/UI.md cites every line of it.
//! Nothing here is invented except `s`, which the owner asked for in issue
//! #16 and which no COSMIC app has a precedent for because no COSMIC app
//! captures its own view.
//!
//! Going through libcosmic's [`KeyBind`] rather than matching keys by hand
//! buys two things: [`KeyBind::matches`] falls back to the physical key
//! position on non-Latin layouts, which is the concern the hand-rolled space
//! handler used to solve by matching the physical key and losing every other
//! binding to layout; and the same map is what draws the accelerators beside
//! the menu items.
//!
//! `Escape` is deliberately absent: libcosmic hands it to the app through
//! `Application::on_escape`, and a binding here would fire twice.

use std::collections::HashMap;

use cosmic::iced::keyboard::Key;
use cosmic::iced::keyboard::key::Named;
use cosmic::widget::menu::action::MenuAction;
use kyerag_render::Nudge;

use crate::app::{ContextPage, Message};

/// Seconds a jump key or button moves the position (cosmic-player
/// `src/main.rs:1933-1977`).
pub const JUMP: f64 = 10.0;

pub use cosmic::widget::menu::key_bind::{KeyBind, Modifier};

/// One thing the pilot can ask for, from a key or from a menu item. The menu
/// draws an action's accelerator by looking the action up in the map, so
/// every menu item names one of these even when no key is bound to it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Action {
    About,
    CopyFrame,
    DefaultView,
    FileClearRecents,
    FileClose,
    FileOpen,
    FileOpenRecent(usize),
    Fullscreen,
    NextFrame,
    PlayPause,
    PreviousFrame,
    Quit,
    SaveFrame,
    SeekBackward,
    SeekForward,
    Settings,
    ZoomIn,
    ZoomOut,
}

impl MenuAction for Action {
    type Message = Message;

    fn message(&self) -> Message {
        match self {
            Self::About => Message::ToggleContextPage(ContextPage::About),
            Self::CopyFrame => Message::NotYet,
            Self::DefaultView => Message::Look(Nudge::Reset),
            Self::FileClearRecents => Message::FileClearRecents,
            Self::FileClose => Message::FileClose,
            Self::FileOpen => Message::FileOpen,
            Self::FileOpenRecent(index) => Message::FileOpenRecent(*index),
            Self::Fullscreen => Message::Fullscreen,
            Self::NextFrame => Message::StepFrame(1),
            Self::PlayPause => Message::PlayPause,
            Self::PreviousFrame => Message::StepFrame(-1),
            Self::Quit => Message::Quit,
            Self::SaveFrame => Message::NotYet,
            Self::SeekBackward => Message::SeekRelative(-JUMP),
            Self::SeekForward => Message::SeekRelative(JUMP),
            Self::Settings => Message::ToggleContextPage(ContextPage::Settings),
            Self::ZoomIn => Message::Look(Nudge::ZoomIn),
            Self::ZoomOut => Message::Look(Nudge::ZoomOut),
        }
    }
}

pub fn key_binds() -> HashMap<KeyBind, Action> {
    let mut binds = HashMap::new();

    macro_rules! bind {
        ([$($modifier:ident),* $(,)?], $key:expr, $action:ident) => {
            binds.insert(
                KeyBind {
                    modifiers: vec![$(Modifier::$modifier),*],
                    key: $key,
                },
                Action::$action,
            );
        };
    }

    // Transport. Space is a `Character` and not a `Named`, because winit
    // reports it as one and iced's `Named` has no `Space` at all.
    bind!([], Key::Character(" ".into()), PlayPause);
    bind!([], Key::Named(Named::ArrowLeft), SeekBackward);
    bind!([], Key::Named(Named::ArrowRight), SeekForward);
    bind!([], Key::Character(",".into()), PreviousFrame);
    bind!([], Key::Character(".".into()), NextFrame);

    // The window.
    bind!([], Key::Character("f".into()), Fullscreen);
    bind!([Alt], Key::Named(Named::Enter), Fullscreen);

    // The view. cosmic-edit's source notes why these three characters in
    // particular: they are not special to terminals, so they are free
    // (`src/key_bind.rs:41`).
    bind!([Ctrl], Key::Character("=".into()), ZoomIn);
    bind!([Ctrl], Key::Character("+".into()), ZoomIn);
    bind!([Ctrl], Key::Character("-".into()), ZoomOut);
    bind!([Ctrl], Key::Character("0".into()), DefaultView);

    // The application keys the other two first-party apps share.
    bind!([Ctrl], Key::Character("o".into()), FileOpen);
    bind!([Ctrl], Key::Character("w".into()), FileClose);
    bind!([Ctrl], Key::Character("q".into()), Quit);
    bind!([Ctrl], Key::Character(",".into()), Settings);

    // The frame, which issue #15 makes do something.
    bind!([], Key::Character("s".into()), SaveFrame);
    bind!([Ctrl], Key::Character("c".into()), CopyFrame);

    binds
}

#[cfg(test)]
mod tests {
    use cosmic::iced::keyboard::Modifiers;
    use cosmic::iced::keyboard::key::{Code, Physical};

    use super::*;

    fn pressed(modifiers: Modifiers, key: Key) -> Option<Action> {
        let physical = Physical::Code(Code::KeyA);
        key_binds()
            .into_iter()
            .find(|(bind, _)| bind.matches(modifiers, &key, Some(&physical)))
            .map(|(_, action)| action)
    }

    /// The four keys a pilot reaches for without looking, and the modifier
    /// that must not change what they mean.
    #[test]
    fn the_transport_keys_are_bare() {
        assert_eq!(
            pressed(Modifiers::empty(), Key::Character(" ".into())),
            Some(Action::PlayPause)
        );
        assert_eq!(
            pressed(Modifiers::empty(), Key::Named(Named::ArrowLeft)),
            Some(Action::SeekBackward)
        );
        assert_eq!(
            pressed(Modifiers::empty(), Key::Named(Named::ArrowRight)),
            Some(Action::SeekForward)
        );
        assert_eq!(
            pressed(Modifiers::empty(), Key::Character("f".into())),
            Some(Action::Fullscreen)
        );
    }

    /// `,` alone steps a frame back and `Ctrl+,` opens the settings: the one
    /// place in the map where a modifier changes the meaning entirely.
    #[test]
    fn the_comma_is_two_different_keys() {
        let comma = Key::Character(",".into());
        assert_eq!(
            pressed(Modifiers::empty(), comma.clone()),
            Some(Action::PreviousFrame)
        );
        assert_eq!(pressed(Modifiers::CTRL, comma), Some(Action::Settings));
    }

    /// Escape belongs to `on_escape`, so binding it here would fire twice.
    #[test]
    fn escape_is_not_in_the_map() {
        assert_eq!(pressed(Modifiers::empty(), Key::Named(Named::Escape)), None);
    }

    /// Every action a menu item can name has to be findable in the map or
    /// not in it at all; a duplicate binding would draw one accelerator on
    /// two items and fire whichever the hash map answered with.
    #[test]
    fn no_two_keys_share_an_action() {
        let mut actions: Vec<Action> = key_binds().into_values().collect();
        actions.retain(|action| *action != Action::Fullscreen && *action != Action::ZoomIn);
        let before = actions.len();
        actions.sort_by_key(|action| format!("{action:?}"));
        actions.dedup();
        assert_eq!(actions.len(), before);
    }
}
