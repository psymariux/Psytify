//! Pannello destro, apribile e chiudibile, con tre schede: «In riproduzione» (copertina o mini
//! videoclip, titolo, qualità, crediti, brano successivo), «Coda» e «Testo». Quello in riproduzione
//! prende la tinta della copertina.
//!
//! Le schede sono un selettore a pillola: il cursore scivola con la molla sotto la scheda scelta e
//! il contenuto nuovo entra salendo di qualche pixel.

use std::time::Duration;

use iced::widget::{Space, button, column, container, row, scrollable, stack, text};
use iced::{Alignment, Element, Length, Padding, alignment};

use super::components::{
    H1_LINE, ICON_BUTTON, PLAYING_LINE, SUB_LINE, TITLE_LINE, art_of, centered, count_label, duration_label,
    empty_block, h2, heart, hoverable, icon_button, line, muted, outlined_button, playing_bars, small,
    text_button, toggle_button, total_label,
};
use super::cover::{Art, cover, cover_round};
use super::icons::Icon;
use super::{focus, lyrics, motion, pages, theme};
use crate::api::Track;
use crate::app::{App, Dialog, Load, MenuTarget, Message, Screen, SideTab, TrackSource};
use crate::config::FollowedArtist;

/// Spazio utile dentro il pannello: larghezza meno il padding da 16 per lato.
const INNER: f32 = theme::SIDE_PANEL_WIDTH - 32.0;
/// Lato della foto dell'artista nella sua scheda.
const ARTIST_SIDE: f32 = 56.0;

/// Larghezza di ogni scheda del selettore, nell'ordine di `SideTab::ALL`: «In riproduzione» è lunga.
const TAB_WIDTHS: [f32; 3] = [128.0, 62.0, 68.0];
const TAB_HEIGHT: f32 = 32.0;
/// Spazio fra il binario e il cursore.
const TRACK_INSET: f32 = 2.0;
/// Il contenuto di una scheda appena scelta sale di tanto.
const TAB_RISE: f32 = 10.0;

pub fn view(app: &App) -> Element<'_, Message> {
    let header = row![
        tabs(app),
        Space::with_width(Length::Fill),
        icon_button(Icon::Close, "Chiudi il pannello", false, Some(Message::ClosePanel)),
    ]
    .align_y(Alignment::Center);

    let body = match app.side_tab {
        SideTab::NowPlaying => now_playing(app),
        SideTab::Queue => queue_body(app),
        SideTab::Lyrics => lyrics::view(app),
    };
    let entering = app.side_tab_motion.elapsed().as_secs_f32() / theme::MOTION_PANEL.as_secs_f32();
    let rise = TAB_RISE * (1.0 - motion::ease_out(entering));
    let body = container(body).padding(Padding { top: rise, ..Padding::ZERO }).height(Length::Fill);

    let content = container(column![header, body].spacing(14).height(Length::Fill))
        .width(theme::SIDE_PANEL_WIDTH)
        .height(Length::Fill)
        .padding(16);
    match (app.side_tab, app.queue.current()) {
        (SideTab::NowPlaying, Some(track)) => {
            let tint = app.covers.tint_of(art_of(track));
            // Mentre suona la sfumatura respira; in pausa sta ferma.
            let drift = if app.playback.is_playing() { motion::drift(Duration::from_secs(14)) } else { 0.0 };
            content.style(theme::aurora(tint, drift)).into()
        }
        _ => content.style(theme::panel_box).into(),
    }
}

/// Dove sta il cursore quando il selettore è a `value` (in schede, anche fra due o un po' oltre
/// con il rimbalzo della molla): distanza dall'inizio del binario e larghezza.
fn thumb_at(value: f32) -> (f32, f32) {
    let starts = [0.0, TAB_WIDTHS[0], TAB_WIDTHS[0] + TAB_WIDTHS[1]];
    let index = (value.floor() as usize).min(1);
    let t = value - index as f32;
    let x = starts[index] + (starts[index + 1] - starts[index]) * t;
    let width = TAB_WIDTHS[index] + (TAB_WIDTHS[index + 1] - TAB_WIDTHS[index]) * t;
    let total: f32 = TAB_WIDTHS.iter().sum();
    let width = width.clamp(TAB_WIDTHS[1], total);
    (x.clamp(0.0, total - width), width)
}

