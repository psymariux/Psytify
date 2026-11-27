//! La coda nel player del telefono: il brano in corso, poi i successivi che si riordinano con la
//! pressione lunga sulla maniglia a destra e il trascinamento (`gesture::reorder`). Ogni riga è alta
//! quanto un bersaglio di tocco; un tocco sul brano lo fa partire, la «x» lo toglie.

use iced::widget::{Space, button, column, container, row, scrollable};
use iced::{Alignment, Element, Length};

use super::touch_icon;
use crate::api::Track;
use crate::app::{App, MenuTarget, Message, PhoneMsg, TrackSource};
use crate::ui::components::{
    PLAYING_LINE, SUB_LINE, TITLE_LINE, art_of, count_label, empty_block, hoverable, line, outlined_button,
    playing_bars, small, text_button, total_label,
};
use crate::ui::cover::cover;
use crate::ui::gesture::reorder::{GRIP_WIDTH, reorder};
use crate::ui::icons::{Icon, icon};
use crate::ui::theme;

/// Identificatore dello scorrimento della coda (per lo scorrimento automatico del riordino).
pub const QUEUE_SCROLL: &str = "phone-queue";
/// Altezza di una riga: un bersaglio di tocco più un filo d'aria.
const ROW: f32 = theme::TOUCH_TARGET + 12.0;

pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let queue = &app.queue;
    let summary = small(format!("{} · {}", count_label(queue.len()), total_label(queue.total_duration_ms())));
    let body: Element<'_, Message> = if queue.is_empty() {
        empty_block("La coda è vuota. Scegli un brano per iniziare.")
    } else {
        let text_width = width - theme::ROW_COVER - 44.0 - theme::TOUCH_TARGET - GRIP_WIDTH - 3.0 * 10.0;
        let mut list = column![].spacing(2);
        if let (Some(position), Some(current)) = (queue.position(), queue.current()) {
            list = list
                .push(container(small("In riproduzione")).padding([4, 8]))
                .push(entry(app, position, current, text_width, true));
        }
        let upcoming: Vec<(usize, &Track)> = queue.upcoming().collect();
        list = list.push(Space::with_height(12)).push(container(small("Successivi")).padding([4, 8]));
        if let Some(&(first, _)) = upcoming.first() {
            let rows =
                upcoming.iter().map(|&(pos, track)| entry(app, pos, track, text_width, false)).collect();
            list = list.push(reorder(
                rows,
                ROW,
                theme::RAISED,
                move |from, to| Message::Phone(PhoneMsg::QueueMove(first + from, first + to)),
                |dy| Message::Phone(PhoneMsg::QueueScroll(dy)),
            ));
        } else {
            list = list.push(container(small("Nessun altro brano.")).padding([4, 8]));
        }
        scrollable(list)
            .id(scrollable::Id::new(QUEUE_SCROLL))
            .height(Length::Fill)
            .style(theme::scroll)
            .into()
    };

    let seed = queue.tracks_in_order();
    let footer = row![
        outlined_button(
            "Salva come playlist",
            Some(Icon::Plus),
            (!queue.is_empty()).then(|| Message::OpenDialog(crate::app::Dialog::NewPlaylist {
                name: app.suggested_playlist_name(),
                seed,
            })),
        ),
        Space::with_width(Length::Fill),
        text_button("Svuota", Message::ClearQueue),
    ]
    .align_y(Alignment::Center);

    column![summary, body, footer].spacing(14).height(Length::Fill).into()
}

/// Una riga: copertina, titolo e artista, «x» e, per i successivi, la maniglia del riordino.
fn entry<'a>(
    app: &App,
    position: usize,
    track: &'a Track,
    text_width: f32,
    current: bool,
) -> Element<'a, Message> {
    let lead: Element<'a, Message> =
        if current { playing_bars(app.playback.is_playing()) } else { Space::with_width(0).into() };
    let play = button(
        row![
            lead,
            cover(&app.covers, art_of(track), theme::ROW_COVER),
            column![
                line(&track.title, text_width, if current { PLAYING_LINE } else { TITLE_LINE }),
                line(&track.artist.name, text_width, SUB_LINE),
            ]
            .spacing(3)
            .width(Length::Fill),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .height(Length::Fill)
    .width(Length::Fill)
    .padding([0, 8])
    .style(theme::row(current))
    .on_press(Message::PlayQueueAt(position));
    let mut cells = row![play, touch_icon(Icon::Close, theme::MUTED, Message::RemoveFromQueue(position))]
        .align_y(Alignment::Center);
    if !current {
        // La maniglia: una zona di `GRIP_WIDTH`, senza pulsante (la afferra il widget del riordino).
        cells = cells.push(container(icon(Icon::List, 0.0, theme::FAINT)).center(Length::Fixed(GRIP_WIDTH)));
    }
    let item = container(cells).height(if current {
        Length::Fixed(theme::TOUCH_TARGET + 8.0)
    } else {
        Length::Fill
    });
    let menu = MenuTarget::track(track, TrackSource::Queue(position));
    hoverable(track.id, true, Some(menu), item.into())
}
