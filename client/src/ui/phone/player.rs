//! Il player a schermo intero del telefono, sulla tela nera: la copertina grande, titolo e artista, la
//! barra di avanzamento con i tempi sotto, i controlli grandi per il pollice e, in fondo, le schede
//! Testo e Coda. Aperta una scheda, il testo o la coda prendono il posto della copertina, che resta
//! piccola accanto al titolo; i controlli restano dove sono.

use iced::widget::{Space, button, column, container, row, slider, stack, text};
use iced::{Alignment, Color, Element, Length, Padding, Size, alignment};

use super::touch_icon;
use crate::api::Track;
use crate::app::{App, MenuTarget, Message, PhoneMsg, SideTab};
use crate::player::queue::Repeat;
use crate::player::seek::{self, format_time, progress as progress_of};
use crate::ui::components::{H1_LINE, MUTED_LINE, TITLE_LINE, art_of, heart, line, small};
use crate::ui::cover::cover;
use crate::ui::focus::breathing_cover;
use crate::ui::icons::{Icon, icon};
use crate::ui::pointer::pointer;
use crate::ui::touch_seek::touch_seek;
use crate::ui::{lyrics, theme, volume_side};

/// Margine del player dai bordi.
const GUTTER: f32 = 24.0;
/// Il tasto Play: un quadrato primario, il più grande dello schermo dopo la copertina.
const PLAY: f32 = psytify_design::tokens::size::phone::PHONE_PLAY_BUTTON;
/// Precedente e successivo: più grandi degli interruttori ai lati.
const SKIP: f32 = 56.0;
/// La copertina piccola sopra il testo e la coda.
const MINI_COVER: f32 = 52.0;

/// Il download del brano in corso; spento se la licenza non lo permette (il tocco dice perché),
/// e «…» mentre scarica.
fn download_button<'a>(app: &App, track: &Track) -> Element<'a, Message> {
    if let Some(item) = app.downloads.item(track.id).filter(|item| item.is_busy()) {
        // In corso: la percentuale se si sa; in coda o in preparazione, «…».
        let label = item.percent().map_or_else(|| "…".to_owned(), |percent| format!("{percent}%"));
        return container(text(label).size(theme::FS_TITLE).color(theme::TEXT))
            .center_x(Length::Fixed(theme::TOUCH_TARGET))
            .center_y(Length::Fixed(theme::TOUCH_TARGET))
            .into();
    }
    if app.downloads.is_saved(track.id) {
        // Sul dispositivo: il segno di «scaricato». Un file perso torna il tasto, con la tinta d'errore,
        // per riscaricarlo; con il file a posto un tocco dice che il brano è già qui.
        let (glyph, tint) = if app.downloads.lost_reason(track.id).is_some() {
            (Icon::Download, theme::ERROR)
        } else {
            (Icon::Downloaded, theme::ICON_ACTIVE)
        };
        return touch_icon(glyph, tint, Message::Download(track.clone()));
    }
    let tint = if track.downloadable { theme::MUTED } else { theme::DISABLED };
    touch_icon(Icon::Download, tint, Message::Download(track.clone()))
}

pub fn view(app: &App, window: Size, edges: Padding) -> Element<'_, Message> {
    let Some(track) = app.queue.current() else {
        return Space::new(Length::Fill, Length::Fill).into();
    };
    let width = (window.width - edges.left - edges.right - 2.0 * GUTTER).max(200.0);
    let header = row![
        touch_icon(Icon::ChevronDown, theme::TEXT, Message::Phone(PhoneMsg::DismissPlayer)),
        container(text("In riproduzione").size(theme::FS_SMALL).font(theme::FONT_MEDIUM).color(theme::MUTED))
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Center),
        touch_icon(Icon::People, theme::TEXT, Message::OpenJam),
        touch_icon(Icon::More, theme::TEXT, Message::OpenMenu(MenuTarget::Player)),
    ]
    .align_y(Alignment::Center);

    let mut detail = track.artist.name.clone();
    if let Some(album) = &track.album {
        detail = format!("{detail} · {}", album.title);
    }

    // Testo e coda prendono tutto lo spazio fra il brano e i controlli; la copertina si fa piccola.
    let middle: Element<'_, Message> = match app.side_tab {
        SideTab::NowPlaying => {
            let side = (window.height * theme::PHONE_PLAYER_COVER).min(width).max(160.0);
            let title_width = width - 2.0 * theme::TOUCH_TARGET - 8.0;
            let title = row![
                column![line(&track.title, title_width, H1_LINE), line(&detail, title_width, MUTED_LINE)]
                    .spacing(6)
                    .width(Length::Fill),
                download_button(app, track),
                heart(app, track, true),
            ]
            .spacing(8)
            .align_y(Alignment::Center);
            let cover_and_title = column![
                container(breathing_cover(app, track, side))
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .center(Length::Fill),
                title,
            ]
            .spacing(16)
            .height(Length::Fill);
            // Il volume sta sul bordo destro, sopra la copertina che resta al centro.
            let bar = (window.height * 0.26).clamp(130.0, 230.0);
            stack![cover_and_title, volume_side::overlay(app, bar, true)]
                .width(Length::Fill)
                .height(Length::Fill)
                .into()
        }
        tab => {
            let title_width = width - MINI_COVER - theme::TOUCH_TARGET - 2.0 * 12.0;
            // Un tocco sul brano torna alla copertina grande.
            let summary = button(
                row![
                    cover(&app.covers, art_of(track), MINI_COVER),
                    column![
                        line(&track.title, title_width, TITLE_LINE),
                        line(&detail, title_width, MUTED_LINE)
                    ]
                    .spacing(4)
                    .width(Length::Fill),
                ]
                .spacing(12)
                .align_y(Alignment::Center),
            )
            .padding(0)
            .style(theme::bare)
            .on_press(Message::SideTab(SideTab::NowPlaying));
            let body: Element<'_, Message> = if tab == SideTab::Queue {
                super::queue::view(app, width)
            } else {
                lyrics::view_as(app, lyrics::PHONE)
            };
            column![
                row![summary, heart(app, track, true)].spacing(12).align_y(Alignment::Center),
                container(body).height(Length::Fill),
            ]
            .spacing(16)
            .height(Length::Fill)
            .into()
        }
    };

    // Le due schede: toccare quella accesa torna alla copertina.
    let choice = |label: &'static str, tab: SideTab| -> Element<'_, Message> {
        let on = app.side_tab == tab;
        button(text(label).size(theme::FS_BODY).font(theme::FONT_MEDIUM))
            .padding([8, 18])
            .style(theme::pill(on))
            .on_press(Message::SideTab(if on { SideTab::NowPlaying } else { tab }))
            .into()
    };
    let tabs = container(row![choice("Testo", SideTab::Lyrics), choice("Coda", SideTab::Queue)].spacing(8))
        .width(Length::Fill)
        .align_x(alignment::Horizontal::Center);

    let page = column![header, middle, progress(app), controls(app), tabs].spacing(14).height(Length::Fill);

    container(page)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(Padding {
            top: edges.top + 4.0,
            bottom: edges.bottom + 12.0,
            left: edges.left + GUTTER - 10.0,
            right: edges.right + GUTTER - 10.0,
        })
        .style(theme::canvas)
        .into()
}

