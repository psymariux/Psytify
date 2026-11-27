//! Impaginazione da telefono: un'app pensata per il pollice, non il desktop ristretto. Le quattro
//! schede (Home, Cerca, Libreria, Novità) hanno un titolo grande in cima alla pagina e nessuna
//! barra sopra; le pagine aperte da un'altra (album, artista, impostazioni) hanno solo «Indietro».
//! La pagina va da bordo a bordo sulla tela nera, piatta.
//! In basso il mini-player (un tocco apre il player) e la barra delle schede.
//!
//! Su Android la barra di stato e quella dei gesti stanno sopra l'app: si lascia il loro spazio.
//! In Cerca il campo sta in cima alla pagina e la tastiera del sistema si apre entrando
//! (`App::update`).

pub mod drawer;
pub mod home;
pub mod library;
mod player;
pub mod queue;

use std::sync::Arc;

use iced::widget::{Space, button, column, container, row, scrollable, stack, text};
use iced::{Alignment, Element, Length, Padding, Size};

use super::components::{Line, art_of, heart, line};
use super::cover::{Art, cover, cover_round};
use super::gesture::area::area;
use super::gesture::memory;
use super::gesture::recognizer::{Axis, Output};
use super::gesture::sheet::sheet;
use super::gesture::swipe::{Reveal, swipe_row};
use super::icons::{Icon, icon};
use super::theme;
use crate::api::Track;
use crate::app::{Account, App, Message, PhoneMsg, Screen, SideTab, phone_tab_of};
use crate::player::seek::progress as progress_of;

/// Lato della copertina nel mini-player.
const MINI_COVER: f32 = psytify_design::tokens::size::cover::COVER_S;
/// Titolo e artista nel mini-player: due righe della stessa scala, come la barra alta quanto una scheda.
const MINI_TITLE: Line = Line { size: theme::FS_SMALL, font: theme::FONT_MEDIUM, color: Some(theme::TEXT) };
const MINI_SUB: Line = Line { size: theme::FS_SMALL, font: theme::FONT, color: Some(theme::MUTED) };
/// Spazio fra il fondo della pagina e il mini-player: l'ultima riga non finisce sotto le barre.
const PAGE_BOTTOM: f32 = 32.0;

/// Lo spazio delle barre del sistema: quello che Android dice (barra di stato, gesti o tre
/// pulsanti, notch); finché non lo dice, una stima. Sul desktop niente.
fn insets(app: &App) -> Padding {
    match app.insets {
        Some(found) => Padding { top: found.top, right: found.right, bottom: found.bottom, left: found.left },
        None if cfg!(target_os = "android") => {
            Padding { top: theme::PHONE_STATUS_INSET, bottom: theme::PHONE_GESTURE_INSET, ..Padding::ZERO }
        }
        None => Padding::ZERO,
    }
}

/// Il foglio del player: sopra la pagina (che si disegna solo mentre il foglio non è del tutto
/// aperto), con l'ingresso a molla e il trascinamento che lo chiude.
fn player_sheet(app: &App, window: Size) -> Element<'_, Message> {
    let edges = insets(app);
    let under: Element<'_, Message> =
        if app.phone.underlay { page(app, window) } else { Space::new(Length::Fill, Length::Fill).into() };
    // Dove il dito può afferrare il foglio: sulla copertina tutta la parte alta (a parte il bordo del
    // volume), sul testo e sulla coda solo l'intestazione, perché sotto scorrono le liste.
    let (grab, except_right) = match app.side_tab {
        SideTab::NowPlaying => (window.height * 0.55, 64.0),
        _ => (edges.top + 150.0, 0.0),
    };
    let foglio = sheet(
        player::view(app, window, edges),
        Message::Phone(PhoneMsg::SheetDragging),
        Message::Phone(PhoneMsg::SheetSettled),
        Message::Phone(PhoneMsg::SheetClosed),
    )
    .enter(app.phone.underlay)
    .dismiss(app.phone.dismiss)
    .grab(grab)
    .except_right(except_right);
    stack![under, foglio].width(Length::Fill).height(Length::Fill).into()
}

pub fn view(app: &App, window: Size) -> Element<'_, Message> {
    if app.focus && app.queue.current().is_some() {
        return player_sheet(app, window);
    }
    page(app, window)
}

