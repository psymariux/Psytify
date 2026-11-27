//! «Scaricati»: i brani sul dispositivo, che suonano anche senza rete, e i download in corso
//! (componenti, §19). In cima lo spazio occupato e la cartella; poi «In corso», con l'avanzamento a dieci
//! scatti, l'annulla e il riprova; poi «Sul dispositivo», con formato, peso e il tasto per toglierli.
//! Tutto viene dalla vista dei download del core (`app.downloads`): la pagina non calcola e non tocca il
//! disco, e le righe oltre il blocco mostrato non si costruiscono (`PAGE_ROWS` alla volta).

use iced::widget::{button, column, container, row};
use iced::{Alignment, Element, Length};
use psytify_core::snapshot::{Item, ItemState, LostReason, SavedTrack};

use crate::app::{App, MenuTarget, Message, Screen, TrackSource, is_phone};
use crate::ui::components::{
    Line, MUTED_LINE, PLAYING_LINE, SUB_LINE, TITLE_LINE, art_of, centered, count_label, empty_state,
    hoverable, icon_button, icon_button_side, line, muted, outlined_button, page_header, play_controls,
    section, size_label, step_bar, text_button,
};
use crate::ui::cover::cover;
use crate::ui::icons::Icon;
use crate::ui::theme;

/// Righe dell'elenco «Sul dispositivo» mostrate alla volta: il resto arriva con «Mostra altri».
pub const PAGE_ROWS: usize = 100;
/// Distanza fra la copertina, il testo e i tasti di una riga.
const GAP: f32 = 12.0;
/// Il testo di un errore: stessa misura della riga di dettaglio, con la tinta d'errore.
const ERROR_LINE: Line = Line { size: theme::FS_SMALL, font: theme::FONT, color: Some(theme::ERROR) };

pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let downloads = &app.downloads;
    let tracks = &app.downloaded;
    let subline = if downloads.verifying {
        "Controllo dei file…".to_owned()
    } else if tracks.is_empty() {
        "Nessun brano sul dispositivo".to_owned()
    } else {
        format!("{} · {}", count_label(tracks.len()), size_label(downloads.used_bytes))
    };
    let actions = (!tracks.is_empty())
        .then(|| play_controls(Message::PlayTracks(tracks.clone(), 0), Message::ShufflePlay(tracks.clone())));
    let mut page = column![page_header("Scaricati", subline, actions), storage(app, width)].spacing(24);
    if !downloads.items.is_empty() {
        page = page.push(in_progress(app, width));
    }
    page.push(on_device(app, width)).into()
}

/// Lo spazio e la cartella: dove finiscono i brani, in che qualità, e i due gesti sulla raccolta.
fn storage(app: &App, width: f32) -> Element<'_, Message> {
    let downloads = &app.downloads;
    let folder = downloads.folder.display().to_string();
    let mut actions = row![].spacing(8).align_y(Alignment::Center);
    // Il gestore di file non si apre dal telefono.
    if !is_phone() {
        actions = actions.push(outlined_button("Apri cartella", None, Some(Message::DownloadsOpenFolder)));
    }
    let can_verify = !downloads.verifying && !downloads.saved.is_empty();
    actions = actions.push(outlined_button(
        "Controlla i file",
        None,
        can_verify.then_some(Message::DownloadsVerify),
    ));
    container(
        column![
            line("Cartella dei download", width - 2.0 * theme::PAD, SUB_LINE),
            line(&folder, width - 2.0 * theme::PAD, MUTED_LINE),
            muted(format!("Qualità dei download: {}. Si cambia in Impostazioni.", downloads.quality.label())),
            actions,
        ]
        .spacing(8),
    )
    .padding(theme::PAD)
    .width(Length::Fill)
    .style(theme::card)
    .into()
}

/// I download in coda, in corso o falliti, nell'ordine in cui sono stati chiesti.
fn in_progress(app: &App, width: f32) -> Element<'_, Message> {
    let rows = column(app.downloads.items.iter().map(|item| progress_row(app, item, width))).spacing(4);
    section("In corso", None, rows.into())
}

