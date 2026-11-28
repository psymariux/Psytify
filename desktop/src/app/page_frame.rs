//! What every page keeps of the v2 frame (desktop v2 §2, §3): the [`Frame`] measured from its real width,
//! the page enter in progress and the zoom its list was measured at.
//!
//! A page renders [`PageFrame::measure`] behind its content (a `canvas` that reads the width once per draw
//! and reports a new frame on the next tick), calls [`PageFrame::begin`] at the top of `render` (ends a
//! finished enter, asks for frames while one runs, says whether the zoom changed so the page can remeasure
//! its list) and wraps each section in [`PageFrame::wrap`]. `Shell::page_shown` starts the enter with
//! [`PageFrame::enter`].

use std::time::Instant;

use gpui::{
    AnyElement, App, Context, IntoElement, ListState, ParentElement, Styled, WeakEntity, Window, canvas, div,
};
use psytify_design::tokens::space::SPACE_3;

use crate::components::frame::Frame;
use crate::motion::{self, Enter};
use crate::theme::{Ap, dp, to_dp, zoom};

#[derive(Clone, Copy, Debug)]
pub struct PageFrame {
    pub frame: Frame,
    enter: Option<Enter>,
    /// The zoom the page's list measured its rows at.
    zoom: f32,
}

impl Default for PageFrame {
    fn default() -> Self {
        Self { frame: Frame::default(), enter: None, zoom: zoom() }
    }
}

impl PageFrame {
    /// The canvas margin each side, in dp.
    pub fn margin(&self) -> f32 {
        self.frame.margin
    }

    /// The page comes into view: its sections enter, rising unless it came from Back or Forward.
    pub fn enter(&mut self, rise: bool, cx: &App) {
        self.enter = Enter::begin(rise, cx);
    }

    /// At the top of `render`: drops a finished enter, asks for the next frame while one runs, and remeasures
    /// `list` (rows off screen kept their old height) when the zoom changed since the last draw.
    pub fn begin(&mut self, window: &Window, list: Option<&ListState>) {
        let now = Instant::now();
        if self.enter.is_some_and(|enter| !enter.running(now)) {
            self.enter = None;
        }
        motion::keep_running(window, self.enter.is_some());
        let zoom = zoom();
        if (zoom - self.zoom).abs() > f32::EPSILON {
            self.zoom = zoom;
            if let Some(list) = list {
                list.remeasure();
            }
        }
    }

    /// Section `section` in its enter move (as it is once the enter is over).
    pub fn wrap(&self, element: AnyElement, section: usize, window: &Window) -> AnyElement {
        match self.enter {
            Some(enter) => enter.wrap(element, section, Ap::of(window), Instant::now()),
            None => element,
        }
    }

    /// The width probe: behind the content, it reads the page width at every draw and hands a changed frame
    /// to `set` on the next tick (one frame late, never in the middle of a draw).
    pub fn measure<V: 'static>(
        &self,
        entity: WeakEntity<V>,
        set: fn(&mut V, Frame, &mut Context<V>),
    ) -> impl IntoElement {
        let current = self.frame;
        canvas(
            move |bounds, window, cx| {
                let frame = Frame::of(to_dp(bounds.size.width), to_dp(window.viewport_size().width));
                if frame != current {
                    cx.defer(move |cx| {
                        entity.update(cx, |this, cx| set(this, frame, cx)).ok();
                    });
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full()
    }

    /// A row component with its own `SPACE_3` inset (`TrackRow`, `SourceRow`), placed so its content lines up
    /// with the frame margin.
    pub fn row(&self, row: impl IntoElement) -> AnyElement {
        div().w_full().px(dp(self.frame.margin - SPACE_3)).child(row).into_any_element()
    }

    /// Content laid at the frame margin.
    pub fn inset(&self, content: impl IntoElement) -> gpui::Div {
        div().w_full().px(dp(self.frame.margin)).child(content)
    }
}
