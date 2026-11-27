//! Passo 4.9: il videoclip vive nel core. Qui si prova l'app intera con il core sincrono dei test:
//! quando il pannello (o la vista a tutto schermo) mostra il video l'app lo dice al core, che chiede
//! l'indirizzo al server, arma il timer del 720p, prepara i videoclip dei brani in arrivo e ferma
//! quello che non si apre; l'app apre il player su ciò che il core pubblica. GStreamer qui non gira:
//! il player resta senza video (`player: None`), e ciò che si prova è la strada fino a lì.

use std::sync::Arc;

use super::{app_with_audio, loaded, local, publish, tone, track};
use crate::api::{SignedUrl, Track};
use crate::app::{App, Message, Notice, SideTab};
use crate::error::AppError;
use psytify_core::snapshot::ClipState;
use psytify_core::testing::PendingJob;

fn clip_track(n: u128) -> Track {
    Track { has_video: true, ..track(n, 60) }
}

/// Un'app con la coda ripresa dall'ultima sessione, ferma sul primo brano: i due hanno il videoclip.
fn app_with_clips() -> App {
    App::for_tests_with(None, |config| {
        Arc::make_mut(&mut config.queue).replace(vec![clip_track(1), clip_track(2)], 0, 1);
    })
}

fn signed(height: Option<u32>) -> SignedUrl {
    SignedUrl {
        kind: "video".into(),
        url: "https://media.test/v1/media?t=firmato".into(),
        expires_at: "2099-01-01T00:00:00Z".into(),
        height,
        format: None,
        quality: None,
    }
}

/// Gli indirizzi chiesti al server dall'ultima volta: (brano, numero della richiesta).
fn urls_asked(app: &mut App) -> Vec<(uuid::Uuid, u64)> {
    local(app)
        .take_jobs()
        .into_iter()
        .filter_map(|job| match job {
            PendingJob::VideoUrl { track, ticket } => Some((track, ticket)),
            _ => None,
        })
        .collect()
}

fn prepares_asked(app: &mut App) -> Vec<uuid::Uuid> {
    local(app)
        .take_jobs()
        .into_iter()
        .filter_map(|job| match job {
            PendingJob::VideoPrepare { track } => Some(track),
            _ => None,
        })
        .collect()
}

/// Il server risponde alla richiesta `ticket` dell'indirizzo.
fn answered(app: &mut App, ticket: u64, result: Result<SignedUrl, AppError>) {
    let snapshot = local(app).video_url_loaded(ticket, result);
    let _ = publish(app, snapshot);
}

fn state(app: &App) -> ClipState {
    app.core_snapshot.video.state.clone()
}

#[test]
fn the_clip_is_asked_while_the_panel_shows_it_and_dropped_when_it_does_not() {
    let mut app = app_with_clips();
    assert!(urls_asked(&mut app).is_empty(), "nessuno lo mostra");

    // Il pannello «In riproduzione» sul video: il core chiede l'indirizzo del brano in corso, e
    // l'app mostra «Preparo il videoclip…».
    let _ = app.update(Message::OpenPanel(SideTab::NowPlaying));
    let asked = urls_asked(&mut app);
    assert_eq!(asked.iter().map(|(track, _)| *track).collect::<Vec<_>>(), [super::id(1)]);
    assert_eq!(state(&app), ClipState::Loading);
    assert!(app.video_loading() && app.video.is_none());

    // L'indirizzo arriva a 360p: il player si apre su di lui (qui senza GStreamer) e il core aspetta
    // la versione completa.
    answered(&mut app, asked[0].1, Ok(signed(Some(360))));
    let stage = app.video.as_ref().expect("il player si apre sul videoclip pubblicato");
    assert_eq!((stage.track_id, stage.seq), (super::id(1), 1));
    assert!(local(&mut app).video_upgrade_pending());

    // Il pannello torna alla copertina: il player si chiude e il core non aspetta più niente.
    let _ = app.update(Message::SetPanelVideo(false));
    assert!(app.video.is_none());
    assert_eq!(state(&app), ClipState::Idle);
    assert!(!local(&mut app).video_upgrade_pending());
    assert!(urls_asked(&mut app).is_empty());
}

