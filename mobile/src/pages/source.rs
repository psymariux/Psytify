//! An album of the official catalogue or of Bandcamp, or a YouTube playlist, as the GPUI page
//! (`desktop/src/app/source_page.rs`): the header with the cover, the kind, the title, the owner,
//! tracks and length; Riproduci and «Salva nella libreria»; how the audio arrives; then the tracks.
//! A track enters the server's catalogue when it is played (`ExternalCmd::PlayPlaylist`, one
//! command: the core imports and plays); saving imports them all, one at a time.

use psytify_core::Command;
use psytify_core::Snapshot;
use psytify_core::api::{SourceItem, SourceKind, SourcePlaylist};
use psytify_core::catalog::Load;
use psytify_core::command::ExternalCmd;
use psytify_core::snapshot::Importing;

use super::album::button;
use super::search::source_detail;
use super::{Art, Built, Header, Mark, PageAction, PageStatus, Row, RowState, Target};
use crate::downloads::DownloadMark;
use crate::format;

pub(super) fn build(playlist: &SourcePlaylist, snapshot: &Snapshot) -> Built {
    let mut page = Built::titled(kicker(playlist));
    page.action(PageAction::Retry, Target::Send(Command::External(ExternalCmd::RetryPlaylist)));
    let external = &snapshot.external;
    let open = external
        .open
        .as_ref()
        .filter(|open| open.playlist.source == playlist.source && open.playlist.id == playlist.id);
    let items = match open.map(|open| &open.items) {
        Some(Load::Ready(items)) => Some(items),
        Some(Load::Failed(error)) => {
            page.status = PageStatus::Failed { message: error.to_string() };
            None
        }
        _ => None,
    };
    let mut buttons = Vec::new();
    if items.is_some_and(|items| !items.is_empty()) {
        buttons.push(button(PageAction::Play, "Riproduci", true, false));
        buttons.push(button(PageAction::Save, "Salva nella libreria", false, false));
        let play = ExternalCmd::PlayPlaylist { start: 0, video: false };
        page.action(PageAction::Play, Target::Send(Command::External(play)));
        page.action(PageAction::Save, Target::SaveSource);
    }
    let queued = external.queued();
    let mut note = explanation(playlist.source).to_owned();
    if queued > 0 {
        note = format!("{} in arrivo nella coda. {note}", format::tracks(queued));
    }
    page.header = Some(Header {
        kicker: kicker(playlist).to_owned(),
        title: playlist.title.clone(),
        subtitle: None,
        subtitle_opens: false,
        meta: meta(playlist, items.map(|items| &items[..])),
        art: Art::Cover { url: playlist.cover_url.clone() },
        buttons,
        note: Some(note),
    });
    let Some(items) = items else { return page };
    page.status = if items.is_empty() {
        PageStatus::Empty { message: "Questa playlist non ha brani che si possano ascoltare.".into() }
    } else {
        PageStatus::Ready
    };
    for (index, item) in items.iter().enumerate() {
        let row = Row::Track {
            key: format!("item:{index}:{}", item.id),
            number: Some(u32::try_from(index + 1).unwrap_or(u32::MAX)),
            art: None,
            title: item.title.clone(),
            subtitle: source_detail(item),
            duration: item.duration_ms.map(format::clock).unwrap_or_default(),
            state: RowState::Idle,
            more: false,
            download: DownloadMark::None,
        };
        let play = ExternalCmd::PlayPlaylist { start: index, video: false };
        let mark = Mark::Import(Importing::Source(item.source, item.id.clone()));
        page.push(row, mark, Target::Send(Command::External(play)));
    }
    page
}

fn kicker(playlist: &SourcePlaylist) -> &'static str {
    match playlist.source {
        SourceKind::Deezer => playlist.kind_label(),
        SourceKind::Bandcamp => "Album · Bandcamp",
        SourceKind::YouTube | SourceKind::SoundCloud => "Playlist · YouTube",
    }
}

/// How the audio arrives (the texts of the Iced client).
fn explanation(source: SourceKind) -> &'static str {
    if source == SourceKind::Deezer {
        "Il server trova l'audio quando premi play. Salvando, i brani entrano nella libreria uno alla volta, \
         anche se chiudi l'app."
    } else {
        "L'audio passa dal server: ogni brano entra nel catalogo quando lo ascolti, o tutti quando salvi la \
         playlist. Quelli senza licenza libera verificata si ascoltano ma non si scaricano."
    }
}

/// «Autore · 12 brani · 48 min» (the length if the source declares it).
fn meta(playlist: &SourcePlaylist, items: Option<&[SourceItem]>) -> String {
    let Some(items) = items else { return playlist.owner.clone() };
    let total: u64 = items.iter().filter_map(|item| item.duration_ms).sum();
    let mut parts = vec![playlist.owner.clone(), format::tracks(items.len())];
    if total > 0 {
        parts.push(format::total(total));
    }
    parts.retain(|part| !part.is_empty());
    parts.join(" · ")
}
