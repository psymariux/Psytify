//! Pagina album: la testa è l'oggetto stesso, poi le tracce e i crediti di licenza.

use std::sync::Arc;

use iced::widget::{column, row};
use iced::{Alignment, Element, Length};
use uuid::Uuid;

use crate::api::{Album, Track};
use crate::app::{App, Load, Message};
use crate::ui::components::{
    HERO_COVER, Origin, TableKind, count_label, empty_block, error_block, h2, hero, play_bar, skeleton,
    skeleton_rows, small, total_label, track_table,
};
use crate::ui::cover::{Art, cover};
use crate::ui::theme;

pub fn view(app: &App, id: Uuid, width: f32) -> Element<'_, Message> {
    match &app.album {
        Some((current, Load::Ready(album))) if *current == id => loaded(app, album, width),
        Some((current, Load::Failed(err))) if *current == id => error_block("l'album", err),
        _ => column![header_skeleton(), skeleton_rows(8, theme::ROW_PLAIN, TableKind::Album)]
            .spacing(24)
            .into(),
    }
}

fn loaded<'a>(app: &'a App, album: &'a Album, width: f32) -> Element<'a, Message> {
    let tracks: Arc<[Track]> = Arc::from(album.tracks.as_slice());
    let total: u64 = tracks.iter().map(|t| t.duration_ms).sum();
    let mut meta = vec![album.artist.name.clone()];
    if let Some(year) = album.released_on.as_deref().and_then(|d| d.get(..4)) {
        meta.push(year.to_string());
    }
    meta.push(count_label(tracks.len()));
    if total > 0 {
        meta.push(total_label(total));
    }

    let art = cover(
        &app.covers,
        Art { id: album.id, title: &album.title, url: album.cover_url.as_deref() },
        HERO_COVER,
    );
    let mut page =
        column![hero(art, "Album", &album.title, &meta.join(" · "), width), play_bar(&tracks, vec![])]
            .spacing(24);
    if tracks.is_empty() {
        return page.push(empty_block("Questo album non ha ancora brani pubblicati.")).into();
    }
    page = page.push(track_table(app, &tracks, TableKind::Album, width, Origin::Catalogue));
    if let Some(credits) = credits(&tracks) {
        page = page.push(credits);
    }
    page.into()
}

fn header_skeleton<'a>() -> Element<'a, Message> {
    row![
        skeleton(Length::Fixed(HERO_COVER), HERO_COVER),
        column![skeleton(Length::Fixed(320.0), 40.0), skeleton(Length::Fixed(200.0), 14.0)].spacing(12),
    ]
    .spacing(24)
    .align_y(Alignment::End)
    .into()
}

/// Titolari da citare per le licenze che lo richiedono (CC BY, CC BY-SA).
pub fn credits<'a>(tracks: &[Track]) -> Option<Element<'a, Message>> {
    let lines: Vec<Element<'a, Message>> = tracks
        .iter()
        .filter(|t| t.licence.requires_attribution())
        .map(|t| {
            small(format!("{} — {} · {}", t.title, t.licence.label(), t.licence.attribution))
                .color(theme::MUTED)
                .into()
        })
        .collect();
    (!lines.is_empty()).then(|| column![h2("Crediti"), column(lines).spacing(6)].spacing(12).into())
}
