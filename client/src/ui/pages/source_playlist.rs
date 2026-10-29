//! Una playlist di YouTube o un album di Bandcamp, come una playlist di Spotify: la testa con il
//! titolo e l'autore, Riproduci e «Salva nella libreria», poi i brani. Ogni brano entra nel
//! catalogo quando lo si ascolta; salvare la playlist li importa tutti, uno alla volta.

use iced::widget::{column, row};
use iced::{Alignment, Element};

use crate::api::{SourceItem, SourceKind};
use crate::app::{App, Load, Message, Screen};
use crate::ui::components::{
    HERO_COVER, TableKind, count_label, empty_block, error_block, hero, outlined_button, primary_button,
    skeleton_rows, small, total_label,
};
use crate::ui::cover::{Art, cover};
use crate::ui::icons::Icon;
use crate::ui::pages::search::{art_id, external_row, play_action, source_detail};
use crate::ui::theme;

pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let Some(open) = &app.external.open else {
        return empty_block("Nessuna playlist aperta. Cercane una dalla ricerca.");
    };
    let playlist = &open.playlist;
    let art = cover(
        &app.covers,
        Art { id: art_id(&playlist.id), title: &playlist.title, url: playlist.cover_url.as_deref() },
        HERO_COVER,
    );
    let kicker = match playlist.source {
        SourceKind::Deezer => playlist.kind_label(),
        SourceKind::Bandcamp => "Album · Bandcamp",
        SourceKind::YouTube | SourceKind::SoundCloud => "Playlist · YouTube",
    };
    let (meta, ready) = match &open.items {
        Load::Ready(items) => {
            let total: u64 = items.iter().filter_map(|i| i.duration_ms).sum();
            let mut meta = vec![playlist.owner.clone(), count_label(items.len())];
            if total > 0 {
                meta.push(total_label(total));
            }
            (meta.join(" · "), !items.is_empty())
        }
        _ => (playlist.owner.clone(), false),
    };

    let back =
        outlined_button("Torna alla ricerca", Some(Icon::Search), Some(Message::Navigate(Screen::Search)));
    let mut actions = row![].spacing(10).align_y(Alignment::Center);
    if ready {
        actions =
            actions.push(primary_button("Riproduci", Some(Icon::Play), Message::PlaySourcePlaylist(0))).push(
                outlined_button("Salva nella libreria", Some(Icon::Plus), Some(Message::SaveSourcePlaylist)),
            );
    }
    actions = actions.push(back);
    if app.external.queued() > 0 {
        actions = actions.push(small(format!("{} brani in arrivo nella coda", app.external.queued())));
    }

    let mut page = column![hero(art, kicker, &playlist.title, &meta, width), actions].spacing(24);
    let note = if playlist.source == SourceKind::Deezer {
        "Il server trova l'audio quando premi play. Salvando, i brani entrano nella libreria uno alla \
         volta, anche se chiudi l'app."
    } else {
        "L'audio passa dal server: ogni brano entra nel catalogo quando lo ascolti, o tutti quando salvi la \
         playlist. Quelli senza licenza libera verificata si ascoltano ma non si scaricano."
    };
    page = page.push(small(note));
    let body: Element<'_, Message> = match &open.items {
        Load::Idle | Load::Loading => skeleton_rows(8, theme::ROW_MEDIA, TableKind::Mixed),
        Load::Failed(err) => error_block("la playlist", err),
        Load::Ready(items) if items.is_empty() => {
            empty_block("Questa playlist non ha brani che si possano ascoltare.")
        }
        Load::Ready(items) => {
            let text_width = width - 2.0 * 12.0 - 50.0 - 16.0 * 2.0 - 40.0;
            column(items.iter().enumerate().map(|(index, item)| track_row(app, index, item, text_width)))
                .into()
        }
    };
    page.push(body).into()
}

/// Un brano: il play suona la playlist da qui in poi.
fn track_row<'a>(app: &'a App, index: usize, item: &'a SourceItem, text_width: f32) -> Element<'a, Message> {
    let importing = app.external.importing(item.source, &item.id);
    external_row(
        app,
        Art { id: art_id(&item.id), title: &item.title, url: item.cover_url.as_deref() },
        &item.title,
        &source_detail(item),
        play_action(importing, Message::PlaySourcePlaylist(index)),
        text_width,
    )
}
