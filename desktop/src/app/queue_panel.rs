//! The queue panel (320 dp on the right): header with count and duration, then the entries
//! in listening order. The playing track has the equaliser in place of its number (lime only while it
//! plays, `STONE` when paused, the loader while loading); a click on an entry plays it, «Rimuovi» removes
//! it. The list is a `uniform_list`: only the visible rows are built, and the texts of each entry
//! are rebuilt once when the queue changes, not at every draw.

use std::sync::Arc;

use gpui::{
    ClickEvent, Context, Entity, IntoElement, ParentElement, Render, ScrollStrategy, SharedString, Styled,
    Subscription, UniformListScrollHandle, Window, div, uniform_list,
};
use psytify_core::command::PlaybackCmd;
use psytify_core::player::{PlaybackState, Queue};
use psytify_design::tokens::color::{TEXT_PRIMARY, TEXT_TERTIARY};
use psytify_design::tokens::space::{SPACE_3, SPACE_4, SPACE_5};
use psytify_design::tokens::type_::{TYPE_HEADING, TYPE_META};

use crate::components::button::Button;
use crate::components::indicators::PlayState;
use crate::components::overlay::EmptyState;
use crate::components::playing_spot::PlayingSpot;
use crate::components::queue_row::{QueueRow, QueueRowData};
use crate::components::separator::Separator;
use crate::core_model::{Changed, CoreModel};
use crate::format;
use crate::theme::{TypeStyle, col, dp};

pub struct QueuePanel {
    core: Entity<CoreModel>,
    queue: Arc<Queue>,
    /// Position of the playing track in listening order, and how to show it.
    current: Option<(usize, PlayState)>,
    /// The text «N brani · duration» and the texts of each entry, rebuilt only when the queue changes.
    summary: SharedString,
    rows: Vec<QueueRowData>,
    scroll: UniformListScrollHandle,
    /// The last track the list was brought to: it scrolls only when the track changes.
    scrolled_to: Option<usize>,
    /// The equaliser spot of the playing track: `Shell` paints it (BACKLOG 23).
    spot: PlayingSpot,
    _changes: Subscription,
}

impl QueuePanel {
    /// This view's equaliser spot, for `Shell`'s layer.
    pub fn spot(&self) -> PlayingSpot {
        self.spot.clone()
    }

    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let changes = cx.subscribe(&core, |this, core, changed: &Changed, cx| {
            if changed.playback {
                this.refresh(&core, cx);
            }
        });
        let mut panel = Self {
            queue: Arc::default(),
            current: None,
            summary: SharedString::default(),
            rows: Vec::new(),
            scroll: UniformListScrollHandle::new(),
            scrolled_to: None,
            core: core.clone(),
            spot: PlayingSpot::new(),
            _changes: changes,
        };
        panel.refresh(&core, cx);
        panel
    }

    /// Reads the playback domain again: new queue, new playing track or new state.
    fn refresh(&mut self, core: &Entity<CoreModel>, cx: &mut Context<Self>) {
        let snapshot = core.read(cx).snapshot();
        let view = &snapshot.playback;
        let mut dirty = false;
        if !Arc::ptr_eq(&view.queue, &self.queue) {
            self.queue = Arc::clone(&view.queue);
            self.summary = format!(
                "{} · {}",
                format::tracks(self.queue.len()),
                format::clock_ms(self.queue.total_duration_ms())
            )
            .into();
            self.rows = self
                .queue
                .entries()
                .map(|(position, track)| QueueRowData {
                    index: u32::try_from(position + 1).unwrap_or(u32::MAX),
                    title: track.title.clone().into(),
                    artist: track.artist.name.clone().into(),
                    duration: format::clock_ms(track.duration_ms).into(),
                    cover_url: track.cover_url.clone().map(SharedString::from),
                })
                .collect();
            dirty = true;
        }
        let state = match view.state {
            PlaybackState::Playing => Some(PlayState::Playing),
            PlaybackState::Paused => Some(PlayState::Paused),
            PlaybackState::Loading { .. } => Some(PlayState::Buffering),
            PlaybackState::Stopped | PlaybackState::Error(_) => None,
        };
        let current = self.queue.position().zip(state);
        if current != self.current {
            self.current = current;
            dirty = true;
        }
        let position = self.queue.position();
        if position != self.scrolled_to {
            if let Some(position) = position {
                self.scroll.scroll_to_item(position, ScrollStrategy::Nearest);
            }
            self.scrolled_to = position;
            dirty = true;
        }
        if dirty {
            cx.notify();
        }
    }

    fn play_at(&self, position: usize, cx: &gpui::App) {
        self.core.read(cx).playback(PlaybackCmd::PlayAt(position));
    }

    fn remove(&self, position: usize, cx: &gpui::App) {
        self.core.read(cx).playback(PlaybackCmd::Remove(position));
    }
}

impl Render for QueuePanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The spot is marked again in this frame's paint, if the playing row is visible.
        self.spot.clear();
        let len = self.queue.len();
        let clear = {
            let core = self.core.clone();
            move |_: &ClickEvent, _: &mut Window, cx: &mut gpui::App| {
                core.read(cx).playback(PlaybackCmd::Clear)
            }
        };
        let header = div()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap(dp(SPACE_3))
            .px(dp(SPACE_5))
            .py(dp(SPACE_4))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(div().type_role(TYPE_HEADING).text_color(col(TEXT_PRIMARY)).child("Coda"))
                    .child(
                        div().type_role(TYPE_META).text_color(col(TEXT_TERTIARY)).child(self.summary.clone()),
                    ),
            )
            .child(Button::new("queue-clear", "Svuota").ghost().disabled(len == 0).on_click(clear));

        let body: gpui::AnyElement = if len == 0 {
            div()
                .flex_1()
                .flex()
                .justify_center()
                .pt(dp(48.0))
                .child(EmptyState::crate_("La coda è vuota", "Cerca un brano e premi invio per ascoltarlo."))
                .into_any_element()
        } else {
            div()
                .flex_1()
                .min_h_0()
                .child(
                    uniform_list(
                        "queue-list",
                        len,
                        cx.processor(|this: &mut Self, range: std::ops::Range<usize>, _window, cx| {
                            range
                                .filter_map(|position| {
                                    let data = this.rows.get(position)?.clone();
                                    let mut row = QueueRow::new(("queue-row", position), data)
                                        .on_play(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                            this.play_at(position, cx)
                                        }))
                                        .on_remove(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                            this.remove(position, cx)
                                        }));
                                    if let Some((current, state)) = this.current
                                        && current == position
                                    {
                                        row = row.current_at(state, &this.spot);
                                    }
                                    Some(row)
                                })
                                .collect::<Vec<_>>()
                        }),
                    )
                    .track_scroll(&self.scroll)
                    .size_full(),
                )
                .into_any_element()
        };

        div().size_full().flex().flex_col().child(header).child(Separator::horizontal()).child(body)
    }
}
