//! Barra "now playing" (88px) sul nero: brano corrente con il cuore, controlli e avanzamento al
//! centro, pannelli e volume a destra.

use iced::widget::{Space, button, column, container, row, slider};
use iced::{Alignment, Element, Length, alignment};

use super::components::{
    Line, META_LINE, SUB_LINE, TITLE_LINE, art_of, heart, hoverable, icon_button, line, small, text_button,
    toggle_button,
};
use super::cover::cover;
use super::icons::{Icon, icon};
use super::{motion, theme};
use crate::app::{App, MenuTarget, Message, Screen, SideTab, TrackSource};
use crate::player::seek::{format_time, progress as progress_of};
use crate::player::{PlaybackState, Repeat};

const PADDING: f32 = 16.0;
const SPACING: f32 = 24.0;
const COVER: f32 = 56.0;
/// «Riprova» e la distanza dal messaggio d'errore.
const RETRY_WIDTH: f32 = 64.0;

pub fn view(app: &App, window: f32) -> Element<'_, Message> {
    // Le tre zone dividono la larghezza 3:4:3; il testo del brano sta dopo copertina e cuore.
    let info_width =
        (window - 2.0 * PADDING - 2.0 * SPACING) * 0.3 - COVER - 14.0 - super::components::ICON_BUTTON - 8.0;
    row![
        container(track_info(app, info_width)).width(Length::FillPortion(3)),
        container(transport(app)).width(Length::FillPortion(4)).align_x(alignment::Horizontal::Center),
        container(extras(app)).width(Length::FillPortion(3)).align_x(alignment::Horizontal::Right),
    ]
    .height(theme::NOW_PLAYING_HEIGHT)
    .padding([0.0, PADDING])
    .spacing(SPACING)
    .align_y(Alignment::Center)
    .into()
}

fn track_info(app: &App, width: f32) -> Element<'_, Message> {
    let Some(track) = app.queue.current() else {
        return line("Nessun brano in coda. Scegline uno dal catalogo.", width + 70.0, META_LINE);
    };
    let detail: Element<'_, Message> = match &app.playback.state {
        PlaybackState::Loading { .. } => small("Caricamento…").into(),
        // Una riga sola: il messaggio si accorcia (intero è nell'avviso) e «Riprova» resta intero.
        PlaybackState::Error(message) => row![
            line(message, (width - RETRY_WIDTH).max(0.0), Line { color: Some(theme::NEGATIVE), ..META_LINE }),
            text_button("Riprova", Message::TogglePlay),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
        .into(),
        _ if track.licence.requires_attribution() => {
            line(&format!("{} · {}", track.licence.label(), track.licence.attribution), width, META_LINE)
        }
        _ => Space::with_height(0).into(),
    };
    // La copertina apre il pannello "In riproduzione".
    let art = button(cover(&app.covers, art_of(track), COVER))
        .padding(0)
        .style(theme::bare)
        .on_press(Message::OpenPanel(SideTab::NowPlaying));
    let source = app.queue.position().map_or(TrackSource::Catalogue, TrackSource::Queue);
    let info = row![
        art,
        column![
            line(&track.title, width, TITLE_LINE),
            button(line(&track.artist.name, width, SUB_LINE))
                .padding(0)
                .style(theme::text_action)
                .on_press(Message::Navigate(Screen::Artist(track.artist.id))),
            detail,
        ]
        .spacing(3)
        .width(Length::Fill),
        heart(app, track, true),
    ]
    .spacing(14)
    .align_y(Alignment::Center);
    hoverable(track.id, false, Some(MenuTarget::track(track, source)), info.into())
}

pub(super) fn transport(app: &App) -> Element<'_, Message> {
    column![controls(app), progress(app)].spacing(4).align_x(Alignment::Center).max_width(620).into()
}

