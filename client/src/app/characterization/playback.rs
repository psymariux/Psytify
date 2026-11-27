//! Riproduzione: le transizioni di stato (caricamento, play, pausa, errori) e ciò che succede a
//! fine brano al battito del core. Dal passo 4.3 la logica sta nel core (i suoi test, con il tempo
//! finto, sono in `core/src/state/playback.rs`): qui si prova l'app intera, messaggio dopo
//! messaggio, con il core sincrono dei test. Il suono esce da un'uscita staccata e il tempo
//! d'ascolto passa solo quando il test legge i campioni; i lavori di rete del core si risolvono a
//! mano (vedi `mod.rs`).

use std::sync::Arc;
use std::time::Duration;

use super::{
    app_with_audio, autoplay_loaded, id, jobs, listen, listen_to_the_end, loaded, loads, local, near, play,
    publish, sign_in, social_jobs, tick, tone, track, warmed,
};
use crate::api::ArtistPage;
use crate::app::{App, Message, Notice, SleepChoice};
use crate::error::AppError;
use crate::player::{PlaybackState, Repeat};
use psytify_core::command::{LibraryCmd, PlaybackCmd};
use psytify_core::testing::{PendingJob, SocialWork};

fn loading(n: u128) -> PlaybackState {
    PlaybackState::Loading { track_id: id(n) }
}

#[tokio::test(start_paused = true)]
async fn a_stale_audio_generation_is_ignored() {
    let (mut app, _mixer) = app_with_audio();
    let _ = app.update(Message::PlayTracks(Arc::from(vec![track(1, 4), track(2, 4)]), 0));
    let first = app.playback.generation;
    let _ = app.update(Message::Next);
    let second = app.playback.generation;
    assert_eq!(second, first + 1, "ogni caricamento ha la sua generazione");
    assert_eq!(app.playback.state, loading(2));

    // Le risposte del caricamento superato, buone o cattive, arrivano tardi e non cambiano nulla.
    let _ = loaded(&mut app, first, Ok(tone(4)));
    let _ = loaded(&mut app, first, Err(AppError::Network));
    assert_eq!(app.playback.state, loading(2));
    assert!(app.recent.is_empty(), "nessun ascolto registrato");

    let _ = loaded(&mut app, second, Ok(tone(4)));
    assert_eq!(app.playback.state, PlaybackState::Playing);
    assert_eq!(app.recent.first().map(|t| t.id), Some(id(2)));
}

#[tokio::test(start_paused = true)]
async fn play_from_stopped_loads_the_current_track() {
    // La coda ripresa dall'ultima sessione, ferma sul secondo brano.
    let mut app = App::for_tests_with(None, |config| {
        Arc::make_mut(&mut config.queue).replace(vec![track(1, 4), track(2, 4)], 1, 7);
    });
    assert_eq!(app.playback.state, PlaybackState::Stopped);
    let before = app.playback.generation;

    let _ = app.update(Message::TogglePlay);
    assert_eq!(app.playback.state, loading(2), "parte il brano corrente, non il primo della coda");
    assert_eq!(app.playback.generation, before + 1);
    assert_eq!(app.queue.position(), Some(1));
    assert_eq!(app.position(), Duration::ZERO);

    // Il brano si chiede al server: un solo caricamento, di questo brano.
    assert_eq!(loads(&mut app), [(id(2), before + 1)]);

    // Play durante il caricamento non fa nulla.
    let _ = app.update(Message::TogglePlay);
    assert_eq!((app.playback.state.clone(), app.playback.generation), (loading(2), before + 1));
    assert!(loads(&mut app).is_empty());

    // Dopo un errore, Play riprova lo stesso brano con un nuovo caricamento.
    let _ = loaded(&mut app, before + 1, Err(AppError::Network));
    assert_eq!(app.playback.state, PlaybackState::Error(AppError::Network.to_string()));
    let _ = app.update(Message::TogglePlay);
    assert_eq!((app.playback.state.clone(), app.playback.generation), (loading(2), before + 2));
    assert_eq!(loads(&mut app), [(id(2), before + 2)]);
}