/// Una chiave per la memoria dello scorrimento di una schermata.
pub fn page_key(screen: Screen) -> u64 {
    let (label, id) = match screen {
        Screen::Album(id) => ("album", Some(id.as_u128())),
        Screen::Playlist(id) => ("playlist", Some(id.as_u128())),
        Screen::Artist(id) => ("artist", Some(id.as_u128())),
        Screen::Home => ("home", None),
        Screen::Search => ("search", None),
        Screen::Library => ("library", None),
        Screen::Settings => ("settings", None),
        Screen::Jam => ("jam", None),
        Screen::Liked => ("liked", None),
        Screen::Downloads => ("downloads", None),
        Screen::News => ("news", None),
        Screen::SourcePlaylist => ("source", None),
        Screen::Collection => ("collection", None),
        Screen::SignIn => ("signin", None),
        Screen::Friends => ("friends", None),
        Screen::Profile => ("profile", None),
        Screen::FriendPlaylist => ("friendplaylist", None),
    };
    memory::key_of(label, id)
}

/// Una riga di brani con le azioni a scorrimento: a destra «aggiungi alla coda», a sinistra «mi
/// piace» (cuore bianco). Fuori dal telefono la riga resta com'è.
pub fn swipe_track<'a>(
    app: &App,
    tracks: &Arc<[Track]>,
    index: usize,
    content: Element<'a, Message>,
) -> Element<'a, Message> {
    if !crate::app::is_phone() {
        return content;
    }
    let Some(track) = tracks.get(index) else {
        return content;
    };
    let liked = app.is_liked(track.id);
    let zone = |glyph: Icon, color| -> Element<'a, Message> {
        container(icon(glyph, 0.0, color)).center(Length::Fill).into()
    };
    // Chi sta per piacere mostra il cuore pieno; chi sta per non piacere più, il contorno.
    let heart = if liked { Icon::Heart } else { Icon::HeartFill };
    swipe_row(content)
        .on_right(Reveal {
            idle: zone(Icon::Queue, theme::TEXT),
            armed: zone(Icon::Queue, theme::ON_PRIMARY),
            message: Message::Phone(PhoneMsg::EnqueueAt(tracks.clone(), index)),
            background: theme::SELECTED,
            armed_background: theme::TEXT,
        })
        .on_left(Reveal {
            idle: zone(heart, theme::TEXT),
            armed: zone(heart, theme::ON_PRIMARY),
            message: Message::Phone(PhoneMsg::LikeAt(tracks.clone(), index)),
            background: theme::SELECTED,
            armed_background: theme::TEXT,
        })
        .into()
}

/// La pagina del telefono: scheda o pagina aperta, mini-player e barra delle schede.
fn page(app: &App, window: Size) -> Element<'_, Message> {
    let edges = insets(app);
    let full = (window.width - edges.left - edges.right).max(280.0);
    let inner = full - 2.0 * theme::PHONE_GUTTER;
    let root = phone_tab_of(app.screen) == Some(app.screen);

    // La Home impagina da sé i margini: le file di copertine scorrono fino al bordo dello schermo.
    // Le altre pagine stanno fra i due margini.
    let (body, gutter) = match app.screen {
        Screen::Home => (home::view(app, full), 0.0),
        _ => (super::page(app, inner), theme::PHONE_GUTTER),
    };
    // Una pagina nuova entra con un taglio e una dissolvenza, come sul desktop.
    let rise = theme::PAGE_RISE * (1.0 - app.page_motion.value());
    let top = if root { 12.0 } else { 4.0 };
    let content = scrollable(container(body).width(Length::Fill).padding(Padding {
        top: top + rise,
        bottom: PAGE_BOTTOM,
        left: gutter,
        right: gutter,
    }))
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::scroll);
    // Tornando a una scheda o a una pagina lo scorrimento riparte da dov'era.
    let content = memory::remember(page_key(app.screen), true, content);

    let mut screen = column![].height(Length::Fill);
    if !root {
        screen = screen.push(back_bar(app));
    }
    // Una versione nuova si vede in cima a ogni scheda, non solo in Impostazioni.
    if root && let Some(banner) = crate::ui::update_banner::view(app, true) {
        screen = screen.push(banner);
    }
    // In Cerca il titolo e il campo restano fermi sopra i risultati, come nelle app da telefono.
    if app.screen == Screen::Search {
        screen = screen.push(search_header(app));
    }
    screen = screen.push(content);
    if let Some(mini) = mini_player(app, full) {
        screen = screen.push(mini);
    }
    let page = container(screen.push(nav(app))).width(Length::Fill).height(Length::Fill).padding(Padding {
        top: edges.top,
        left: edges.left,
        right: edges.right,
        bottom: 0.0,
    });
    // La barra dei gesti resta sul chrome della barra delle schede.
    container(column![
        container(page).width(Length::Fill).height(Length::Fill).style(theme::canvas),
        container(Space::new(Length::Fill, edges.bottom)).style(theme::phone_nav),
    ])
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::ground)
    .into()
}

