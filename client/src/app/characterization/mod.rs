//! Test di caratterizzazione: fissano il comportamento attuale dell'app, messaggio dopo messaggio,
//! prima che la logica si sposti nel core (passo 0 di `docs/transformation/04-core-architecture.md`).
//! Non dicono come l'app dovrebbe comportarsi ma come si comporta oggi: un comportamento che sembra
//! un difetto si fissa lo stesso, con un commento `NOTA:`, e si corregge in un passo a parte.
//!
//! L'app è quella di `App::for_tests`: niente `config.json` dell'utente, nessuna richiesta fuori da
//! una porta chiusa di 127.0.0.1 e, quando serve il suono, un'uscita staccata da cui il test legge i
//! campioni (`AudioOutput::detached`). I test dei salvataggi usano `App::for_tests_with_core`, con un
//! core vero e una cartella dei dati per test. I `Task` restituiti da `update` di norma si scartano e le
//! risposte si danno a mano; quando conta che cosa un `Task` produce, lo si esegue con [`drain`].
//! Dal passo 4.3 la riproduzione sta nel core sincrono dei test (`LocalCore`): i lavori di rete che
//! chiede (`take_jobs`) si risolvono a mano con [`loaded`], [`warmed`] e [`autoplay_loaded`], e il suo
//! battito scocca con [`tick`] (la posizione si legge dall'orologio, `App::position`).

mod flows;
mod playback;
mod system;
#[cfg(feature = "video")]
mod video;

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use iced::Task;
use iced_runtime::Action;
use rodio::Source;
use rodio::mixer::MixerSource;
use uuid::Uuid;

use psytify_core::command::SocialCmd;
use psytify_core::testing::{PendingJob, Section, SocialWork};

use super::{Account, App, Message};
use crate::api::social::MyProfile;
use crate::api::{ArtistPage, Me, Quality, Track};
use crate::auth::Session;
use crate::error::AppError;
use crate::player::AudioOutput;
use crate::player::stream::StreamBuffer;

/// Esegue un `Task` dell'app fino in fondo e restituisce i messaggi che produce, nell'ordine in cui
/// arrivano. Le azioni per la finestra (fuoco, appunti, dimensioni) si scartano: qui una finestra
/// non c'è. Con l'orologio di tokio fermo (`start_paused`) le attese passano subito; le richieste
/// vanno alla porta chiusa di `App::for_tests` e falliscono senza uscire dal computer.
async fn drain(task: Task<Message>) -> Vec<Message> {
    let Some(stream) = iced_runtime::task::into_stream(task) else { return Vec::new() };
    stream
        .filter_map(|action| async move {
            match action {
                Action::Output(message) => Some(message),
                _ => None,
            }
        })
        .collect()
        .await
}

/// L'identificatore del brano di prova `n`.
fn id(n: u128) -> Uuid {
    crate::api::models::fixtures::track(n).id
}

/// Il brano di prova `n`, lungo `seconds` secondi secondo i suoi metadati.
fn track(n: u128, seconds: u64) -> Track {
    let mut track = crate::api::models::fixtures::track(n);
    track.duration_ms = seconds * 1000;
    track
}

/// L'audio di un brano già scaricato per intero: `seconds` secondi di WAV mono a 8 kHz, generati
/// in memoria (nessun file di prova).
fn tone(seconds: u32) -> StreamBuffer {
    StreamBuffer::from_bytes(wav(seconds))
}