/// Il selettore delle schede: un binario, il cursore che scivola e le etichette sopra. L'etichetta
/// si schiarisce man mano che il cursore le arriva sotto.
fn tabs(app: &App) -> Element<'_, Message> {
    const LABELS: [&str; 3] = ["In riproduzione", "Coda", "Testo"];
    let value = app.side_tab_motion.value();
    let (x, width) = thumb_at(value);
    let thumb = container(container(Space::new(14, 3)).style(theme::bar(theme::ACCENT, 1.5)))
        .width(width)
        .height(TAB_HEIGHT - 2.0 * TRACK_INSET)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Bottom)
        .padding(Padding { bottom: 3.0, ..Padding::ZERO })
        .style(theme::segment_thumb);
    let slider = row![Space::with_width(x), thumb];

    let labels = SideTab::ALL.iter().zip(LABELS).zip(TAB_WIDTHS).map(|((&tab, label), width)| {
        let near = (1.0 - (value - tab.index()).abs()).clamp(0.0, 1.0);
        let font = if tab == app.side_tab { theme::FONT_SEMIBOLD } else { theme::FONT_MEDIUM };
        button(
            container(text(label).size(theme::FS_BODY).font(font).color(motion::mix(
                theme::NAV_IDLE,
                theme::TEXT,
                near,
            )))
            .center(Length::Fill),
        )
        .width(width)
        .height(TAB_HEIGHT - 2.0 * TRACK_INSET)
        .padding(0)
        .style(theme::bare)
        .on_press(Message::SideTab(tab))
        .into()
    });
    // Lo `stack` prende la misura del primo livello: entrambi i livelli sono larghi quanto il binario.
    let total: f32 = TAB_WIDTHS.iter().sum();
    container(stack![slider.width(total), row(labels).width(total)].width(total))
        .padding(TRACK_INSET)
        .style(theme::segment_track)
        .into()
}

// ---------- In riproduzione ----------

fn now_playing(app: &App) -> Element<'_, Message> {
    let Some(track) = app.queue.current() else {
        return empty_block("Scegli un brano: qui compaiono la copertina, il videoclip e i crediti.");
    };
    let media = media(app, track);
    let source = app.queue.position().map_or(TrackSource::Catalogue, TrackSource::Queue);
    let title = row![
        column![
            line(&track.title, INNER - ICON_BUTTON - 8.0, H1_LINE),
            button(line(&track.artist.name, INNER - ICON_BUTTON - 8.0, super::components::MUTED_LINE))
                .padding(0)
                .style(theme::text_action)
                .on_press(Message::Navigate(Screen::Artist(track.artist.id))),
        ]
        .spacing(4)
        .width(Length::Fill),
        heart(app, track, true),
    ]
    .spacing(8)
    .align_y(Alignment::Center);

    let mut details = column![small("Licenza"), muted(track.licence.label())].spacing(4);
    if !track.licence.attribution.is_empty() {
        details = details.push(small(track.licence.attribution.clone()));
    }
    let credits = container(details).padding(14).width(Length::Fill).style(theme::card);

    let mut page = column![
        media,
        column![
            hoverable(track.id, false, Some(MenuTarget::track(track, source)), title.into()),
            chips(track)
        ]
        .spacing(10),
        credits
    ]
    .spacing(16);
    if let Some(card) = artist_card(app, track) {
        page = page.push(card);
    }
    if let Some((position, next)) = app.queue.upcoming().next() {
        page = page.push(next_card(app, position, next));
    }
    scrollable(page).height(Length::Fill).style(theme::scroll).into()
}

/// Le etichette del brano: formato, «Lossless» (FLAC o WAV), durata, scaricabile.
fn chips(track: &Track) -> Element<'_, Message> {
    let chip = |label: String, style: fn(&iced::Theme) -> container::Style| -> Element<'_, Message> {
        container(text(label).size(theme::FS_SMALL).font(theme::FONT_MEDIUM))
            .padding([3, 10])
            .style(style)
            .into()
    };
    let format = track.audio_format.to_ascii_lowercase();
    let mut chips = row![chip(format.to_uppercase(), theme::chip)].spacing(6).align_y(Alignment::Center);
    if matches!(format.as_str(), "flac" | "wav") {
        chips = chips.push(chip("Lossless".into(), theme::accent_chip));
    }
    chips = chips.push(chip(duration_label(track.duration_ms), theme::chip));
    if track.downloadable {
        chips = chips.push(chip("Scaricabile".into(), theme::chip));
    }
    chips.into()
}