#[tokio::test(start_paused = true)]
async fn pause_then_resume_keeps_the_position() {
    let (mut app, mut mixer) = app_with_audio();
    play(&mut app, vec![track(1, 4)], 0, tone(4));
    let generation = app.playback.generation;
    listen(&mut mixer, Duration::from_secs(1));
    let _ = tick(&mut app);
    let heard = app.position();
    assert!(near(heard, Duration::from_secs(1)), "{heard:?}");

    let _ = app.update(Message::TogglePlay);
    assert_eq!(app.playback.state, PlaybackState::Paused);
    // In pausa i campioni sono silenzio: la posizione non avanza, e il battito del core non gira.
    listen(&mut mixer, Duration::from_secs(1));
    assert!(!local(&mut app).ticking(), "in pausa il core non si sveglia");
    assert!(near(app.position(), heard), "{:?}", app.position());
    assert!(near(local(&mut app).audio().map_or(Duration::MAX, |audio| audio.position()), heard));

    let _ = app.update(Message::TogglePlay);
    assert_eq!(app.playback.state, PlaybackState::Playing);
    listen(&mut mixer, Duration::from_secs(1));
    let _ = tick(&mut app);
    assert!(near(app.position(), heard + Duration::from_secs(1)), "si riprende da dov'era");
    assert_eq!(app.playback.generation, generation, "pausa e ripresa non ricaricano il brano");
}

#[tokio::test(start_paused = true)]
async fn no_device_reports_the_device_error() {
    // `for_tests` non apre nessuna uscita audio: è il caso di un computer senza scheda.
    let mut app = App::for_tests();
    let _ = app.update(Message::PlayTracks(Arc::from(vec![track(1, 4)]), 0));
    let generation = app.playback.generation;
    let _ = loaded(&mut app, generation, Ok(tone(4)));
    assert_eq!(app.playback.state, PlaybackState::Error(AppError::AudioDevice.to_string()));
    assert!(app.recent.is_empty(), "un brano che non suona non è un ascolto");

    // Senza uscita il battito non gira: né tempo ascoltato né fine del brano.
    assert!(!local(&mut app).ticking());
    let _ = tick(&mut app);
    assert_eq!(app.listened_ms, 0);
    assert_eq!(app.playback.state, PlaybackState::Error(AppError::AudioDevice.to_string()));
}

#[tokio::test(start_paused = true)]
async fn a_missing_track_is_healed() {
    let not_found =
        || AppError::Api { status: 404, code: "not_found".into(), message: "Brano non trovato".into() };
    let (mut app, _mixer) = app_with_audio();
    let missing = track(1, 4);
    app.send_library_quiet(LibraryCmd::ToggleLike(missing.clone()));
    let _ = app.update(Message::PlayTracks(Arc::from(vec![missing.clone(), track(2, 4)]), 0));
    let generation = app.playback.generation;

    // Il server non ha più il brano: il core lo cerca da sé nel catalogo ufficiale (un'attività in
    // corso: l'app non fa niente) e intanto si resta in caricamento, senza errore.
    let _ = loaded(&mut app, generation, Err(not_found()));
    assert_eq!(app.playback.state, loading(1));
    let asked = local(&mut app).take_jobs();
    assert!(
        asked.iter().any(|job| matches!(job, PendingJob::Heal { track } if *track == missing.id)),
        "parte la ricerca nel catalogo: {asked:?}"
    );

    // Ritrovato con un altro identificatore: prende il suo posto in coda e fra i salvati, e parte.
    let found = track(9, 4);
    let snapshot = local(&mut app).healed(missing.id, Ok(found.clone()));
    let _ = publish(&mut app, snapshot);
    assert_eq!(app.queue.current().map(|t| t.id), Some(found.id));
    assert_eq!(app.liked.first().map(|t| t.id), Some(found.id));
    assert_eq!(app.playback.state, loading(9));
    assert_eq!(app.playback.generation, generation + 1);

    // Un brano che non si ritrova resta in errore, e lo si cerca una volta sola per sessione.
    let (mut app, _mixer) = app_with_audio();
    let _ = app.update(Message::PlayTracks(Arc::from(vec![missing.clone()]), 0));
    let generation = app.playback.generation;
    let _ = loaded(&mut app, generation, Err(not_found()));
    let snapshot = local(&mut app).healed(missing.id, Err(AppError::InvalidResponse));
    let _ = publish(&mut app, snapshot);
    assert_eq!(
        app.playback.state,
        PlaybackState::Error(
            "Questo brano non è più disponibile sul server e non l'ho ritrovato nel catalogo.".into()
        )
    );
    let _ = app.update(Message::TogglePlay);
    let generation = app.playback.generation;
    let _ = local(&mut app).take_jobs();
    let _ = loaded(&mut app, generation, Err(not_found()));
    assert_eq!(app.playback.state, PlaybackState::Error(not_found().to_string()));
    assert!(
        !local(&mut app).take_jobs().iter().any(|job| matches!(job, PendingJob::Heal { .. })),
        "la seconda volta non si cerca di nuovo"
    );
}