fn wav(seconds: u32) -> Vec<u8> {
    const RATE: u32 = 8_000;
    let frames = RATE * seconds;
    let data_len = frames * 2;
    let mut bytes = Vec::with_capacity(44 + data_len as usize);
    bytes.extend_from_slice(b"RIFF");
    bytes.extend_from_slice(&(36 + data_len).to_le_bytes());
    bytes.extend_from_slice(b"WAVEfmt ");
    bytes.extend_from_slice(&16u32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&RATE.to_le_bytes());
    bytes.extend_from_slice(&(RATE * 2).to_le_bytes());
    bytes.extend_from_slice(&2u16.to_le_bytes());
    bytes.extend_from_slice(&16u16.to_le_bytes());
    bytes.extend_from_slice(b"data");
    bytes.extend_from_slice(&data_len.to_le_bytes());
    for frame in 0..frames {
        // Un dente di sega piano: il contenuto non conta, basta che sia audio valido.
        let level = (i32::try_from(frame % 100).unwrap_or(0) - 50) * 100;
        bytes.extend_from_slice(&(level as i16).to_le_bytes());
    }
    bytes
}

/// Un'app di prova con un'uscita audio staccata: il tempo d'ascolto passa solo quando il test
/// legge i campioni dal mixer restituito ([`listen`]).
fn app_with_audio() -> (App, MixerSource) {
    let (audio, mixer) = AudioOutput::detached();
    (App::for_tests_with(Some(audio), |_| ()), mixer)
}

/// Il core sincrono dell'app di prova.
fn local(app: &mut App) -> &mut psytify_core::testing::LocalCore {
    app.local_core.as_mut().expect("for_tests ha il core sincrono")
}

/// Pubblica all'app ciò che il core sincrono ha prodotto, se qualcosa è cambiato, come farebbe
/// l'abbonamento al core: con un `Message::Core`, che passa da `update` come ogni messaggio.
fn publish(app: &mut App, snapshot: Option<Arc<psytify_core::Snapshot>>) -> Task<Message> {
    match snapshot {
        Some(snapshot) => app.update(Message::Core(snapshot)),
        None => Task::none(),
    }
}

/// Il server risponde al caricamento `generation`.
fn loaded(app: &mut App, generation: u64, result: Result<StreamBuffer, AppError>) -> Task<Message> {
    let snapshot = local(app).loaded(generation, result);
    publish(app, snapshot)
}

/// Un brano preparato in anticipo è arrivato.
fn warmed(app: &mut App, track: Uuid, quality: Quality, result: Result<StreamBuffer, AppError>) {
    let snapshot = local(app).warmed(track, quality, result);
    drop(publish(app, snapshot));
}

/// Il server risponde alla richiesta dei brani per la riproduzione automatica.
fn autoplay_loaded(app: &mut App, result: Result<ArtistPage, AppError>) -> Task<Message> {
    let snapshot = local(app).autoplay_loaded(result);
    publish(app, snapshot)
}

/// Il server risponde agli scaffali della Home: i recenti, l'elenco A-Z e gli album (vuoti).
fn home_answered(app: &mut App, recent: Vec<Track>, alphabetical: Vec<Track>) {
    fn page<T>(items: Vec<T>) -> crate::api::Page<T> {
        crate::api::Page { items, limit: 50, offset: 0, next_offset: None }
    }
    let snapshot = local(app).home_loaded(Section::Recent, Ok(page(recent)));
    drop(publish(app, snapshot));
    let snapshot = local(app).home_loaded(Section::Alphabetical, Ok(page(alphabetical)));
    drop(publish(app, snapshot));
    let snapshot = local(app).home_albums_loaded(Ok(page(Vec::new())));
    drop(publish(app, snapshot));
}

/// Il battito del core: legge la posizione dell'audio (solo se un brano suona).
fn tick(app: &mut App) -> Task<Message> {
    let snapshot = local(app).tick();
    publish(app, snapshot)
}

/// I caricamenti che il core ha chiesto dall'ultima volta, come (brano, generazione).
fn loads(app: &mut App) -> Vec<(Uuid, u64)> {
    local(app)
        .take_jobs()
        .into_iter()
        .filter_map(|job| match job {
            PendingJob::Load { track, generation } => Some((track, generation)),
            _ => None,
        })
        .collect()
}