/// La barra a tutta larghezza, con il tempo trascorso e la durata sotto i due capi. È alta quanto
/// un bersaglio di tocco, e il dito la trascina (`touch_seek`), non solo la tocca.
fn progress(app: &App) -> Element<'_, Message> {
    let duration = app.queue.current().map(|t| t.duration()).unwrap_or_default();
    let position = app.scrubbing.map_or_else(
        || app.shown_position().min(duration),
        |fraction| seek::seek_fraction(fraction, duration),
    );
    let value = app.scrubbing.unwrap_or_else(|| progress_of(position, duration));
    let bar = pointer(
        slider(0.0..=1.0, value, Message::Scrub)
            .step(0.001_f32)
            .height(theme::TOUCH_TARGET)
            .on_release(Message::ScrubReleased)
            .style(theme::progress_slider),
    );
    column![
        stack![bar, touch_seek(Message::Scrub, Message::ScrubReleased)],
        row![small(format_time(position)), Space::with_width(Length::Fill), small(format_time(duration))],
    ]
    .into()
}

/// Casuale, precedente, Play, successivo e ripeti, distribuiti sulla riga.
fn controls(app: &App) -> Element<'_, Message> {
    let play_glyph = if app.playback.is_playing() { Icon::Pause } else { Icon::Play };
    let play = button(container(icon(play_glyph, 0.0, theme::ON_PRIMARY)).center(Length::Fill))
        .width(PLAY)
        .height(PLAY)
        .padding(0)
        .style(theme::primary)
        .on_press(Message::TogglePlay);
    let skip = |glyph: Icon, message: Message| {
        button(container(icon(glyph, 0.0, theme::TEXT)).center(Length::Fill))
            .width(SKIP)
            .height(SKIP)
            .padding(0)
            .style(theme::bare)
            .on_press(message)
    };
    // Gli interruttori accesi sono lime con un punto sotto (la forma dice lo stato, non solo il colore).
    let switch = |glyph: Icon, on: bool, message: Message| {
        let color = if on { theme::LIME } else { theme::MUTED };
        let dot =
            container(Space::new(4.0, 4.0)).style(theme::bar(if on { color } else { Color::TRANSPARENT }));
        button(
            container(column![icon(glyph, 0.0, color), dot].spacing(3).align_x(Alignment::Center))
                .center(Length::Fill),
        )
        .width(theme::TOUCH_TARGET)
        .height(theme::TOUCH_TARGET)
        .padding(0)
        .style(theme::bare)
        .on_press(message)
    };
    let repeat = match app.queue.repeat() {
        Repeat::One => Icon::RepeatOne,
        Repeat::Off | Repeat::All => Icon::Repeat,
    };
    row![
        switch(Icon::Shuffle, app.queue.shuffle(), Message::ToggleShuffle),
        Space::with_width(Length::Fill),
        skip(Icon::Previous, Message::Previous),
        Space::with_width(Length::Fill),
        play,
        Space::with_width(Length::Fill),
        skip(Icon::Next, Message::Next),
        Space::with_width(Length::Fill),
        switch(repeat, app.queue.repeat() != Repeat::Off, Message::CycleRepeat),
    ]
    .align_y(Alignment::Center)
    .into()
}
