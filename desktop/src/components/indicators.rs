//! Indicatori di stato fatti di sprite: Equalizer (riproduzione), LoadingIndicator (attesa e
//! scheletri) e DownloadIndicator (stato del download di un brano).

use gpui::{
    App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window,
    div,
};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    ERROR, ICON_ACTIVE, ICON_DISABLED, ICON_IDLE, STATE_ON, STONE, SURFACE_RAISED, SURFACE_SELECTED,
    TEXT_TERTIARY,
};
use psytify_design::tokens::type_::TYPE_META;

use super::tooltip::with_tooltip;
use crate::pixel::PixelSprite;
use crate::pixel::SpriteStore;
use crate::theme::{TypeStyle, col, dp};

/// State of the equaliser of a row or a tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayState {
    Playing,
    Paused,
    Buffering,
}

/// `eq` lime animato in riproduzione, `eq_rest` `STONE` in pausa, `loader` in caricamento.
/// At most one animated per list: the caller decides.
#[derive(IntoElement)]
pub struct Equalizer {
    state: PlayState,
}

impl Equalizer {
    pub fn new(state: PlayState) -> Self {
        Self { state }
    }
}

impl RenderOnce for Equalizer {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        match self.state {
            PlayState::Playing => PixelSprite::new(sprite::EQ).tint(STATE_ON),
            PlayState::Paused => PixelSprite::new(sprite::EQ_REST).tint(STONE),
            PlayState::Buffering => PixelSprite::new(sprite::LOADER),
        }
    }
}

/// Waiting: a 12 AP `loader` (showing it only after 300 ms is the caller's job) or a skeleton.
#[derive(IntoElement)]
pub struct LoadingIndicator {
    caption: Option<SharedString>,
}

impl LoadingIndicator {
    pub fn new() -> Self {
        Self { caption: None }
    }

    /// Riga `TYPE_META` accanto al loader (caricamenti lunghi, oltre 2 s).
    pub fn caption(mut self, caption: impl Into<SharedString>) -> Self {
        self.caption = Some(caption.into());
        self
    }
}

impl Default for LoadingIndicator {
    fn default() -> Self {
        Self::new()
    }
}

impl RenderOnce for LoadingIndicator {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap(dp(psytify_design::tokens::space::SPACE_3))
            .child(PixelSprite::new(sprite::LOADER))
            .children(
                self.caption.map(|c| div().type_role(TYPE_META).text_color(col(TEXT_TERTIARY)).child(c)),
            )
    }
}

/// Skeleton block at the content's size: it goes from `SURFACE_RAISED` to `SURFACE_SELECTED` every
/// 600 ms a scatti (nessuna dissolvenza). Si ferma con `reduce_motion`.
#[derive(IntoElement)]
pub struct Skeleton {
    w: f32,
    h: f32,
}

impl Skeleton {
    pub fn new(w: f32, h: f32) -> Self {
        Self { w, h }
    }
}

impl RenderOnce for Skeleton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (on, delay) = match cx.try_global::<SpriteStore>() {
            Some(store) if !store.reduce_motion() => {
                let t = store.elapsed_ms();
                (t / 600 % 2 == 1, Some(600 - t % 600))
            }
            _ => (false, None),
        };
        if let Some(ms) = delay {
            crate::pixel::anim::request_refresh(window, cx, ms);
        }
        div().w(dp(self.w)).h(dp(self.h)).bg(col(if on { SURFACE_SELECTED } else { SURFACE_RAISED }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DownloadState {
    Available,
    /// The track cannot be downloaded: nothing drawn, space kept.
    NotDownloadable,
    Queued,
    /// Avanzamento 0..=1.
    Downloading(f32),
    Done,
    Error,
    /// In the index, but the file is missing or changed: it is downloaded again (error tint).
    Lost,
}

#[derive(IntoElement)]
pub struct DownloadIndicator {
    id: ElementId,
    state: DownloadState,
}

impl DownloadIndicator {
    pub fn new(id: impl Into<ElementId>, state: DownloadState) -> Self {
        Self { id: id.into(), state }
    }
}

impl RenderOnce for DownloadIndicator {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let slot = div().id(self.id).w(dp(24.0)).h(dp(24.0)).flex().items_center().justify_center();
        let (icon, tint, tip): (Option<&'static str>, _, Option<&str>) = match self.state {
            DownloadState::Available => (Some(sprite::ICON_DOWNLOAD), ICON_IDLE, None),
            DownloadState::NotDownloadable => (None, ICON_IDLE, None),
            DownloadState::Queued => (Some(sprite::ICON_DOWNLOAD), ICON_DISABLED, Some("In coda")),
            DownloadState::Downloading(_) => (Some(sprite::ICON_DOWNLOAD), ICON_ACTIVE, None),
            DownloadState::Done => (Some(sprite::ICON_DOWNLOADED), ICON_ACTIVE, None),
            DownloadState::Error => {
                (Some(sprite::ICON_DOWNLOAD), ERROR, Some("Download non riuscito. Riprova"))
            }
            DownloadState::Lost => {
                (Some(sprite::ICON_DOWNLOAD), ERROR, Some("File mancante: scarica di nuovo"))
            }
        };
        let slot = match icon {
            Some(name) => slot.child(PixelSprite::new(name).tint(tint)),
            None => slot,
        };
        match tip {
            Some(text) => with_tooltip(slot, text.to_owned(), None).into_any_element(),
            None => slot.into_any_element(),
        }
    }
}

/// Download progress bar in 10 steps (each step 1 AP, `WARM_WHITE` on `IRON`).
#[derive(IntoElement)]
pub struct DownloadSteps {
    progress: f32,
}

impl DownloadSteps {
    pub fn new(progress: f32) -> Self {
        Self { progress: progress.clamp(0.0, 1.0) }
    }
}

impl RenderOnce for DownloadSteps {
    fn render(self, window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let ap = crate::theme::Ap::of(window);
        let lit = (self.progress * 10.0).floor() as u32;
        div().flex().gap(ap.one()).children((0..10).map(move |i| {
            let color = if i < lit {
                psytify_design::tokens::color::WARM_WHITE
            } else {
                psytify_design::tokens::color::IRON
            };
            div().w(ap.n(2)).h(ap.n(2)).bg(col(color))
        }))
    }
}