#[tokio::test(start_paused = true)]
async fn previous_after_three_seconds_restarts() {
    let (mut app, mut mixer) = app_with_audio();
    play(&mut app, vec![track(1, 8), track(2, 8)], 1, tone(8));
    listen(&mut mixer, Duration::from_millis(3_500));
    let _ = tick(&mut app);
    let generation = app.playback.generation;

    // Oltre tre secondi «precedente» riparte da capo, sullo stesso brano e senza ricaricarlo.
    let _ = app.update(Message::Previous);
    assert_eq!(app.queue.position(), Some(1));
    assert!(near(app.position(), Duration::ZERO), "{:?}", app.position());
    assert_eq!(app.playback.state, PlaybackState::Playing);
    assert_eq!(app.playback.generation, generation);

    // Entro tre secondi passa al brano prima.
    for _ in 0..100 {
        listen(&mut mixer, Duration::from_millis(10));
    }
    let _ = tick(&mut app);
    let _ = app.update(Message::Previous);
    assert_eq!(app.queue.position(), Some(0));
    assert_eq!(app.playback.state, loading(1));
    assert_eq!(app.playback.generation, generation + 1);
}

#[tokio::test(start_paused = true)]
async fn a_cached_or_warm_buffer_skips_the_network() {
    let (mut app, _mixer) = app_with_audio();
    let quality = app.quality;
    play(&mut app, vec![track(1, 4), track(2, 4)], 0, tone(4));

    // Il brano appena suonato si riascolta dal suo buffer: nessuna richiesta, e parte subito.
    let _ = app.update(Message::PlayQueueAt(0));
    assert!(loads(&mut app).is_empty());
    assert_eq!(app.playback.state, PlaybackState::Playing);

    // Un brano preparato in anticipo parte dalla sua testata calda, senza rete.
    warmed(&mut app, id(2), quality, Ok(tone(4)));
    let _ = app.update(Message::Next);
    assert!(loads(&mut app).is_empty());
    assert_eq!(app.playback.state, PlaybackState::Playing);
    assert_eq!(app.queue.position(), Some(1));
}

#[tokio::test(start_paused = true)]
async fn a_finished_track_advances() {
    let (mut app, mut mixer) = app_with_audio();
    play(&mut app, vec![track(1, 1), track(2, 1)], 0, tone(1));
    listen_to_the_end(&mut app, &mut mixer);
    let (generation, listened) = (app.playback.generation, app.listened_ms);

    let _ = tick(&mut app);
    assert_eq!(app.queue.position(), Some(1));
    assert_eq!(app.playback.state, loading(2));
    assert_eq!(app.playback.generation, generation + 1);
    assert_eq!(app.listened_ms, listened, "il battito che trova il brano finito non conta tempo");
    assert!(!local(&mut app).ticking(), "in caricamento il core non si sveglia");
}

#[tokio::test(start_paused = true)]
async fn repeat_one_replays() {
    let (mut app, mut mixer) = app_with_audio();
    play(&mut app, vec![track(1, 1), track(2, 1)], 0, tone(1));
    let _ = app.update(Message::CycleRepeat);
    let _ = app.update(Message::CycleRepeat);
    assert_eq!(app.queue.repeat(), Repeat::One);
    listen_to_the_end(&mut app, &mut mixer);
    let generation = app.playback.generation;

    let _ = tick(&mut app);
    assert_eq!(app.queue.position(), Some(0), "resta sullo stesso brano");
    // Lo stesso brano riparte dal buffer in memoria, senza rete: è già in riproduzione.
    assert!(loads(&mut app).is_empty());
    assert_eq!(app.playback.state, PlaybackState::Playing);
    assert_eq!(app.playback.generation, generation + 1);
    listen(&mut mixer, Duration::from_millis(500));
    let _ = tick(&mut app);
    assert!(near(app.position(), Duration::from_millis(500)), "ricomincia da capo");
}

