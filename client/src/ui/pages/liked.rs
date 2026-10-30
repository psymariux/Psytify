//! «Brani che ti piacciono»: i cuori messi in giro per l'app, il più recente in cima.

use std::sync::Arc;

use iced::Element;
use iced::widget::column;

use crate::api::Track;
use crate::app::{App, Message, Screen};
use crate::ui::components::{
    HERO_COVER, Origin, TableKind, count_label, empty_state, hero, play_bar, total_label, track_table,
};
use crate::ui::pages::album::credits;
use crate::ui::sidebar::liked_cover;

pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let tracks: Arc<[Track]> = Arc::from(app.liked.as_slice());
    let total: u64 = tracks.iter().map(|t| t.duration_ms).sum();
    let meta = format!("{} · {}", count_label(tracks.len()), total_label(total));
    let mut page = column![
        hero(liked_cover(HERO_COVER), "Playlist", "Brani che ti piacciono", &meta, width),
        play_bar(app, &tracks, vec![]),
    ]
    .spacing(24);
    if tracks.is_empty() {
        return page
            .push(empty_state(
                "Ancora nessun cuore",
                "Premi il cuore accanto a un brano e lo ritrovi qui, su questo computer.",
                Some(("Scopri il catalogo", Message::Navigate(Screen::Library))),
            ))
            .into();
    }
    page = page.push(track_table(app, &tracks, TableKind::Mixed, width, Origin::Liked));
    if let Some(credits) = credits(&tracks) {
        page = page.push(credits);
    }
    page.into()
}
