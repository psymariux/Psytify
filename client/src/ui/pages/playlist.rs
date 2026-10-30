//! Playlist locale: stessa struttura della pagina album, con Rinomina ed Elimina.

use std::sync::Arc;

use iced::Element;
use iced::widget::column;
use uuid::Uuid;

use crate::api::Track;
use crate::app::{App, Dialog, Message, Screen};
use crate::ui::components::{
    HERO_COVER, Origin, TableKind, count_label, empty_block, empty_state, hero, icon_button, play_bar,
    skeleton_rows, small, total_label, track_table,
};
use crate::ui::cover::{Art, cover};
use crate::ui::icons::Icon;
use crate::ui::pages::album::credits;
use crate::ui::theme;

pub fn view(app: &App, id: Uuid, width: f32) -> Element<'_, Message> {
    let Some(playlist) = app.playlist(id) else {
        return empty_block("Questa playlist non esiste più.");
    };
    let tracks: Arc<[Track]> = Arc::from(playlist.tracks.as_slice());
    let total: u64 = tracks.iter().map(|t| t.duration_ms).sum();
    let meta = format!("{} · {}", count_label(tracks.len()), total_label(total));
    // La copertina è quella del primo brano; senza brani, un colore e l'iniziale del nome.
    let art = match tracks.first() {
        Some(first) => cover(&app.covers, crate::ui::components::art_of(first), HERO_COVER),
        None => cover(&app.covers, Art::drawn(playlist.id, &playlist.name), HERO_COVER),
    };
    let mut extra = vec![
        icon_button(
            Icon::Edit,
            "Rinomina",
            false,
            Some(Message::OpenDialog(Dialog::Rename { id, name: playlist.name.clone() })),
        ),
        icon_button(
            Icon::Trash,
            "Elimina la playlist",
            false,
            Some(Message::OpenDialog(Dialog::ConfirmDelete(id))),
        ),
    ];
    // Con un profilo, la playlist si può mostrare agli altri.
    if app.signed_in_profile().is_some() {
        let label = if playlist.public {
            "Pubblica sul profilo: tocca per renderla privata"
        } else {
            "Mostra sul profilo"
        };
        extra.push(icon_button(
            Icon::Account,
            label,
            playlist.public,
            Some(Message::PlaylistPublic(id, !playlist.public)),
        ));
    }
    let kicker = if playlist.public { "Playlist pubblica" } else { "Playlist" };
    let mut page =
        column![hero(art, kicker, &playlist.name, &meta, width), play_bar(app, &tracks, extra)].spacing(24);
    if !playlist.pending.is_empty() {
        let source = playlist.pending[0].source.label();
        page = page.push(small(format!(
            "Altri {} brani in arrivo da {source}: entrano uno alla volta, anche se chiudi l'app.",
            playlist.pending.len()
        )));
    }
    if tracks.is_empty() && !playlist.pending.is_empty() {
        return page.push(skeleton_rows(4, theme::ROW_MEDIA, TableKind::Mixed)).into();
    }
    if tracks.is_empty() {
        return page
            .push(empty_state(
                "Questa playlist è vuota",
                "Fai clic destro su un brano e scegli «Aggiungi a una playlist».",
                Some(("Cerca brani", Message::Navigate(Screen::Search))),
            ))
            .into();
    }
    page = page.push(track_table(app, &tracks, TableKind::Mixed, width, Origin::Playlist(id)));
    if let Some(credits) = credits(&tracks) {
        page = page.push(credits);
    }
    page.into()
}
