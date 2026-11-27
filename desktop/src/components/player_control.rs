//! PlayerControl: the transport group. Shuffle, previous, play/pause (primary, 40 dp or 56 dp on the stage),
//! next, three-state repeat (off, all, one). Lime only on shuffle and repeat when on, always with the dot.
//! Play and pause swap through one in-between frame (`icon_play_pause`).

use gpui::{App, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div};
use psytify_design::sprite;
use psytify_design::tokens::motion::SPRITE_FRAME_MIN_MS;
use psytify_design::tokens::size::CONTROL_H_LARGE;
use psytify_design::tokens::space::SPACE_3;

use super::control::{OnClick, Preview};
use super::icon_button::IconButton;
use crate::theme::dp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Repeat {
    Off,
    All,
    One,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    Stopped,
    Playing,
    Loading,
}

#[derive(IntoElement)]
pub struct PlayerControl {
    id: SharedString,
    transport: Transport,
    shuffle: bool,
    repeat: Repeat,
    disabled: bool,
    play_size: f32,
    play_preview: Preview,
    on_play: Option<OnClick>,
    on_shuffle: Option<OnClick>,
    on_previous: Option<OnClick>,
    on_next: Option<OnClick>,
    on_repeat: Option<OnClick>,
}

impl PlayerControl {
    pub fn new(id: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            transport: Transport::Stopped,
            shuffle: false,
            repeat: Repeat::Off,
            disabled: false,
            play_size: CONTROL_H_LARGE,
            play_preview: Preview::Live,
            on_play: None,
            on_shuffle: None,
            on_previous: None,
            on_next: None,
            on_repeat: None,
        }
    }

    pub fn transport(mut self, transport: Transport) -> Self {
        self.transport = transport;
        self
    }

    pub fn shuffle(mut self, on: bool) -> Self {
        self.shuffle = on;
        self
    }

    pub fn repeat(mut self, repeat: Repeat) -> Self {
        self.repeat = repeat;
        self
    }

    /// Coda vuota: tutto disabilitato.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// The play button's side in dp (40 in the player bar, 56 on the stage).
    pub fn play_size(mut self, side: f32) -> Self {
        self.play_size = side;
        self
    }

    pub fn play_preview(mut self, preview: Preview) -> Self {
        self.play_preview = preview;
        self
    }

    pub fn on_play(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_play = Some(std::rc::Rc::new(handler));
        self
    }

    pub fn on_shuffle(
        mut self,
        handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_shuffle = Some(std::rc::Rc::new(handler));
        self
    }

    pub fn on_previous(
        mut self,
        handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_previous = Some(std::rc::Rc::new(handler));
        self
    }

    pub fn on_next(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_next = Some(std::rc::Rc::new(handler));
        self
    }

    pub fn on_repeat(mut self, handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_repeat = Some(std::rc::Rc::new(handler));
        self
    }
}

/// Collega la callback, se c'è, a un pulsante.
fn wired(button: IconButton, handler: Option<OnClick>) -> IconButton {
    match handler {
        Some(handler) => button.on_click(move |event, window, cx| handler(event, window, cx)),
        None => button,
    }
}

impl RenderOnce for PlayerControl {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = |suffix: &str| SharedString::from(format!("{}-{suffix}", self.id));
        let playing = self.transport == Transport::Playing;
        let swapping = crate::motion::since_change(id("glyph"), playing, SPRITE_FRAME_MIN_MS, window, cx);
        let play_icon = match (swapping, playing) {
            (Some(_), _) => sprite::ICON_PLAY_PAUSE,
            (None, true) => sprite::ICON_PAUSE,
            (None, false) => sprite::ICON_PLAY,
        };
        let mut play = IconButton::new(id("play"), play_icon)
            .primary()
            .size(self.play_size)
            .disabled(self.disabled)
            .loading(self.transport == Transport::Loading)
            .preview(self.play_preview)
            .tooltip(
                if self.transport == Transport::Playing { "Pausa" } else { "Riproduci" },
                Some("Spazio"),
            );
        if let Some(handler) = self.on_play {
            play = play.on_click(move |e, w, c| handler(e, w, c));
        }
        let repeat_icon =
            if self.repeat == Repeat::One { sprite::ICON_REPEAT_ONE } else { sprite::ICON_REPEAT };
        div()
            .flex()
            .items_center()
            .gap(dp(SPACE_3))
            .child(wired(
                IconButton::new(id("shuffle"), sprite::ICON_SHUFFLE)
                    .selected(self.shuffle)
                    .disabled(self.disabled)
                    .tooltip("Casuale", Some("Ctrl S")),
                self.on_shuffle,
            ))
            .child(wired(
                IconButton::new(id("prev"), sprite::ICON_PREVIOUS)
                    .disabled(self.disabled)
                    .tooltip("Precedente", Some("Ctrl ←")),
                self.on_previous,
            ))
            .child(play)
            .child(wired(
                IconButton::new(id("next"), sprite::ICON_NEXT)
                    .disabled(self.disabled)
                    .tooltip("Successivo", Some("Ctrl →")),
                self.on_next,
            ))
            .child(wired(
                IconButton::new(id("repeat"), repeat_icon)
                    .selected(self.repeat != Repeat::Off)
                    .disabled(self.disabled)
                    .tooltip("Ripeti", Some("Ctrl R")),
                self.on_repeat,
            ))
    }
}