#[tokio::test(start_paused = true)]
async fn end_of_queue_asks_autoplay_once() {
    let (mut app, mut mixer) = app_with_audio();
    assert!(app.autoplay, "la riproduzione automatica è accesa di default");
    play(&mut app, vec![track(1, 1)], 0, tone(1));
    listen_to_the_end(&mut app, &mut mixer);

    // A fine coda si chiedono altri brani dello stesso artista.
    let _ = tick(&mut app);
    let asked = local(&mut app).take_jobs();
    assert_eq!(asked, [psytify_core::testing::PendingJob::Autoplay { artist: track(1, 1).artist.id }]);
    // BACKLOG 4: mentre si aspettano i brani l'audio è finito, e lo stato lo dice (non più
    // `Playing`: il tasto non mostra la pausa su un brano che non suona).
    assert_eq!(app.playback.state, loading(1));
    // Il battito non gira più, quindi nessuna seconda richiesta.
    assert!(!local(&mut app).ticking());
    assert!(local(&mut app).take_jobs().is_empty());

    // La richiesta fallita, senza un catalogo recente da cui pescare, ferma la musica.
    let _ = autoplay_loaded(&mut app, Err(AppError::Network));
    assert_eq!(app.playback.state, PlaybackState::Stopped);

    // Con dei brani nuovi, invece, si accodano (tolti quelli già in coda) e parte il primo.
    let (mut app, mut mixer) = app_with_audio();
    play(&mut app, vec![track(1, 1)], 0, tone(1));
    listen_to_the_end(&mut app, &mut mixer);
    let _ = tick(&mut app);
    let page = ArtistPage {
        id: track(1, 1).artist.id,
        name: track(1, 1).artist.name,
        image_url: None,
        albums: Vec::new(),
        tracks: vec![track(1, 1), track(4, 1), track(7, 1)],
    };
    let _ = autoplay_loaded(&mut app, Ok(page));
    assert_eq!(app.queue.len(), 3);
    assert_eq!(app.queue.position(), Some(1));
    let next = app.queue.current().map(|t| t.id);
    assert!(next == Some(id(4)) || next == Some(id(7)), "{next:?}");
    assert!(matches!(app.playback.state, PlaybackState::Loading { track_id } if Some(track_id) == next));
    assert_eq!(app.notice, Some(Notice::Info("Riproduzione automatica: ancora musica come questa".into())));
}

#[tokio::test(start_paused = true)]
async fn sleep_at_end_of_track_holds_the_next() {
    let (mut app, mut mixer) = app_with_audio();
    play(&mut app, vec![track(1, 1), track(2, 1)], 0, tone(1));
    let _ = app.update(Message::SetSleep(Some(SleepChoice::EndOfTrack)));
    assert!(app.sleep.is_some());
    listen_to_the_end(&mut app, &mut mixer);
    let generation = app.playback.generation;

    let _ = tick(&mut app);
    assert_eq!(app.playback.state, PlaybackState::Stopped);
    assert_eq!(app.queue.position(), Some(1), "il prossimo brano è pronto in coda, ma non parte");
    assert_eq!(app.playback.generation, generation);
    assert!(app.sleep.is_none(), "il timer si consuma");
    assert_eq!(app.notice, Some(Notice::Info("Buonanotte: la musica si è fermata".into())));

    // Play riparte dal brano dopo.
    let _ = app.update(Message::TogglePlay);
    assert_eq!(app.playback.state, loading(2));
}