/// Il videoclip, se il brano ne ha uno e si è scelto di vederlo; altrimenti la copertina grande.
/// Sotto, la scelta fra videoclip e copertina (si ricorda), il testo sul video e il tutto schermo.
fn media<'a>(app: &'a App, track: &'a Track) -> Element<'a, Message> {
    let art = || container(focus::breathing_cover(app, track, INNER));
    if !app.video_available() {
        return art().into();
    }
    let shown: Element<'a, Message> = match pages::video::mini(app, INNER) {
        Some(video) if app.panel_video => video,
        _ if app.panel_video && app.video_loading() => stack![
            art(),
            container(
                container(small("Preparo il videoclip…"))
                    .padding([8, 12])
                    .width(Length::Fill)
                    .style(theme::scrim(1.0))
            )
            .width(INNER)
            .height(INNER)
            .align_y(alignment::Vertical::Bottom),
        ]
        .into(),
        _ if app.panel_video && app.video_broken() => column![
            art(),
            row![
                small("Il videoclip non si è aperto."),
                Space::with_width(Length::Fill),
                text_button("Riprova", Message::RetryVideo),
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(6)
        .into(),
        _ => art().into(),
    };

    let choice = |label: &'a str, value: bool| -> Element<'a, Message> {
        button(text(label).size(theme::FS_SMALL).font(theme::FONT_MEDIUM))
            .padding([4, 12])
            .style(theme::pill(app.panel_video == value))
            .on_press(Message::SetPanelVideo(value))
            .into()
    };
    let mut bar =
        row![choice("Videoclip", true), choice("Copertina", false), Space::with_width(Length::Fill)]
            .spacing(6)
            .align_y(Alignment::Center);
    if app.panel_video {
        bar = bar.push(toggle_button(
            Icon::Lyrics,
            if app.video_lyrics { "Nascondi il testo sul video" } else { "Mostra il testo sul video" },
            app.video_lyrics,
            Some(Message::ToggleVideoLyrics),
        ));
    }
    bar = bar.push(icon_button(
        Icon::Fullscreen,
        "Videoclip a tutto schermo (V)",
        false,
        Some(Message::ToggleVideoFocus),
    ));
    column![shown, bar].spacing(8).into()
}

/// «Informazioni sull'artista»: foto, fan e Segui, dal catalogo ufficiale. Il clic apre la sua
/// pagina. Nessuna scheda finché il catalogo non ha risposto.
fn artist_card<'a>(app: &'a App, track: &'a Track) -> Option<Element<'a, Message>> {
    let Some(Load::Ready(extras)) = app.external.artists.get(&track.artist.id) else { return None };
    let artist = &extras.artist;
    let width = INNER - 28.0 - ARTIST_SIDE - 12.0;
    let photo = cover_round(
        &app.covers,
        Art { id: track.artist.id, title: &artist.name, url: artist.image_url.as_deref() },
        ARTIST_SIDE,
    );
    let mut about = column![line(&artist.name, width, TITLE_LINE)].spacing(3).width(Length::Fill);
    if artist.fans > 0 {
        about = about.push(line(&fans_label(artist.fans), width, SUB_LINE));
    }
    let open = button(row![photo, about].spacing(12).align_y(Alignment::Center))
        .padding(0)
        .style(theme::bare)
        .on_press(Message::Navigate(Screen::Artist(track.artist.id)));
    let following = app.is_following(track.artist.id);
    let follow = outlined_button(
        if following { "Segui già" } else { "Segui" },
        Some(if following { Icon::Check } else { Icon::UserPlus }),
        Some(Message::ToggleFollow(FollowedArtist {
            id: track.artist.id,
            name: track.artist.name.clone(),
            cover_url: artist.image_url.clone(),
        })),
    );
    Some(
        container(column![h2("Informazioni sull'artista"), open, follow].spacing(12))
            .padding(14)
            .width(Length::Fill)
            .style(theme::card)
            .into(),
    )
}

/// «1,2 Mln di fan», «48.000 fan».
pub fn fans_label(fans: u64) -> String {
    match fans {
        0..1_000 => format!("{fans} fan"),
        1_000..1_000_000 => format!("{}.{:03} fan", fans / 1_000, fans % 1_000),
        _ => {
            let tenths = fans / 100_000;
            format!("{},{} Mln di fan", tenths / 10, tenths % 10)
        }
    }
}

fn next_card<'a>(app: &'a App, position: usize, track: &'a Track) -> Element<'a, Message> {
    let width = INNER - 28.0 - 48.0 - 12.0;
    let item = button(
        row![
            cover(&app.covers, art_of(track), 48.0),
            column![line(&track.title, width, TITLE_LINE), line(&track.artist.name, width, SUB_LINE)]
                .spacing(3)
                .width(Length::Fill),
        ]
        .spacing(12)
        .align_y(Alignment::Center),
    )
    .padding(0)
    .style(theme::bare)
    .on_press(Message::PlayQueueAt(position));
    container(
        column![
            row![
                h2("Prossimo in coda"),
                Space::with_width(Length::Fill),
                text_button("Apri la coda", Message::SideTab(SideTab::Queue))
            ]
            .align_y(Alignment::Center),
            item,
        ]
        .spacing(12),
    )
    .padding(14)
    .width(Length::Fill)
    .style(theme::card)
    .into()
}

// ---------- Coda ----------

pub(super) fn queue_body(app: &App) -> Element<'_, Message> {
    let queue = &app.queue;
    let summary = small(format!("{} · {}", count_label(queue.len()), total_label(queue.total_duration_ms())));
    let body: Element<'_, Message> = if queue.is_empty() {
        empty_block("La coda è vuota. Scegli un brano per iniziare.")
    } else {
        let mut list = column![].spacing(2);
        if let (Some(position), Some(current)) = (queue.position(), queue.current()) {
            list = list
                .push(container(small("In riproduzione")).padding([4, 8]))
                .push(entry(app, position, current, true));
        }
        let upcoming: Vec<Element<'_, Message>> =
            queue.upcoming().map(|(pos, track)| entry(app, pos, track, false)).collect();
        list = list.push(Space::with_height(12)).push(container(small("Successivi")).padding([4, 8]));
        list = if upcoming.is_empty() {
            list.push(container(small("Nessun altro brano.")).padding([4, 8]))
        } else {
            list.extend(upcoming)
        };
        scrollable(list).height(Length::Fill).style(theme::scroll).into()
    };

    let seed = queue.tracks_in_order();
    let footer = row![
        outlined_button(
            "Salva come playlist",
            Some(Icon::Plus),
            (!queue.is_empty()).then(|| Message::OpenDialog(Dialog::NewPlaylist {
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

/// Testo di una voce: pannello 340 meno padding 16 + 8 per lato, copertina 40, durata circa 36,
/// pulsante di rimozione e tre spazi da 10.
const ENTRY_TEXT: f32 = theme::SIDE_PANEL_WIDTH - 48.0 - 40.0 - 36.0 - ICON_BUTTON - 30.0;

fn entry<'a>(app: &App, position: usize, track: &'a Track, current: bool) -> Element<'a, Message> {
    let lead: Element<'a, Message> = if current {
        container(playing_bars(theme::ACCENT, app.playback.is_playing())).width(18).into()
    } else {
        Space::with_width(0).into()
    };
    let item = button(centered(
        row![
            lead,
            cover(&app.covers, art_of(track), 40.0),
            column![
                line(&track.title, ENTRY_TEXT, if current { PLAYING_LINE } else { TITLE_LINE }),
                line(&track.artist.name, ENTRY_TEXT, SUB_LINE),
            ]
            .spacing(3)
            .width(Length::Fill),
            small(duration_label(track.duration_ms)),
            icon_button(Icon::Close, "Togli dalla coda", false, Some(Message::RemoveFromQueue(position))),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    ))
    .height(56)
    .width(Length::Fill)
    .padding([0, 8])
    .style(theme::row(current))
    .on_press(Message::PlayQueueAt(position));
    let menu = MenuTarget::track(track, TrackSource::Queue(position));
    hoverable(track.id, true, Some(menu), item.into())
}
