//! The title bar and the sidebar of `Shell` as `.cached()` views (BACKLOG 23).
//!
//! `Shell` is the window root and GPUI redraws it on every frame: every tick of a sprite it paints (the
//! equalizer over the playing rows of cached pages, `PlayingSpot`), of the views it holds (GPUI marks the
//! ancestors dirty) and every video frame. These two views redraw only when what they show changes (`Key`,
//! compared when `Shell` notifies: `Shell::sync_chrome`), when their bounds change (the sidebar width easing)
//! or when they notify themselves (hover, the mark animating while it plays). `Shell` builds the elements
//! (`Shell::chrome`) with its handlers.

use gpui::{Context, IntoElement, Render, WeakEntity, Window, div};

use super::Shell;
use super::nav::Page;
use super::sidebar::Tab;
use crate::titlebar::AccountChip;

/// Quale parte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    TitleBar,
    Sidebar,
}

/// Ciò che una parte mostra di `Shell`: finché non cambia, la parte si riusa dalla cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    TitleBar { playing: bool, back: bool, forward: bool, account: AccountChip },
    Sidebar { current: Page, counts: [usize; 4], rail: bool, rev: u64, tab: Tab },
}

pub struct Chrome {
    shell: WeakEntity<Shell>,
    part: Part,
}

impl Chrome {
    pub fn new(shell: WeakEntity<Shell>, part: Part) -> Self {
        Self { shell, part }
    }
}

impl Render for Chrome {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let part = self.part;
        self.shell
            .update(cx, |shell, cx| shell.chrome(part, window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}