#[test]
fn the_full_version_arrives_from_the_core_timer_as_a_new_clip() {
    let mut app = app_with_clips();
    let _ = app.update(Message::OpenPanel(SideTab::NowPlaying));
    let (_, ticket) = urls_asked(&mut app)[0];
    answered(&mut app, ticket, Ok(signed(Some(360))));

    // Il timer del core scade e il core richiede l'indirizzo; il server ha il 720p.
    let snapshot = local(&mut app).fire_video_upgrade();
    let _ = publish(&mut app, snapshot);
    let (_, ticket) = urls_asked(&mut app)[0];
    answered(&mut app, ticket, Ok(signed(Some(720))));
    let ClipState::Ready(clip) = state(&app) else { panic!("il videoclip pronto") };
    assert_eq!((clip.seq, clip.height), (2, Some(720)));
    assert_eq!(app.video.as_ref().map(|stage| stage.seq), Some(2), "il player riparte dal videoclip nuovo");
    assert!(!local(&mut app).video_upgrade_pending(), "la versione completa non aspetta altro");
}

#[test]
fn a_refused_clip_shows_a_notice_and_retry_asks_again() {
    let mut app = app_with_clips();
    let _ = app.update(Message::OpenPanel(SideTab::NowPlaying));
    let (_, ticket) = urls_asked(&mut app)[0];
    let refusal =
        AppError::Api { status: 404, code: "not_found".into(), message: "Videoclip non trovato.".into() };
    answered(&mut app, ticket, Err(refusal));

    assert!(app.video_broken() && !app.video_loading());
    assert!(matches!(&app.notice, Some(Notice::Error(text)) if text == "Videoclip non trovato."));
    assert!(app.video.is_none());
    assert!(urls_asked(&mut app).is_empty(), "un videoclip fermo non si ripete da solo");

    // «Riprova»: il core lo chiede da capo.
    let _ = app.update(Message::RetryVideo);
    assert_eq!(urls_asked(&mut app).len(), 1);
    assert!(!app.video_broken() && app.video_loading());
}

#[test]
fn the_server_prepares_the_clips_that_follow_once_a_track_plays() {
    let (mut app, _mixer) = app_with_audio();
    // All'avvio l'app dice al core che cosa vuole: i videoclip in anticipo (la scelta salvata è sì).
    let _ = app.video_wants();
    let _ = app.update(Message::PlayTracks(Arc::from(vec![clip_track(1), clip_track(2), clip_track(3)]), 0));
    assert!(prepares_asked(&mut app).is_empty(), "il brano non è ancora partito");
    let generation = app.playback.generation;
    let _ = loaded(&mut app, generation, Ok(tone(4)));
    assert_eq!(prepares_asked(&mut app), [super::id(1), super::id(2), super::id(3)]);

    // Scelto il pannello sulla copertina il core non prepara più niente; ogni brano una volta.
    let _ = app.update(Message::SetPanelVideo(false));
    let _ = app.update(Message::Next);
    let generation = app.playback.generation;
    let _ = loaded(&mut app, generation, Ok(tone(4)));
    assert!(prepares_asked(&mut app).is_empty());
}

#[test]
fn changing_the_decoder_asks_the_core_to_reopen_the_clip() {
    let mut app = app_with_clips();
    let _ = app.update(Message::OpenPanel(SideTab::NowPlaying));
    let (_, ticket) = urls_asked(&mut app)[0];
    answered(&mut app, ticket, Ok(signed(Some(720))));
    assert!(app.video.is_some());

    let _ = app.update(Message::SetDedicatedVideo(true));
    assert!(app.dedicated_video);
    assert_eq!(state(&app), ClipState::Loading, "il core chiede l'indirizzo da capo");
    assert!(app.video.is_none(), "il player vecchio si chiude");
    assert_eq!(urls_asked(&mut app).len(), 1);
}

#[test]
fn a_clip_that_does_not_open_is_reported_to_the_core_which_retries_then_blocks() {
    let mut app = app_with_clips();
    let _ = app.update(Message::OpenPanel(SideTab::NowPlaying));
    let (_, ticket) = urls_asked(&mut app)[0];
    answered(&mut app, ticket, Ok(signed(Some(360))));

    // GStreamer non apre l'indirizzo: l'app lo dice al core (qui il risultato del thread è un errore).
    let _ = app.update(Message::VideoOpened(super::id(1), 1, Err(AppError::Video)));
    assert_eq!(state(&app), ClipState::Loading, "una volta il core chiede un indirizzo nuovo");
    assert!(app.video.is_none());
    let (_, ticket) = urls_asked(&mut app)[0];
    answered(&mut app, ticket, Ok(signed(Some(360))));
    let _ = app.update(Message::VideoOpened(super::id(1), 2, Err(AppError::Video)));
    assert!(app.video_broken());
    assert!(matches!(&app.notice, Some(Notice::Error(text)) if text == "Videoclip non disponibile."));
    assert!(urls_asked(&mut app).is_empty());
}