fn progress_row<'a>(app: &'a App, item: &'a Item, width: f32) -> Element<'a, Message> {
    let track = &item.track;
    let side = icon_button_side();
    let text_width = width - 2.0 * theme::PAD - theme::ROW_COVER - 2.0 * GAP - 2.0 * side - 2.0;
    let (status, style) = match &item.state {
        ItemState::Queued => ("In coda".to_owned(), SUB_LINE),
        ItemState::Preparing => ("Preparazione…".to_owned(), SUB_LINE),
        ItemState::Downloading { .. } => (
            item.percent().map_or_else(|| "Scaricamento…".to_owned(), |percent| format!("{percent}%")),
            SUB_LINE,
        ),
        ItemState::Failed(error) => (format!("Non riuscito. {error}"), ERROR_LINE),
    };
    let mut text =
        column![line(&track.title, text_width, TITLE_LINE), line(&status, text_width, style)].spacing(3);
    if item.is_busy() {
        text = text.push(step_bar(item.percent()));
    }
    let mut actions = row![].spacing(2).align_y(Alignment::Center);
    if item.is_busy() {
        actions =
            actions.push(icon_button(Icon::Close, "Annulla", false, Some(Message::DownloadCancel(track.id))));
    } else {
        actions = actions
            .push(icon_button(Icon::Repeat, "Riprova", false, Some(Message::DownloadRetry(track.id))))
            .push(icon_button(
                Icon::Close,
                "Togli dall'elenco",
                false,
                Some(Message::DownloadCancel(track.id)),
            ));
    }
    container(
        row![cover(&app.covers, art_of(track), theme::ROW_COVER), text.width(Length::Fill), actions]
            .spacing(GAP)
            .align_y(Alignment::Center),
    )
    .padding([8.0, theme::PAD])
    .width(Length::Fill)
    .into()
}

/// I brani sul dispositivo, il più recente in cima; vuoto, una frase che dice come riempirlo.
fn on_device(app: &App, width: f32) -> Element<'_, Message> {
    let saved = &app.downloads.saved;
    if saved.is_empty() {
        return empty_state(
            "Niente sul dispositivo",
            "I brani che scarichi stanno qui e suonano anche senza rete. Il tasto Scarica sta accanto ai brani che la licenza lascia tenere.",
            Some(("Scopri il catalogo", Message::Navigate(Screen::Library))),
        );
    }
    let shown = saved.len().min(app.downloads_rows);
    let mut rows = column(
        saved.iter().take(shown).enumerate().map(|(index, entry)| saved_row(app, entry, index, width)),
    )
    .spacing(2);
    if saved.len() > shown {
        rows = rows.push(
            container(text_button("Mostra altri", Message::DownloadsShowMore)).padding([8.0, theme::PAD]),
        );
    }
    section("Sul dispositivo", None, rows.into())
}

fn saved_row<'a>(app: &'a App, entry: &'a SavedTrack, index: usize, width: f32) -> Element<'a, Message> {
    let track = &entry.track;
    let lost = app.downloads.lost_reason(track.id);
    let playing = app.is_current(track.id);
    let side = icon_button_side();
    let buttons = if lost.is_some() { 2.0 * side + 2.0 } else { side };
    let text_width = width - 2.0 * theme::PAD - theme::ROW_COVER - 2.0 * GAP - buttons;
    let (detail, style) = match lost {
        Some(LostReason::Missing) => (format!("{} · File mancante", track.artist.name), ERROR_LINE),
        Some(LostReason::Changed) => (format!("{} · File cambiato", track.artist.name), ERROR_LINE),
        None => (
            format!("{} · {} · {}", track.artist.name, entry.format.to_uppercase(), size_label(entry.size)),
            SUB_LINE,
        ),
    };
    let title_style = if playing { PLAYING_LINE } else { TITLE_LINE };
    let content = row![
        cover(&app.covers, art_of(track), theme::ROW_COVER),
        column![line(&track.title, text_width, title_style), line(&detail, text_width, style)]
            .spacing(3)
            .width(Length::Fill),
    ]
    .spacing(GAP)
    .align_y(Alignment::Center);
    let press =
        if playing { Message::TogglePlay } else { Message::PlayTracks(app.downloaded.clone(), index) };
    let body = button(centered(content))
        .height(theme::ROW_PLAIN)
        .width(Length::Fill)
        .padding([0.0, theme::PAD])
        .style(theme::row(playing))
        .on_press(press);
    let mut actions = row![].spacing(2).align_y(Alignment::Center);
    if lost.is_some() {
        actions = actions.push(icon_button(
            Icon::Download,
            "Scarica di nuovo",
            false,
            Some(Message::Download(track.clone())),
        ));
    }
    actions = actions.push(icon_button(
        Icon::Trash,
        "Rimuovi dal dispositivo",
        false,
        Some(Message::DownloadRemove(track.id)),
    ));
    let line_row = row![body, actions].align_y(Alignment::Center);
    hoverable(track.id, false, Some(MenuTarget::track(track, TrackSource::Catalogue)), line_row.into())
}