#[tokio::test(start_paused = true)]
async fn a_play_counts_after_thirty_seconds_or_half() {
    // Un brano di quattro secondi conta a metà.
    let (mut app, mut mixer) = app_with_audio();
    let short = track(1, 4);
    play(&mut app, vec![short.clone()], 0, tone(4));
    listen(&mut mixer, Duration::from_millis(1_500));
    let _ = tick(&mut app);
    assert!(app.stats.is_empty(), "un secondo e mezzo non basta");
    listen(&mut mixer, Duration::from_secs(1));
    let _ = tick(&mut app);
    assert_eq!(app.stats.iter().map(|s| (s.track.id, s.plays)).collect::<Vec<_>>(), [(short.id, 1)]);
    // BACKLOG 5, corretto in 4.7: senza accesso l'ascolto per il profilo si butta, non resta in sospeso.
    // Se poi si accede, non parte niente per un brano sentito prima.
    assert!(social_jobs(&mut app).is_empty(), "senza accesso niente va al profilo");
    sign_in(&mut app);
    listen(&mut mixer, Duration::from_millis(500));
    let _ = tick(&mut app);
    assert_eq!(app.stats[0].plays, 1, "un brano conta una volta per ascolto");
    assert!(
        !social_jobs(&mut app).iter().any(|work| matches!(work, SocialWork::Listen(_))),
        "il brano sentito prima dell'accesso non parte dopo"
    );

    // Con l'accesso fatto, un brano che conta va anche al profilo, una volta sola.
    let (mut app, mut mixer) = app_with_audio();
    sign_in(&mut app);
    let heard = track(3, 4);
    play(&mut app, vec![heard.clone()], 0, tone(4));
    let _ = jobs(&mut app);
    listen(&mut mixer, Duration::from_millis(2_500));
    let _ = tick(&mut app);
    let sent: Vec<SocialWork> =
        social_jobs(&mut app).into_iter().filter(|work| matches!(work, SocialWork::Listen(_))).collect();
    assert_eq!(sent, [SocialWork::Listen(heard.id)]);
    listen(&mut mixer, Duration::from_millis(500));
    let _ = tick(&mut app);
    assert!(social_jobs(&mut app).iter().all(|work| !matches!(work, SocialWork::Listen(_))));

    // Uno lungo conta dopo trenta secondi: qui ci si arriva con un salto in avanti.
    let (mut app, mut mixer) = app_with_audio();
    play(&mut app, vec![track(2, 180)], 0, tone(40));
    listen(&mut mixer, Duration::from_secs(1));
    let _ = tick(&mut app);
    assert!(app.stats.is_empty());
    let _ = app.update(Message::SeekBy(30));
    listen(&mut mixer, Duration::from_millis(100));
    let _ = tick(&mut app);
    assert_eq!(app.stats.iter().map(|s| (s.track.id, s.plays)).collect::<Vec<_>>(), [(id(2), 1)]);
}

/// BACKLOG 6 e 15: il tempo ascoltato segue la posizione dell'audio, non i battiti, e arriva alla
/// libreria quando la riproduzione si ferma.
#[tokio::test(start_paused = true)]
async fn listened_time_follows_the_audio_and_not_the_ticks() {
    let (mut app, mut mixer) = app_with_audio();
    let _ = app.update(Message::PlayTracks(Arc::from(vec![track(1, 4)]), 0));
    assert!(!local(&mut app).ticking(), "in caricamento non si ascolta niente");
    let generation = app.playback.generation;
    let _ = loaded(&mut app, generation, Ok(tone(4)));

    // Nessun campione letto: i battiti passano, la posizione no, e non si ascolta niente.
    for _ in 0..3 {
        let _ = tick(&mut app);
    }
    let _ = app.update(Message::TogglePlay);
    assert_eq!(app.listened_ms, 0, "tre battiti senza suono non sono ascolto");

    // Letto un secondo di audio, in quattro battiti: un secondo ascoltato (la pausa lo consegna).
    let _ = app.update(Message::TogglePlay);
    for _ in 0..4 {
        listen(&mut mixer, Duration::from_millis(250));
        let _ = tick(&mut app);
    }
    let _ = app.update(Message::TogglePlay);
    let heard = Duration::from_millis(app.listened_ms);
    assert!(near(heard, Duration::from_secs(1)), "{heard:?}");

    // Ripresa e di nuovo in pausa senza ascoltare: il totale non cambia.
    let _ = app.update(Message::TogglePlay);
    let _ = tick(&mut app);
    let _ = app.update(Message::TogglePlay);
    assert_eq!(Duration::from_millis(app.listened_ms), heard);
}

/// Il core si porta dietro il comando anche da fuori dei messaggi dell'interfaccia.
#[tokio::test(start_paused = true)]
async fn the_queue_reorder_of_the_phone_goes_through_the_core() {
    let mut app = App::for_tests();
    for n in 1..=3 {
        let _ = app.update(Message::Enqueue(track(n, 4)));
    }
    let order = |app: &App| app.queue.entries().map(|(_, t)| t.id).collect::<Vec<_>>();
    assert_eq!(order(&app), [id(1), id(2), id(3)]);
    let _ = app.send_playback(PlaybackCmd::Move { from: 0, to: 2 });
    assert_eq!(order(&app), [id(2), id(3), id(1)]);
    assert_eq!(app.queue.current().map(|t| t.id), Some(id(1)), "il brano corrente si sposta con la coda");
}