/// Titolo grande di una scheda, con le azioni a destra: la prima riga delle quattro schermate
/// principali.
pub fn title_row<'a>(title: &'a str, actions: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    let mut bar =
        row![text(title).size(theme::FS_PHONE_TITLE).font(theme::FONT_SEMIBOLD).width(Length::Fill)]
            .spacing(4)
            .align_y(Alignment::Center)
            .height(theme::TOUCH_TARGET);
    for action in actions {
        bar = bar.push(action);
    }
    bar.into()
}

/// Un'icona da toccare, del lato minimo di un tocco.
pub fn touch_icon<'a>(glyph: Icon, color: iced::Color, on_press: Message) -> Element<'a, Message> {
    button(container(icon(glyph, 0.0, color)).center(Length::Fill))
        .width(theme::TOUCH_TARGET)
        .height(theme::TOUCH_TARGET)
        .padding(0)
        .style(theme::bare)
        .on_press(on_press)
        .into()
}

/// L'avatar di chi è collegato (la foto, o il segnaposto tondo con l'icona dell'account).
pub fn avatar<'a>(app: &App, size: f32) -> Element<'a, Message> {
    match app.signed_in_profile() {
        Some(profile) => cover_round(
            &app.covers,
            Art {
                id: crate::ui::pages::search::art_id(&profile.handle),
                title: &profile.display_name,
                url: profile.avatar_url.as_deref(),
            },
            size,
        ),
        // Senza accesso: il segnaposto tondo con l'icona dell'account.
        None => cover_round(&app.covers, Art::drawn(uuid::Uuid::nil(), ""), size),
    }
}

/// Gli amici, dalla Home e dalla Libreria; un punto bianco se c'è una richiesta da guardare.
pub fn friends_icon(app: &App) -> Element<'_, Message> {
    let glyph = touch_icon(Icon::Account, theme::TEXT, Message::Navigate(Screen::Friends));
    if app.friend_requests() == 0 {
        return glyph;
    }
    iced::widget::stack![
        glyph,
        container(container(Space::new(7.0, 7.0)).style(theme::bar(theme::TEXT))).padding(Padding {
            top: 9.0,
            left: 29.0,
            ..Padding::ZERO
        }),
    ]
    .into()
}

/// Le impostazioni, dall'angolo in alto a destra della Home e della Libreria.
pub fn settings_icon<'a>() -> Element<'a, Message> {
    touch_icon(Icon::Settings, theme::TEXT, Message::Navigate(Screen::Settings))
}

/// La barra delle pagine aperte da un'altra: solo «Indietro». Una pagina riaperta all'avvio non
/// ha dove tornare: porta alla Home.
fn back_bar(app: &App) -> Element<'_, Message> {
    let back = if app.can_go_back() { Message::GoBack } else { Message::Navigate(Screen::Home) };
    container(touch_icon(Icon::ChevronLeft, theme::TEXT, back))
        .height(theme::PHONE_TOP_HEIGHT)
        .padding(Padding { left: 4.0, ..Padding::ZERO })
        .align_y(Alignment::Center)
        .into()
}

/// Cerca: il titolo e il campo, fermi sopra i risultati.
fn search_header(app: &App) -> Element<'_, Message> {
    column![title_row("Cerca", Vec::new()), super::top_bar::search_field(app, Length::Fill, false),]
        .spacing(10)
        .padding(Padding { top: 12.0, bottom: 12.0, left: theme::PHONE_GUTTER, right: theme::PHONE_GUTTER })
        .into()
}