/// Legge dal mixer i campioni di `duration`, come farebbe la scheda audio in quel tempo.
fn listen(mixer: &mut MixerSource, duration: Duration) {
    let per_second = f64::from(mixer.sample_rate().get()) * f64::from(mixer.channels().get());
    let samples = (duration.as_secs_f64() * per_second) as usize;
    mixer.by_ref().take(samples).for_each(drop);
}

/// Legge campioni finché l'uscita non dice che il brano è finito.
fn listen_to_the_end(app: &mut App, mixer: &mut MixerSource) {
    for _ in 0..600 {
        if local(app).audio().is_some_and(AudioOutput::finished) {
            return;
        }
        listen(mixer, Duration::from_millis(100));
    }
    panic!("il brano non finisce");
}

/// Fa partire `tracks` da `start` e consegna all'app l'audio che il server avrebbe mandato.
fn play(app: &mut App, tracks: Vec<Track>, start: usize, audio: StreamBuffer) {
    let _ = app.update(Message::PlayTracks(Arc::from(tracks), start));
    let generation = app.playback.generation;
    let _ = loaded(app, generation, Ok(audio));
    assert!(app.playback.is_playing(), "{:?}", app.playback.state);
    // Le richieste di rete di quando il brano parte (la testata dei prossimi) non servono ai test.
    let _ = local(app).take_jobs();
}

/// Due durate uguali a meno di 100 ms. La posizione di rodio avanza a passi di 5 ms, e il mixer,
/// per ricampionare, legge il brano una cinquantina di millisecondi in anticipo (in avanti a inizio
/// brano, indietro dopo un seek): lo scarto non dipende dal tempo, è sempre lo stesso.
fn near(actual: Duration, expected: Duration) -> bool {
    actual.abs_diff(expected) <= Duration::from_millis(100)
}

/// Collega l'app con un profilo che condivide l'attività, come dopo un accesso riuscito: il core dei
/// test adotta la sessione (`SocialCmd::Adopt`, solo per i test). La sessione ha un token fresco,
/// quindi nessuna richiesta parte verso Clerk. I lavori che l'ingresso chiede (gli amici) si
/// scartano: i test guardano ciò che parte dopo.
fn sign_in(app: &mut App) {
    let clerk = psytify_core::auth::ClerkApp {
        issuer: super::TEST_CLERK_ISSUER.into(),
        port: 53682,
        session_file: None,
    };
    let profile = MyProfile {
        handle: "prova".into(),
        display_name: "Prova".into(),
        avatar_url: None,
        share_activity: true,
    };
    let me = Me { user_id: "user_prova".into(), admin: false, profile: Some(profile) };
    let _ = app.send_social(SocialCmd::Adopt { session: Session::for_tests(clerk), me });
    assert!(matches!(app.social.view.account, Account::SignedIn(_)), "l'accesso è fatto");
    let _ = jobs(app);
}

/// I lavori di rete che il core ha chiesto dall'ultima volta.
fn jobs(app: &mut App) -> Vec<PendingJob> {
    app.local_core.as_mut().map(|core| core.take_jobs()).unwrap_or_default()
}

/// I lavori sociali fra i lavori chiesti dall'ultima volta.
fn social_jobs(app: &mut App) -> Vec<SocialWork> {
    jobs(app)
        .into_iter()
        .filter_map(|job| match job {
            PendingJob::Social(work) => Some(work),
            _ => None,
        })
        .collect()
}

#[test]
fn the_test_app_reads_no_user_config_and_opens_no_device() {
    let app = App::for_tests();
    assert!(app.local_core.as_ref().is_some_and(|core| core.audio().is_none()), "nessuna scheda audio");
    assert!(crate::config::dir().is_some_and(|dir| dir.starts_with(std::env::temp_dir())));
    assert_eq!(app.api_host(), "127.0.0.1:9");
    assert_eq!(app.queue.len(), 0, "la configurazione è quella di default");
    assert_eq!(app.social.view.account, Account::SignedOut, "il login c'è, ma su 127.0.0.1");
    assert!(app.core.is_none(), "nessun core: niente si salva");
}
