//! Passo 4.3b: il testo del brano, le radio, gli album, i mix e i collegamenti col sistema vivono nel
//! core. Qui si prova l'app intera con il core sincrono dei test: i lavori di rete che il core chiede
//! (`take_jobs`) si risolvono a mano, e ciò che il core chiede di mostrare al sistema o a Discord
//! (`take_system`) si legge dalla coda degli effetti.

use std::sync::Arc;

use super::{id, local, publish, track};
use crate::api::{Album, ArtistPage, Track};
use crate::app::{App, Load, Message, Notice, SideTab};
use crate::config::taste;
use crate::player::PlaybackState;
use psytify_core::command::LibraryCmd;
use psytify_core::media::Event;
use psytify_core::testing::PendingJob;

/// Un'app con una coda ripresa dall'ultima sessione: il brano corrente è il primo.
fn app_with_queue(tracks: Vec<Track>) -> App {
    App::for_tests_with(None, |config| Arc::make_mut(&mut config.queue).replace(tracks, 0, 1))
}

fn page(artist: &Track, tracks: Vec<Track>) -> ArtistPage {
    ArtistPage {
        id: artist.artist.id,
        name: artist.artist.name.clone(),
        image_url: None,
        albums: Vec::new(),
        tracks,
    }
}

fn lyrics_asked(app: &mut App) -> Vec<uuid::Uuid> {
    local(app)
        .take_jobs()
        .into_iter()
        .filter_map(|job| match job {
            PendingJob::Lyrics { track } => Some(track),
            _ => None,
        })
        .collect()
}

#[test]
fn the_lyrics_are_asked_only_while_the_panel_or_the_focus_view_shows_them() {
    let mut app = app_with_queue(vec![track(1, 60), track(2, 60)]);
    assert!(lyrics_asked(&mut app).is_empty(), "nessuno li mostra");

    // Il pannello sul «Testo»: il core li chiede e l'app mostra «in caricamento».
    let _ = app.update(Message::OpenPanel(SideTab::Lyrics));
    assert!(matches!(app.lyrics, Load::Loading));
    assert_eq!(lyrics_asked(&mut app), vec![id(1)]);

    // Arrivano: l'app li mostra, con la riga in corso da capo.
    let text = Arc::new(crate::player::lyrics::Text { lines: Vec::new(), plain: Some("ciao".into()) });
    let snapshot = local(&mut app).lyrics_loaded(id(1), Ok(Arc::clone(&text)));
    let _ = publish(&mut app, snapshot);
    assert!(matches!(&app.lyrics, Load::Ready(shown) if Arc::ptr_eq(shown, &text)));

    // Il pannello si chiude: un altro brano non chiede più niente.
    let _ = app.update(Message::ClosePanel);
    let _ = app.update(Message::Next);
    assert!(lyrics_asked(&mut app).is_empty());

    // La vista a tutto schermo li vuole di nuovo, e questa volta sono quelli del brano nuovo: il
    // testo vecchio non si vede nemmeno per un attimo.
    let _ = app.update(Message::ToggleFocus);
    assert!(matches!(app.lyrics, Load::Loading));
    assert_eq!(lyrics_asked(&mut app), vec![id(2)]);
}

#[test]
fn a_lyrics_failure_is_shown_and_retried_when_the_panel_is_opened_again() {
    let mut app = app_with_queue(vec![track(1, 60)]);
    let _ = app.update(Message::OpenPanel(SideTab::Lyrics));
    assert_eq!(lyrics_asked(&mut app), vec![id(1)]);
    let snapshot = local(&mut app).lyrics_loaded(id(1), Err(crate::error::AppError::Network));
    let _ = publish(&mut app, snapshot);
    assert!(matches!(app.lyrics, Load::Failed(_)));

    let _ = app.update(Message::ClosePanel);
    let _ = app.update(Message::OpenPanel(SideTab::Lyrics));
    assert_eq!(lyrics_asked(&mut app), vec![id(1)], "si riprova");
    assert!(matches!(app.lyrics, Load::Loading));
}

#[tokio::test(start_paused = true)]
async fn an_album_and_a_radio_play_through_the_core_with_their_notices() {
    let mut app = App::for_tests();
    let _ = app.update(Message::PlayAlbum(uuid::Uuid::from_u128(77)));
    let ticket = match local(&mut app).take_jobs().as_slice() {
        [PendingJob::Album { id, ticket }] => {
            assert_eq!(*id, uuid::Uuid::from_u128(77));
            *ticket
        }
        other => panic!("{other:?}"),
    };
    let album = Album {
        id: uuid::Uuid::from_u128(77),
        title: "Album".into(),
        released_on: None,
        artist: track(7, 60).artist,
        cover_url: None,
        tracks: vec![track(7, 60), track(8, 60)],
    };
    let snapshot = local(&mut app).album_loaded(ticket, Ok(album));
    let _ = publish(&mut app, snapshot);
    assert_eq!(app.queue.len(), 2);
    assert_eq!(app.playback.state, PlaybackState::Loading { track_id: id(7) });

    // La radio: l'avviso arriva quando parte, non quando si chiede.
    let start = track(1, 60);
    let _ = app.update(Message::StartRadio(Box::new(start.clone())));
    assert!(!matches!(&app.notice, Some(Notice::Info(text)) if text.starts_with("Radio")));
    let jobs = local(&mut app).take_jobs();
    let ticket = match jobs.iter().find(|job| matches!(job, PendingJob::Radio { .. })) {
        Some(PendingJob::Radio { start, artist, ticket }) => {
            assert_eq!((*start, *artist), (id(1), track(1, 60).artist.id));
            *ticket
        }
        other => panic!("{other:?} in {jobs:?}"),
    };
    let own = vec![track(4, 60), track(7, 60), track(10, 60)];
    let snapshot = local(&mut app).radio_loaded(ticket, start.clone(), Ok(page(&start, own)));
    let _ = publish(&mut app, snapshot);
    assert_eq!(app.queue.current().map(|t| t.id), Some(id(1)));
    assert!(app.queue.len() > 1);
    assert_eq!(app.notice, Some(Notice::Info("Radio di «Brano 1»".into())));
}

#[test]
fn a_mix_of_the_home_plays_from_the_core() {
    let mut app = App::for_tests();
    let recent: Vec<Track> = (1..=12).map(|n| track(n, 180)).collect();
    let now = taste::now_secs();
    for (at, plays) in [(0, 3), (1, 2), (2, 1)] {
        for _ in 0..plays {
            app.send_library_quiet(LibraryCmd::CountPlay { track: recent[at].clone(), at: now });
        }
    }
    super::home_answered(&mut app, recent.clone(), recent);
    let mix = app.home.feed.mixes.first().expect("un mix per l'artista più ascoltato").tracks.clone();

    let _ = app.update(Message::PlayMix(0));
    assert_eq!(app.queue.len(), mix.len());
    assert_eq!(app.queue.current().map(|t| t.id), Some(mix[0].id));
    assert!(matches!(app.playback.state, PlaybackState::Loading { .. }));
}

#[test]
fn media_keys_reach_the_core_without_the_ui() {
    let mut app = app_with_queue(vec![track(1, 60), track(2, 60)]);
    let snapshot = local(&mut app).media(Event::Toggle);
    let _ = publish(&mut app, snapshot);
    assert_eq!(app.playback.state, PlaybackState::Loading { track_id: id(1) });

    let snapshot = local(&mut app).media(Event::Next);
    let _ = publish(&mut app, snapshot);
    assert_eq!(app.queue.current().map(|t| t.id), Some(id(2)));
}