/// Casuale, precedente, play, successivo e ripeti.
pub(super) fn controls(app: &App) -> Element<'_, Message> {
    let has_track = app.queue.current().is_some();
    let play_glyph = if app.playback.is_playing() { Icon::Pause } else { Icon::Play };
    // Al cambio fra Play e Pausa l'icona rimbalza un poco; il tasto resta della sua misura.
    let pop = app.play_pop.map_or(0.0, |since| motion::pop(since, theme::MOTION_PANEL));
    let play = button(
        container(icon(
            play_glyph,
            18.0 * (1.0 + 0.28 * pop),
            if has_track { theme::ON_PRIMARY } else { theme::FAINT },
        ))
        .center(Length::Fill),
    )
    .width(theme::CONTROL_HEIGHT + 4.0)
    .height(theme::CONTROL_HEIGHT + 4.0)
    .padding(0)
    .style(theme::primary)
    .on_press_maybe(has_track.then_some(Message::TogglePlay));

    let (repeat_glyph, repeat_label) = match app.queue.repeat() {
        Repeat::Off => (Icon::Repeat, "Ripeti: no"),
        Repeat::All => (Icon::Repeat, "Ripeti: tutta la coda"),
        Repeat::One => (Icon::RepeatOne, "Ripeti: questo brano"),
    };
    let shuffle_label = if app.queue.shuffle() { "Casuale: sì" } else { "Casuale: no" };
    row![
        toggle_button(Icon::Shuffle, shuffle_label, app.queue.shuffle(), Some(Message::ToggleShuffle)),
        icon_button(Icon::Previous, "Precedente", false, has_track.then_some(Message::Previous)),
        play,
        icon_button(Icon::Next, "Successivo", false, has_track.then_some(Message::Next)),
        toggle_button(
            repeat_glyph,
            repeat_label,
            app.queue.repeat() != Repeat::Off,
            Some(Message::CycleRepeat)
        ),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .into()
}

/// Tempo trascorso, barra di avanzamento trascinabile e durata.
pub(super) fn progress(app: &App) -> Element<'_, Message> {
    let duration = app.queue.current().map(|t| t.duration()).unwrap_or_default();
    let position = app.shown_position().min(duration);
    let value = app.playback.scrubbing.unwrap_or_else(|| progress_of(position, duration));
    let bar = slider(0.0..=1.0, value, Message::Scrub)
        .step(0.001_f32)
        .on_release(Message::ScrubReleased)
        .style(theme::progress_slider);
    let time = |t| container(small(format_time(t))).width(44);
    row![container(time(position)).align_x(alignment::Horizontal::Right), bar, time(duration)]
        .spacing(10)
        .align_y(Alignment::Center)
        .into()
}

fn extras(app: &App) -> Element<'_, Message> {
    let silent = app.playback.volume == 0.0;
    let (volume_glyph, volume_label) = if silent {
        (Icon::Mute, "Riattiva l'audio")
    } else if app.playback.volume < 0.4 {
        (Icon::VolumeLow, "Disattiva l'audio")
    } else {
        (Icon::Volume, "Disattiva l'audio")
    };
    let showing = |tab: SideTab| app.panel_open && app.side_tab == tab;
    // Il videoclip e il testo a tutto schermo hanno un tasto ciascuno.
    let video: Element<'_, Message> = if app.video_available() {
        toggle_button(
            Icon::Video,
            "Videoclip a tutto schermo",
            app.focus && app.focus_video,
            Some(Message::ToggleVideoFocus),
        )
    } else {
        Space::with_width(0).into()
    };
    let lyrics_full = toggle_button(
        Icon::Expand,
        "Testo a tutto schermo (F)",
        app.focus && !app.focus_video,
        app.queue.current().map(|_| Message::ToggleFocus),
    );
    row![
        video,
        lyrics_full,
        toggle_button(
            Icon::Note,
            "In riproduzione",
            showing(SideTab::NowPlaying),
            Some(Message::OpenPanel(SideTab::NowPlaying))
        ),
        toggle_button(
            Icon::Lyrics,
            "Testo",
            showing(SideTab::Lyrics),
            Some(Message::OpenPanel(SideTab::Lyrics))
        ),
        toggle_button(Icon::Queue, "Coda", showing(SideTab::Queue), Some(Message::OpenPanel(SideTab::Queue))),
        // Vista immersiva, equalizzatore, riproduzione automatica e timer di spegnimento.
        toggle_button(
            if app.sleep.is_some() { Icon::Moon } else { Icon::More },
            "Altro: timer, equalizzatore, vista immersiva",
            app.sleep.is_some(),
            Some(Message::OpenMenu(MenuTarget::Player)),
        ),
        icon_button(volume_glyph, volume_label, false, Some(Message::ToggleMute)),
        slider(0.0..=1.0, app.playback.volume, Message::Volume)
            .step(0.01_f32)
            .on_release(Message::VolumeReleased)
            .width(100)
            .style(theme::volume_slider),
    ]
    .spacing(4)
    .align_y(Alignment::Center)
    .into()
}