/// Le cinque schede. Quella accesa ha l'icona piena, il testo bianco e il segnalino lime in cima.
/// Novità ha un punto bianco quando ci sono uscite da ascoltare; Profilo quando ci sono richieste.
fn nav(app: &App) -> Element<'_, Message> {
    let tab = app.phone_tab();
    let unread = app.unread_news() > 0;
    let requests = app.friend_requests() > 0;
    let items = [
        (Icon::Home, Icon::HomeFill, "Home", Screen::Home, Message::Navigate(Screen::Home)),
        (Icon::Search, Icon::SearchFill, "Cerca", Screen::Search, Message::Navigate(Screen::Search)),
        (
            Icon::Library,
            Icon::LibraryFill,
            "Libreria",
            Screen::Collection,
            Message::Navigate(Screen::Collection),
        ),
        (Icon::Account, Icon::Account, "Profilo", Screen::Profile, profile_target(app)),
        (Icon::Bell, Icon::BellFill, "Novità", Screen::News, Message::Navigate(Screen::News)),
    ];
    let buttons = items.into_iter().map(|(idle, filled, label, screen, press)| {
        let active = tab == screen;
        let color = if active { theme::TEXT } else { theme::FAINT };
        let mut glyph = row![icon(if active { filled } else { idle }, 0.0, color)];
        let dot = match screen {
            Screen::News => unread,
            Screen::Profile => requests,
            _ => false,
        };
        if dot {
            glyph = glyph.push(
                container(Space::new(7.0, 7.0))
                    .style(theme::bar(theme::TEXT))
                    .padding(Padding { bottom: 16.0, ..Padding::ZERO }),
            );
        }
        // Il segnalino lime sul bordo alto della scheda scelta (spazio uguale per le altre).
        let marker = container(Space::new(Length::Fill, 2.0 * theme::AP)).style(theme::bar(if active {
            theme::LIME
        } else {
            iced::Color::TRANSPARENT
        }));
        button(column![
            marker,
            container(
                column![glyph, text(label).size(11.0).font(theme::FONT_MEDIUM).color(color)]
                    .spacing(4)
                    .align_x(Alignment::Center),
            )
            .center(Length::Fill),
        ])
        .width(Length::Fill)
        .height(theme::PHONE_NAV_HEIGHT)
        .padding(0)
        .style(theme::bare)
        .on_press(press)
        .into()
    });
    container(row(buttons)).width(Length::Fill).style(theme::phone_nav).into()
}

/// Dove porta la scheda Profilo: il proprio profilo se c'è, altrimenti le impostazioni o l'accesso.
pub(super) fn profile_target(app: &App) -> Message {
    match &app.social.view.account {
        Account::SignedIn(me) => match &me.profile {
            Some(profile) => Message::OpenProfile(profile.handle.clone()),
            None => Message::Navigate(Screen::Settings),
        },
        Account::SignedOut | Account::Working => Message::SignIn,
        Account::Unavailable => Message::Navigate(Screen::Settings),
    }
}

/// Il brano in corso sopra le schede, sul chrome: un tocco apre il player.
/// Sul fondo, una linea sottile con l'avanzamento.
fn mini_player(app: &App, window: f32) -> Option<Element<'_, Message>> {
    let track = app.queue.current()?;
    // Fascia piena larghezza, appoggiata sulla barra delle schede: la copertina sta sul margine
    // della pagina, la linea d'avanzamento corre sul fondo da bordo a bordo.
    let inner = window - 2.0 * theme::PHONE_GUTTER;
    let text_width = inner - MINI_COVER - 3.0 * 10.0 - 2.0 * theme::TOUCH_TARGET;
    let glyph = if app.playback.is_playing() { Icon::Pause } else { Icon::Play };
    let play = button(container(icon(glyph, 0.0, theme::TEXT)).center(Length::Fill))
        .width(theme::TOUCH_TARGET)
        .height(theme::TOUCH_TARGET)
        .padding(0)
        .style(theme::bare)
        .on_press(Message::TogglePlay);
    let body = row![
        cover(&app.covers, art_of(track), MINI_COVER),
        column![line(&track.title, text_width, MINI_TITLE), line(&track.artist.name, text_width, MINI_SUB)]
            .spacing(2)
            .width(Length::Fill),
        heart(app, track, true),
        play,
    ]
    .spacing(10)
    .align_y(Alignment::Center);
    let open =
        button(container(body).padding(Padding::from([0.0, theme::PHONE_GUTTER])).center_y(Length::Fill))
            .padding(0)
            .height(Length::Fill)
            .width(Length::Fill)
            .style(theme::bare)
            .on_press(Message::Phone(PhoneMsg::OpenPlayer));
    // Uno scorrimento in su apre il player come un tocco: oltre 40 dp, o un lancio.
    let open = area(open, |gesture| match gesture {
        Output::PanEnd { axis: Axis::Vertical, total } if total.y < -40.0 => {
            Some(Message::Phone(PhoneMsg::OpenPlayer))
        }
        Output::Fling { axis: Axis::Vertical, velocity, .. } if velocity.y < 0.0 => {
            Some(Message::Phone(PhoneMsg::OpenPlayer))
        }
        _ => None,
    })
    .axis(Axis::Vertical);

    let fraction = progress_of(app.shown_position(), track.duration()).clamp(0.0, 1.0);
    let progress = container(
        container(Space::new(Length::Fixed(window * fraction), theme::AP)).style(theme::bar(theme::TEXT)),
    )
    .width(window)
    .style(theme::bar(theme::SELECTED));

    Some(
        container(column![open, progress])
            .height(theme::PHONE_MINI_HEIGHT)
            .width(Length::Fill)
            .style(theme::mini_player)
            .into(),
    )
}
