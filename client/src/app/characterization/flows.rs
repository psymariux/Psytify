//! Flussi fuori dalla riproduzione: salvataggi, mix della Home, ricerca, importazioni, download,
//! presenza e playlist pubbliche, jam. Come in `playback.rs`, si fissa il comportamento di oggi.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use iced::window;
use psytify_core::Snapshot as CoreSnapshot;
use psytify_core::command::LibraryCmd;
use psytify_core::snapshot::{ImportTarget, Importing};
use uuid::Uuid;

use super::{
    app_with_audio, drain, home_answered, id, jobs, listen, loaded, local, near, publish, sign_in,
    social_jobs, tick, tone, track,
};
use crate::api::{
    DiscoverImport, DownloadQuality, Downloaded, MetaArtist, MetaArtistPage, Page, SearchResults, SourceItem,
    SourceKind, SourcePlaylist, SourcePlaylistPage, Track,
};
use crate::app::{Account, App, AuthField, AuthMode, Feed, Load, Message, Notice, Screen};
use crate::config::{self, FollowedArtist, Playlist, taste};
use crate::error::AppError;
use crate::jam::{Event, Member, Remote, Snapshot, Target};
use crate::player::PlaybackState;
use psytify_core::testing::{JamOp, PendingJob, SocialWork};

// ---------- Salvataggi ----------
//
// `config.json` lo scrive solo il core (passo 4.1): l'app gli manda lo stato e alla chiusura ne
// aspetta il salvataggio. Come e quando scrive lo fissano i test del riduttore
// (`core/src/state/settings.rs`); qui c'è l'app, con un core vero e una cartella dei dati sua.

/// Un'app di prova con un core vero e una cartella dei dati che nessun altro test tocca: niente
/// lucchetti fra i test (`docs/transformation/BACKLOG.md`, 11).
fn app_with_core() -> (App, PathBuf) {
    let dir = std::env::temp_dir().join(format!("psytify-test-{}", Uuid::new_v4()));
    (App::for_tests_with_core(Some(dir.clone())), dir)
}

/// I campi del core nel `config.json` di `dir`.
fn on_disk(dir: &Path) -> config::CoreConfig {
    config::load(Some(&config::file_in(dir))).core
}

/// Il `config.json` di `dir` quando `ready` dice che c'è ciò che serve. Lo scrive il core quando
/// vuole lui, sul runtime del test: si guarda il disco finché non arriva (al più 10 s veri).
async fn on_disk_once(dir: &Path, ready: impl Fn(&config::CoreConfig) -> bool) -> config::CoreConfig {
    let give_up = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let config = on_disk(dir);
        if ready(&config) || std::time::Instant::now() > give_up {
            return config;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// L'istantanea del core in cui `ready` è vero, aspettando le pubblicazioni che servono.
async fn published(app: &App, ready: impl Fn(&CoreSnapshot) -> bool) -> Arc<CoreSnapshot> {
    let mut changes = app.core.as_ref().expect("un'app con il core").changes();
    loop {
        let snapshot = Arc::clone(&changes.borrow_and_update());
        if ready(&snapshot) {
            return snapshot;
        }
        changes.changed().await.expect("il core è acceso");
    }
}

/// La schermata che il core salverà per l'interfaccia Iced.
fn saved_screen(snapshot: &CoreSnapshot) -> Option<&str> {
    snapshot.settings.ui.get("last_screen")?.get("kind")?.as_str()
}

/// Più del ritardo con cui il core raccoglie le modifiche prima di scriverle (mezzo secondo).
const PAST_THE_SAVE_DELAY: Duration = Duration::from_secs(1);

#[tokio::test(start_paused = true)]
async fn saves_go_through_the_core_and_the_last_state_wins() {
    let (mut app, dir) = app_with_core();
    let quality = app.downloads.quality;
    assert_ne!(quality, DownloadQuality::Low);

    // Due cambi di fila: l'app li manda al core e non aspetta nessun salvataggio.
    assert!(drain(app.update(Message::SetAutoplay(false))).await.is_empty());
    assert!(drain(app.update(Message::SetDownloadQuality(DownloadQuality::Low))).await.is_empty());

    // Il core li scrive da solo, poco dopo, con l'ultimo stato.
    let on_disk = on_disk_once(&dir, |config| config.download_quality == DownloadQuality::Low).await;
    assert_eq!(on_disk.download_quality, DownloadQuality::Low);
    assert!(!on_disk.autoplay);
    app.core.take().expect("il core").shutdown().await.unwrap().unwrap();
}

/// `docs/transformation/BACKLOG.md`, 12: andando in background (il telefono, che Android può
/// chiudere in ogni momento) il core scrive subito, senza aspettare il mezzo secondo. Il tempo del
/// test è finto: se la scrittura aspettasse il timer, il disco la vedrebbe solo dopo `SAVE_DELAY`.
#[tokio::test(start_paused = true)]
async fn going_to_the_background_saves_at_once() {
    let (mut app, dir) = app_with_core();
    let start = tokio::time::Instant::now();
    let _ = app.update(Message::SetAutoplay(false));
    let _ = app.update(Message::Backgrounded);

    let on_disk = on_disk_once(&dir, |config| !config.autoplay).await;
    assert!(!on_disk.autoplay);
    assert!(start.elapsed() < Duration::from_millis(500), "scritto dopo {:?}", start.elapsed());
    app.core.take().expect("il core").shutdown().await.unwrap().unwrap();
}

/// `docs/transformation/BACKLOG.md`, 7: la chiusura passa dal core, che aspetta la scrittura in
/// corso e scrive l'ultimo stato; solo dopo l'app esce. Mai due scritture insieme, quindi mai due
/// mani su `config.json.tmp`.
#[tokio::test(start_paused = true)]
async fn close_flushes_before_exit() {
    let (mut app, dir) = app_with_core();
    let window = window::Id::unique();
    // Un cambio che il core sta già salvando, o salverà fra poco.
    let _ = app.update(Message::SetAutoplay(false));
    tokio::time::sleep(PAST_THE_SAVE_DELAY).await;
    app.send_library_quiet(LibraryCmd::ToggleLike(track(1, 180)));
    let _ = app.update(Message::PlayTracks(Arc::from(vec![track(2, 180), track(3, 180)]), 1));

    let messages = drain(app.update(Message::CloseRequested(window))).await;
    assert!(matches!(messages.as_slice(), [Message::Exit(closed)] if *closed == window), "{messages:?}");
    assert!(app.core.is_none(), "si esce a core fermo");
    let on_disk = on_disk(&dir);
    assert_eq!(on_disk.liked.iter().map(|t| t.id).collect::<Vec<_>>(), [id(1)]);
    assert_eq!(on_disk.queue.current().map(|t| t.id), Some(id(3)));
    assert_eq!(on_disk.queue.position(), Some(1));
    assert!(!on_disk.autoplay, "dentro c'è anche il cambio già in salvataggio");
    assert!(!config::file_in(&dir).with_extension("json.tmp").exists());
}

#[tokio::test(start_paused = true)]
async fn a_failed_save_shows_a_notice() {
    // Al posto della cartella dei dati c'è un file: il core non riesce a scrivere.
    let blocked = std::env::temp_dir().join(format!("psytify-test-{}", Uuid::new_v4()));
    std::fs::write(&blocked, "").unwrap();
    let mut app = App::for_tests_with_core(Some(blocked.join("psytify")));
    let _ = app.update(Message::SetAutoplay(false));

    let failed = published(&app, |snapshot| snapshot.settings.save_error.is_some()).await;
    let _ = app.update(Message::Core(failed));
    assert!(
        matches!(&app.notice, Some(Notice::Error(text)) if text.starts_with("Impostazioni non salvate.")),
        "{:?}",
        app.notice
    );
}

/// Prestazioni: ferma, l'app non riceve istantanee (nemmeno quando il core finisce di salvare), e
/// navigare ne produce una sola, solo se cambia la schermata da salvare.
#[tokio::test(start_paused = true)]
async fn idle_and_navigation_publish_only_real_changes() {
    let (mut app, _dir) = app_with_core();
    // La prima navigazione porta al core anche lo stato dell'app (qui, il login di prova). Una
    // schermata senza dati dal catalogo: la Libreria o la Home farebbero partire richieste, e la loro
    // risposta (qui un errore di rete) è un'altra pubblicazione, legittima.
    let _ = app.update(Message::Navigate(Screen::Collection));
    let state = app.stored_config();
    let settled =
        published(&app, |s| saved_screen(s) == Some("collection") && *s.settings.config == state).await;

    let mut changes = app.core.as_ref().expect("il core").changes();
    changes.borrow_and_update();
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(!changes.has_changed().unwrap(), "5 s idle, the save included: no snapshot");

    let _ = app.update(Message::Navigate(Screen::Liked));
    let liked = published(&app, |s| saved_screen(s) == Some("liked")).await;
    assert_eq!(liked.version, settled.version + 1, "one change, one snapshot");

    for _ in 0..3 {
        let _ = app.update(Message::Navigate(Screen::Liked));
    }
    let core = app.core.take().expect("il core");
    let last = core.changes();
    core.shutdown().await.unwrap().unwrap();
    assert_eq!(last.borrow().version, liked.version, "the same screen again publishes nothing");
}

// ---------- Gusti ----------

/// Consegna all'app le pubblicazioni del core, come farebbe l'abbonamento di Iced, finché `ready`
/// è vero. Restituisce i messaggi dei `Task` che le reazioni hanno prodotto.
async fn deliver_until(app: &mut App, ready: impl Fn(&App) -> bool) -> Vec<Message> {
    let mut produced = Vec::new();
    while !ready(app) {
        let seen = app.core_snapshot.version;
        let next = published(app, |snapshot| snapshot.version > seen).await;
        produced.extend(drain(app.update(Message::Core(next))).await);
    }
    produced
}

/// Passo 4.2: la playlist nata vuota si apre quando il core la pubblica, non prima (la pagina non
/// la troverebbe: «Questa playlist non esiste più»).
#[tokio::test(start_paused = true)]
async fn a_new_playlist_opens_when_the_core_publishes_it() {
    let (mut app, _dir) = app_with_core();
    let _ = app.update(Message::OpenDialog(crate::app::Dialog::NewPlaylist {
        name: "Sera".into(),
        seed: Vec::new(),
    }));
    let _ = app.update(Message::DialogSubmit);
    assert!(app.playlists.is_empty(), "the core has not published yet");
    assert_eq!(app.screen, Screen::Home);

    deliver_until(&mut app, |app| matches!(app.screen, Screen::Playlist(_))).await;
    let id = app.playlists.first().map(|p| p.id).expect("la playlist è nella vista");
    assert_eq!(app.screen, Screen::Playlist(id));
    assert_eq!(app.playlist(id).map(|p| p.name.as_str()), Some("Sera"));
    app.core.take().expect("il core").shutdown().await.unwrap().unwrap();
}

/// Passo 4.4: i brani recenti della Home e il feed che il core ne ricava arrivano nella stessa
/// istantanea: la Home non passa mai per «Il catalogo è ancora vuoto».
#[test]
fn the_home_shows_new_tracks_with_their_feed() {
    let mut app = App::for_tests();
    assert!(matches!(app.home.recent, Load::Idle));
    let recent: Vec<Track> = (1..=6).map(|n| track(n, 180)).collect();
    let snapshot = local(&mut app).home_loaded(psytify_core::testing::Section::Recent, Ok(page(recent)));
    assert!(snapshot.is_some(), "an answer is a change");
    let before = app.home.feed.releases.len();
    let _ = publish(&mut app, snapshot);
    assert!(matches!(app.home.recent, Load::Ready(_)));
    assert!(app.home.feed.releases.len() > before, "the tracks and their feed arrive together");
}

/// Prestazioni (passo 4.2): ferma, la libreria non pubblica nulla; un cambio vero è una sola
/// istantanea; lo stesso cambio ripetuto, o un comando senza effetto, nessuna.
#[tokio::test(start_paused = true)]
async fn library_idle_and_repeats_publish_only_real_changes() {
    let (mut app, _dir) = app_with_core();
    let _ = app.update(Message::ToggleLike(track(1, 180)));
    let settled = published(&app, |s| s.library.liked.len() == 1).await;
    let _ = app.update(Message::Core(Arc::clone(&settled)));
    // Lasciar passare il salvataggio del file: l'ultima pubblicazione del giro.
    let mut changes = app.core.as_ref().expect("il core").changes();
    changes.borrow_and_update();
    tokio::time::sleep(Duration::from_secs(5)).await;
    let last = Arc::clone(&changes.borrow_and_update());
    assert!(!changes.has_changed().unwrap(), "idle: no snapshot");

    // Comandi che non cambiano nulla: né pubblicazioni né scritture.
    let core = app.core.as_ref().expect("il core");
    for command in [
        LibraryCmd::ForgetSearch(None),
        LibraryCmd::MarkNewsSeen,
        LibraryCmd::RefreshFeed,
        LibraryCmd::Listen(0),
        LibraryCmd::DeletePlaylist(Uuid::from_u128(1)),
    ] {
        core.send(psytify_core::Command::library(command));
    }
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(!changes.has_changed().unwrap(), "commands without effect publish nothing");

    // Un cambio vero: una pubblicazione sola, che porta libreria e file insieme.
    let _ = app.update(Message::ToggleLike(track(2, 180)));
    let next = published(&app, |s| s.library.liked.len() == 2).await;
    assert_eq!(next.version, last.version + 1);
    let mut watch = app.core.as_ref().expect("il core").changes();
    watch.borrow_and_update();
    tokio::time::sleep(Duration::from_secs(5)).await;
    // Dopo il salvataggio il core non pubblica più: l'errore di salvataggio o la sua fine non
    // sono cambiamenti della libreria.
    assert!(Arc::ptr_eq(&watch.borrow().library, &next.library));
    app.core.take().expect("il core").shutdown().await.unwrap().unwrap();
}

/// Una pagina sola del catalogo, con questi elementi.
fn page<T>(items: Vec<T>) -> Page<T> {
    Page { items, limit: 50, offset: 0, next_offset: None }
}

#[tokio::test(start_paused = true)]
async fn mixes_stay_fixed_for_the_session() {
    let mut app = App::for_tests();
    let recent: Vec<Track> = (1..=12).map(|n| track(n, 180)).collect();
    let now = taste::now_secs();
    for (at, plays) in [(0, 3), (1, 2), (2, 1)] {
        for _ in 0..plays {
            app.send_library_quiet(LibraryCmd::CountPlay { track: recent[at].clone(), at: now });
        }
    }
    home_answered(&mut app, recent.clone(), recent.clone());
    // Il seme dei mix è quello del core dei test.
    let seed = 1;
    let mixes = app.home.feed.mixes.clone();
    assert_eq!(mixes.len(), 3, "un mix per ciascuno dei tre artisti ascoltati");
    assert_eq!(
        mixes,
        Feed::build(&recent, &[], &app.stats, seed).mixes,
        "i mix nascono dal seme della sessione"
    );

    // Si esce dalla Home e si rientra: il feed si rifà (l'artista seguito nel frattempo compare), i
    // mix restano gli stessi, brano per brano.
    let followed = FollowedArtist { id: Uuid::from_u128(5000), name: "Seguito".into(), cover_url: None };
    app.send_library_quiet(LibraryCmd::ToggleFollow(followed.clone()));
    let _ = app.update(Message::Navigate(Screen::Liked));
    let _ = app.update(Message::Navigate(Screen::Home));
    assert_eq!(app.home.feed.artists.first().map(|card| card.artist.id), Some(followed.id));
    assert_eq!(app.home.feed.mixes, mixes);
    // È il seme a tenerli fermi: con un altro, gli stessi ascolti danno un altro ordine.
    assert_ne!(
        Feed::build(&recent, &[], &app.stats, 1).mixes,
        Feed::build(&recent, &[], &app.stats, 2).mixes
    );
}

// ---------- Ricerca ----------

#[test]
fn typing_debounces_and_late_results_are_dropped() {
    let mut app = App::for_tests();
    let _ = app.update(Message::SearchInput("lo".into()));
    assert_eq!(app.screen, Screen::Search, "scrivere porta alla ricerca");
    assert!(local(&mut app).search_pending(), "il core aspetta 300 ms dopo l'ultimo tasto");
    let _ = app.update(Message::SearchInput("low".into()));
    assert_eq!(app.search.results, Load::Idle, "mentre si scrive non parte nessuna ricerca");
    assert!(local(&mut app).take_jobs().is_empty());

    // L'attesa dell'ultimo tasto fa partire la ricerca, e solo quella.
    let snapshot = local(&mut app).fire_search();
    let _ = publish(&mut app, snapshot);
    assert_eq!(app.search.results, Load::Loading);
    assert_eq!(app.search.submitted, "low");
    assert_eq!(*app.search_history, ["low"]);
    let ticket = local(&mut app)
        .take_jobs()
        .into_iter()
        .find_map(|job| match job {
            psytify_core::testing::PendingJob::Search { ticket, .. } => Some(ticket),
            _ => None,
        })
        .expect("la ricerca parte");

    // La risposta di una ricerca superata si scarta; quella giusta si mostra.
    let found = |n| SearchResults { tracks: vec![track(n, 180)], ..SearchResults::default() };
    let snapshot = local(&mut app).searched(ticket - 1, Ok(found(1)));
    let _ = publish(&mut app, snapshot);
    assert_eq!(app.search.results, Load::Loading);
    let snapshot = local(&mut app).searched(ticket, Ok(found(2)));
    let _ = publish(&mut app, snapshot);
    assert_eq!(app.search.results, Load::Ready(Arc::new(found(2))));

    // Si scrive ancora: i risultati restano, e una risposta in ritardo non li sovrascrive.
    let _ = app.update(Message::SearchInput("lowe".into()));
    let snapshot = local(&mut app).searched(ticket, Ok(found(3)));
    let _ = publish(&mut app, snapshot);
    assert_eq!(app.search.results, Load::Ready(Arc::new(found(2))));
}

// ---------- Catalogo ----------

fn artist_page(id: Uuid, name: &str) -> crate::api::ArtistPage {
    crate::api::ArtistPage { id, name: name.into(), image_url: None, albums: Vec::new(), tracks: Vec::new() }
}

/// Passo 4.4: la pagina artista aperta mostra il caricamento, poi la pagina; un aggiornamento in
/// silenzio la sostituisce senza passare dal caricamento, e una pagina in anticipo di un altro
/// artista non la tocca.
#[test]
fn the_open_artist_follows_the_core_including_silent_refreshes() {
    let mut app = App::for_tests();
    let artist = Uuid::from_u128(77);
    let _ = app.update(Message::Navigate(Screen::Artist(artist)));
    assert!(matches!(&app.artist, Some((open, Load::Loading)) if *open == artist));
    assert!(app.shows_skeleton());

    let snapshot = local(&mut app).catalog_artist(artist, false, Ok(artist_page(artist, "Prima")));
    let _ = publish(&mut app, snapshot);
    assert!(matches!(&app.artist, Some((_, Load::Ready(page))) if page.name == "Prima"));

    let snapshot = local(&mut app).catalog_artist(artist, true, Ok(artist_page(artist, "Dopo")));
    let _ = publish(&mut app, snapshot);
    assert!(
        matches!(&app.artist, Some((_, Load::Ready(page))) if page.name == "Dopo"),
        "the refreshed page replaces the shown one"
    );

    let other = Uuid::from_u128(78);
    let snapshot = local(&mut app).catalog_artist(other, true, Ok(artist_page(other, "Altro")));
    let _ = publish(&mut app, snapshot);
    assert!(matches!(&app.artist, Some((open, Load::Ready(page))) if *open == artist && page.name == "Dopo"));
}

/// Passo 4.4: un artista del catalogo ufficiale si apre quando il core lo ha portato nel server, e un
/// errore è un avviso.
#[test]
fn a_catalogue_artist_opens_when_the_core_answers_and_a_failure_is_a_notice() {
    let mut app = App::for_tests();
    let _ = app.update(Message::OpenMetaArtist(5));
    let snapshot = local(&mut app).meta_artist_opened(Ok(Uuid::from_u128(9)));
    let _ = publish(&mut app, snapshot);
    assert_eq!(app.screen, Screen::Artist(Uuid::from_u128(9)));

    let _ = app.update(Message::OpenMetaArtist(6));
    let snapshot = local(&mut app).meta_artist_opened(Err(AppError::Network));
    let _ = publish(&mut app, snapshot);
    assert!(matches!(&app.notice, Some(Notice::Error(_))), "{:?}", app.notice);
    assert_eq!(app.screen, Screen::Artist(Uuid::from_u128(9)), "a failure does not move the page");
}

/// Passo 4.4: l'elenco della libreria arriva a pagine, in ordine; un altro ordine ricarica da capo.
#[test]
fn library_pages_are_appended_in_order_and_a_new_order_reloads() {
    let mut app = App::for_tests();
    let _ = app.update(Message::Navigate(Screen::Library));
    assert!(matches!(app.library.tracks, Load::Loading));
    let more = |n: std::ops::RangeInclusive<u128>, next| Page {
        items: n.map(|n| track(n, 180)).collect::<Vec<_>>(),
        limit: 50,
        offset: 0,
        next_offset: next,
    };
    let snapshot = local(&mut app).library_tracks_loaded(false, Ok(more(1..=2, Some(2))));
    let _ = publish(&mut app, snapshot);
    assert_eq!(app.library.tracks.ready().map(|t| t.len()), Some(2));
    assert_eq!(app.library.next_offset, Some(2));

    let _ = app.update(Message::LibraryMore);
    assert!(app.library.loading_more);
    let snapshot = local(&mut app).library_tracks_loaded(true, Ok(more(3..=3, None)));
    let _ = publish(&mut app, snapshot);
    let ids: Vec<Uuid> =
        app.library.tracks.ready().map(|t| t.iter().map(|t| t.id).collect()).unwrap_or_default();
    assert_eq!(ids, [id(1), id(2), id(3)]);
    assert!(!app.library.loading_more && app.library.next_offset.is_none());

    let _ = app.update(Message::LibrarySort(crate::api::Sort::Title));
    assert!(matches!(app.library.tracks, Load::Loading), "a new order starts again");
    assert_eq!(app.library.sort, crate::api::Sort::Title);
}

// ---------- Importazioni e download ----------
//
// Dal passo 4.5 la coda delle importazioni è del core (`core/src/state/external.rs`, che prova
// l'ordine, la pausa, gli appuntamenti con la libreria e la versione della vista). Qui c'è l'app:
// i messaggi diventano comandi, la vista si copia, e gli eventi del core diventano gli avvisi.

fn item(id: &str) -> SourceItem {
    SourceItem {
        source: SourceKind::Deezer,
        id: id.into(),
        title: format!("Brano {id}"),
        artist: "Artista".into(),
        duration_ms: Some(180_000),
        cover_url: None,
    }
}

fn evening() -> SourcePlaylist {
    SourcePlaylist {
        source: SourceKind::Deezer,
        id: "playlist/1".into(),
        title: "Sera".into(),
        owner: "Psytify".into(),
        cover_url: None,
        kind: "playlist".into(),
        released_on: None,
    }
}

/// Apre la playlist «Sera» del catalogo, con questi brani, e la salva nella libreria.
fn save_playlist(app: &mut App, items: Vec<SourceItem>) -> Uuid {
    let _ = app.update(Message::OpenSourcePlaylist(evening()));
    let ticket = local(app)
        .take_jobs()
        .into_iter()
        .find_map(|job| match job {
            PendingJob::ExternalPlaylist { ticket, .. } => Some(ticket),
            _ => None,
        })
        .expect("la playlist si chiede al server");
    let page = SourcePlaylistPage {
        source: SourceKind::Deezer,
        id: "playlist/1".into(),
        title: "Sera".into(),
        owner: "Psytify".into(),
        items,
    };
    let snapshot = local(app).playlist_loaded(ticket, SourceKind::Deezer, Ok(page));
    drop(publish(app, snapshot));
    let _ = app.update(Message::SaveSourcePlaylist);
    app.playlists.last().map(|p| p.id).expect("la playlist è salvata")
}

/// Le importazioni che il core ha chiesto dall'ultima volta: dove vanno e che cosa.
fn imports(app: &mut App) -> Vec<(ImportTarget, Importing, bool)> {
    local(app)
        .take_jobs()
        .into_iter()
        .filter_map(|job| match job {
            PendingJob::Import { target, key, video } => Some((target, key, video)),
            _ => None,
        })
        .collect()
}

fn imported(app: &mut App, target: ImportTarget, result: Result<DiscoverImport, AppError>) {
    let snapshot = local(app).imported(target, result);
    drop(publish(app, snapshot));
}

fn one(n: u128) -> Result<DiscoverImport, AppError> {
    Ok(DiscoverImport { album: None, tracks: vec![track(n, 180)] })
}

fn importing(app: &App) -> Vec<&'static str> {
    ["a", "b", "c"].into_iter().filter(|id| app.external.importing(SourceKind::Deezer, id)).collect()
}

fn deezer(id: &str) -> Importing {
    Importing::Source(SourceKind::Deezer, id.into())
}

/// `imports_run_one_at_a_time`: l'app mostra la riga «Apro…» e gli avvisi che il core pubblica; il
/// numero nell'avviso finale concorda (BACKLOG 8).
#[tokio::test(start_paused = true)]
async fn imports_run_one_at_a_time() {
    let mut app = App::for_tests();
    let playlist = save_playlist(&mut app, vec![item("a"), item("b"), item("c")]);
    assert_eq!(
        app.notice,
        Some(Notice::Info("«Sera» è nella libreria: i 3 brani arrivano uno alla volta.".into()))
    );
    assert_eq!(importing(&app), ["a"]);
    assert_eq!(app.screen, Screen::Playlist(playlist), "la playlist nuova si apre");

    let target = ImportTarget::Playlist(playlist);
    assert_eq!(imports(&mut app), [(target, deezer("a"), false)], "una richiesta alla volta");
    imported(&mut app, target, one(1));
    assert_eq!(importing(&app), ["b"]);
    let missing = AppError::Api { status: 404, code: "not_found".into(), message: "Non trovato".into() };
    imported(&mut app, target, Err(missing));
    assert_eq!(importing(&app), ["c"], "un brano non trovato non ferma gli altri");
    imported(&mut app, target, one(3));
    assert!(importing(&app).is_empty());

    let saved = app.playlist(playlist).expect("la playlist c'è ancora");
    assert_eq!(saved.tracks.iter().map(|t| t.id).collect::<Vec<_>>(), [id(1), id(3)]);
    assert!(saved.pending.is_empty());
    // BACKLOG 8: prima diceva «1 non disponibili».
    assert_eq!(app.notice, Some(Notice::Info("«Sera» è pronta: 2 brani, 1 non disponibile.".into())));
}

#[tokio::test(start_paused = true)]
async fn a_rate_limit_pauses_then_resumes() {
    let mut app = App::for_tests();
    let playlist = save_playlist(&mut app, vec![item("a"), item("b")]);
    let target = ImportTarget::Playlist(playlist);
    let limited =
        AppError::Api { status: 429, code: "rate_limited".into(), message: "Troppe richieste".into() };
    drop(imports(&mut app));

    imported(&mut app, target, Err(limited));
    assert!(importing(&app).is_empty());
    assert!(app.external.imports.paused, "la pausa è nella vista del core");
    assert!(imports(&mut app).is_empty(), "in pausa non parte nulla");
    assert!(local(&mut app).import_resume_pending(), "il timer della pausa è del core");
    assert_eq!(app.playlist(playlist).map(|p| p.pending.len()), Some(2), "il brano resta da importare");

    // Dopo venti secondi si riprende, dallo stesso brano.
    let snapshot = local(&mut app).fire_import_resume();
    drop(publish(&mut app, snapshot));
    assert_eq!(importing(&app), ["a"]);
    assert_eq!(imports(&mut app), [(target, deezer("a"), false)]);
}

/// BACKLOG 24 e 17: premere play su un risultato del catalogo ufficiale è un comando solo; la riga
/// «Apro…» viene dalla vista del core, e un fallimento è un avviso, non un `Task` dell'app.
#[tokio::test(start_paused = true)]
async fn playing_a_search_result_is_one_core_command_and_failures_become_notices() {
    let mut app = App::for_tests();
    let _ = app.update(Message::PlaySource(SourceKind::Deezer, "7".into()));
    assert!(app.external.importing(SourceKind::Deezer, "7"));
    let click = ImportTarget::Click(1);
    // Il videoclip si chiede con il brano se questa build lo mostra e il pannello è aperto.
    assert_eq!(imports(&mut app), [(click, deezer("7"), cfg!(feature = "video") && app.panel_video)]);

    let missing = AppError::Api { status: 404, code: "not_found".into(), message: "Non trovato".into() };
    imported(&mut app, click, Err(missing.clone()));
    assert!(!app.external.importing(SourceKind::Deezer, "7"));
    assert_eq!(app.notice, Some(Notice::Error(missing.to_string())));

    let _ = app.update(Message::PlayDiscovered("vuoto".into()));
    assert!(app.external.importing_archive("vuoto"));
    imported(&mut app, ImportTarget::Click(2), Ok(DiscoverImport { album: None, tracks: Vec::new() }));
    assert_eq!(app.notice, Some(Notice::Error("Questo contenuto non ha brani da ascoltare.".into())));

    // Il brano importato suona.
    let _ = app.update(Message::PlaySource(SourceKind::Deezer, "8".into()));
    imported(&mut app, ImportTarget::Click(3), one(8));
    assert_eq!(app.queue.current().map(|t| t.id), Some(id(8)));
    assert!(app.external.imports.play.is_none());
}

/// «Altro di …»: la scheda dell'artista la chiede il core quando la pagina è aperta, e l'app la legge
/// dalla vista.
#[tokio::test(start_paused = true)]
async fn the_artist_extras_come_from_the_core_view() {
    let mut app = App::for_tests();
    let artist = Uuid::from_u128(77);
    let _ = app.update(Message::Navigate(Screen::Artist(artist)));
    let snapshot = local(&mut app).catalog_artist(artist, false, Ok(artist_page(artist, "Artista 77")));
    drop(publish(&mut app, snapshot));
    let asked = local(&mut app)
        .take_jobs()
        .into_iter()
        .any(|job| matches!(job, PendingJob::ArtistExtras { id, .. } if id == artist));
    assert!(asked, "il core chiede da solo la scheda");
    assert!(matches!(app.external.artist(artist), Some(Load::Loading)));

    let extras = MetaArtistPage {
        artist: MetaArtist { id: 9, name: "Artista 77".into(), image_url: None, fans: 3 },
        top: vec![item("t1")],
        albums: vec![evening()],
    };
    let snapshot = local(&mut app).artist_extras_loaded(artist, Ok(extras));
    drop(publish(&mut app, snapshot));
    let loaded = app.external.artist(artist).and_then(|load| load.ready().cloned());
    assert_eq!(loaded.map(|extras| (extras.top.len(), extras.albums.len())), Some((1, 1)));
}

/// I brani il cui download il core ha fatto partire dall'ultima volta, in ordine.
fn started(app: &mut App) -> Vec<Uuid> {
    local(app)
        .take_jobs()
        .into_iter()
        .filter_map(|job| match job {
            PendingJob::Download { track } => Some(track),
            _ => None,
        })
        .collect()
}

/// Il download di `track` finisce, e l'app riceve ciò che il core pubblica.
fn downloaded(app: &mut App, track: Uuid, result: Result<Downloaded, AppError>) {
    let snapshot = local(app).downloaded(track, result);
    drop(publish(app, snapshot));
}

/// `download_all_runs_two_at_a_time_and_reports_once`: la coda dei download è del core
/// (`core/src/state/downloads.rs`, che prova anche il tempo e l'avanzamento); qui c'è l'app, che
/// mostra il conteggio del lotto e dice un solo avviso alla fine.
#[tokio::test(start_paused = true)]
async fn download_all_runs_two_at_a_time_and_reports_once() {
    let mut app = App::for_tests();
    let mut tracks: Vec<Track> = (1..=5).map(|n| track(n, 180)).collect();
    for track in &mut tracks[..4] {
        track.downloadable = true;
    }
    let saved = |n: u128| {
        Ok(Downloaded {
            path: PathBuf::from(format!("{n}.flac")),
            size: 1,
            sha256: String::new(),
            format: "flac".into(),
            delivered: None,
            asked: DownloadQuality::Lossless,
        })
    };

    let _ = app.update(Message::DownloadAll(Arc::from(tracks)));
    assert_eq!(app.downloads.batch_progress(), Some((0, 4)), "il brano che non si può scaricare non conta");
    assert_eq!(started(&mut app), [id(1), id(2)], "due alla volta");
    assert!(app.downloads.is_busy(id(3)) && !app.downloads.is_busy(id(5)));

    downloaded(&mut app, id(1), saved(1));
    assert_eq!((app.downloads.batch_progress(), started(&mut app)), (Some((1, 4)), vec![id(3)]));
    downloaded(&mut app, id(2), Err(AppError::Network));
    assert_eq!((app.downloads.batch_progress(), started(&mut app)), (Some((2, 4)), vec![id(4)]));
    downloaded(&mut app, id(3), saved(3));
    assert_eq!((app.downloads.batch_progress(), started(&mut app)), (Some((3, 4)), Vec::new()));
    assert_eq!(app.notice, None, "nessun avviso brano per brano");

    // Un solo avviso alla fine, con l'ultimo errore.
    downloaded(&mut app, id(4), saved(4));
    assert_eq!(app.downloads.batch_progress(), None);
    assert!(!app.downloads.is_busy(id(4)));
    assert_eq!(app.notice, Some(Notice::Error(format!("Scaricati 3 brani su 4. {}", AppError::Network))));
}

/// Un brano chiesto da solo dice dov'è finito, e uno che non si può scaricare lo dice senza partire.
#[tokio::test(start_paused = true)]
async fn a_single_download_reports_where_it_landed_and_a_locked_one_says_why() {
    let mut app = App::for_tests();
    let locked = track(1, 180);
    let _ = app.update(Message::Download(locked));
    assert!(started(&mut app).is_empty());
    assert!(matches!(&app.notice, Some(Notice::Info(text)) if text.contains("la licenza non lo permette")));

    let open = Track { downloadable: true, ..track(2, 180) };
    let _ = app.update(Message::Download(open));
    assert_eq!(started(&mut app), [id(2)]);
    assert!(app.downloads.is_busy(id(2)));
    let saved = Downloaded {
        path: PathBuf::from("/musica/Brano 2.flac"),
        size: 1,
        sha256: String::new(),
        format: "flac".into(),
        delivered: Some(DownloadQuality::Lossless),
        asked: DownloadQuality::Lossless,
    };
    downloaded(&mut app, id(2), Ok(saved));
    assert!(!app.downloads.is_busy(id(2)));
    assert_eq!(app.notice, Some(Notice::Info("Salvato in /musica/Brano 2.flac · qualità lossless".into())));
}

/// Un brano scaricato entra nell'elenco dei brani sul dispositivo (la lista che la pagina «Scaricati»
/// mostra), non si scarica una seconda volta, e «Rimuovi» lo toglie subito e lo dice quando il file è
/// andato.
#[tokio::test(start_paused = true)]
async fn a_saved_track_joins_the_device_list_and_removing_it_says_so() {
    let mut app = App::for_tests();
    let open = Track { downloadable: true, ..track(2, 180) };
    let _ = app.update(Message::Download(open));
    assert_eq!(started(&mut app), [id(2)]);
    assert!(app.downloaded.is_empty());
    let saved = Downloaded {
        path: PathBuf::from("/musica/Brano 2.flac"),
        size: 31_457_280,
        sha256: "abc".into(),
        format: "flac".into(),
        delivered: Some(DownloadQuality::Lossless),
        asked: DownloadQuality::Lossless,
    };
    downloaded(&mut app, id(2), Ok(saved));
    assert_eq!(app.downloaded.iter().map(|track| track.id).collect::<Vec<_>>(), [id(2)]);
    assert!(app.downloads.is_saved(id(2)) && app.downloads.is_offline_ready(id(2)));
    assert_eq!(app.downloads.used_bytes, 31_457_280);

    // Già sul dispositivo: il core lo dice e non riparte.
    let _ = app.update(Message::Download(Track { downloadable: true, ..track(2, 180) }));
    assert!(started(&mut app).is_empty());
    assert_eq!(app.notice, Some(Notice::Info("Questo brano è già sul dispositivo".into())));

    let _ = app.update(Message::DownloadRemove(id(2)));
    assert!(app.downloaded.is_empty() && !app.downloads.is_saved(id(2)), "la voce sparisce subito");
    let jobs = local(&mut app).take_jobs();
    assert!(jobs.contains(&PendingJob::Remove { title: "Brano 2".into() }), "{jobs:?}");
    let snapshot = local(&mut app).removed("Brano 2", Ok(false));
    drop(publish(&mut app, snapshot));
    assert_eq!(app.notice, Some(Notice::Info("«Brano 2» tolto dal dispositivo".into())));
}

// ---------- Social e jam ----------

/// La presenza e le playlist pubbliche le manda il core (passo 4.7): qui l'app, con il core sincrono
/// dei test, che dice quali lavori sociali parte a ogni messaggio. Il tempo è quello vero di
/// `Instant::now()`: i casi che dipendono dai secondi (lo scarto, l'attesa fra due invii) li fissano i
/// test del riduttore, con il tempo finto (`core/src/state/social.rs`).
#[tokio::test(start_paused = true)]
async fn presence_is_sent_on_drift_not_per_message() {
    let (mut app, mut mixer) = app_with_audio();
    sign_in(&mut app);
    let presence = |works: Vec<SocialWork>| -> Vec<(Option<Uuid>, bool, u64)> {
        works
            .into_iter()
            .filter_map(|work| match work {
                SocialWork::Presence { track, paused, position_ms } => Some((track, paused, position_ms)),
                _ => None,
            })
            .collect()
    };

    // BACKLOG 9, corretto in 4.7: mentre il brano si carica non parte niente (prima partiva «in
    // pausa», poi di nuovo appena suonava: due invii per ogni brano). Ne parte uno solo, quando suona.
    let _ = app.update(Message::PlayTracks(Arc::from(vec![track(1, 20)]), 0));
    assert!(presence(social_jobs(&mut app)).is_empty(), "in caricamento non si dice niente");
    let generation = app.playback.generation;
    let _ = loaded(&mut app, generation, Ok(tone(20)));
    let sent = presence(social_jobs(&mut app));
    assert_eq!(sent.len(), 1, "{sent:?}");
    let (track_now, paused, position_ms) = sent[0];
    assert_eq!((track_now, paused), (Some(id(1)), false));
    assert!(position_ms < 50, "{position_ms}");

    // Frame e Tick non rimandano nulla: uno scarto sotto i cinque secondi non conta.
    listen(&mut mixer, Duration::from_secs(1));
    let _ = tick(&mut app);
    let _ = app.update(Message::Frame);
    let _ = app.update(Message::Tick);
    assert!(near(app.position(), Duration::from_secs(1)));
    assert!(social_jobs(&mut app).is_empty());

    // Un salto lontano subito dopo un invio aspetta: il core arma un attesa invece di mandare.
    let _ = app.update(Message::SeekBy(10));
    assert!(social_jobs(&mut app).is_empty(), "fra due invii passano almeno due secondi");
    assert!(app.local_core.as_ref().is_some_and(|core| core.presence_wait_pending()));

    // La pausa parte subito.
    let _ = app.update(Message::TogglePlay);
    let sent = presence(social_jobs(&mut app));
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert!(sent[0].1, "in pausa");
    // E il core non resta armato per un salto che non conta più.
    let _ = app.update(Message::Frame);
    assert!(social_jobs(&mut app).is_empty());
}

#[tokio::test(start_paused = true)]
async fn public_playlists_sync_by_fingerprint() {
    let mut app = App::for_tests();
    sign_in(&mut app);
    let playlist = Playlist::new("Sera", vec![track(1, 180)]);
    let id = playlist.id;
    app.send_library_quiet(LibraryCmd::CreatePlaylist {
        id,
        name: playlist.name,
        tracks: playlist.tracks,
        pending: Vec::new(),
    });
    let _ = jobs(&mut app);

    // Quante richieste al server partono dopo ogni messaggio: una quando la playlist pubblica cambia
    // (nome o brani) o torna privata, nessuna altrimenti.
    let steps = [
        ("pubblicata", Message::PlaylistPublic(id, true), 1),
        ("frame", Message::Frame, 0),
        ("brano aggiunto", Message::AddToPlaylist(id, track(2, 180)), 1),
        ("frame", Message::Frame, 0),
        ("tolta dal profilo", Message::PlaylistPublic(id, false), 1),
        ("frame", Message::Frame, 0),
        ("ripubblicata", Message::PlaylistPublic(id, true), 1),
    ];
    for (step, message, expected) in steps {
        let _ = drain(app.update(message)).await;
        let synced = social_jobs(&mut app)
            .into_iter()
            .filter(|work| {
                matches!(work, SocialWork::PublishPlaylist { .. } | SocialWork::UnpublishPlaylist(_))
            })
            .count();
        assert_eq!(synced, expected, "{step}");
    }
}

/// Il nome del profilo di prova di `sign_in`.
fn me_named(name: &str) -> crate::api::Me {
    crate::api::Me {
        user_id: "user_prova".into(),
        admin: false,
        profile: Some(crate::api::social::MyProfile {
            handle: name.to_lowercase(),
            display_name: name.into(),
            avatar_url: None,
            share_activity: true,
        }),
    }
}

fn clerk() -> psytify_core::auth::ClerkApp {
    psytify_core::auth::ClerkApp {
        issuer: crate::app::TEST_CLERK_ISSUER.into(),
        port: 53682,
        session_file: None,
    }
}

/// Dal passo 4.7 l'accesso sta nel core: il modulo manda i suoi campi come `SocialCmd`, la password
/// non resta nel campo e un secondo tocco non rimanda niente. L'app dice com'è andata dalla vista.
#[tokio::test(start_paused = true)]
async fn the_sign_in_form_goes_to_the_core_and_the_password_does_not_stay() {
    let mut app = App::for_tests();
    let _ = app.update(Message::SignIn);
    assert_eq!(app.screen, Screen::SignIn);
    let _ = app.update(Message::AuthInput(AuthField::Identifier, "psy".into()));
    let _ = app.update(Message::AuthInput(AuthField::Password, "una-password".into()));
    assert!(app.social.form.can_submit(&app.social.view.auth));

    let _ = app.update(Message::AuthSubmit);
    assert!(app.social.form.password.is_empty(), "la password è partita e non resta nel campo");
    assert_eq!(social_jobs(&mut app), [SocialWork::SignIn]);
    assert_eq!(app.social.view.account, Account::Working);
    assert!(app.social.view.auth.busy);
    // Un secondo tocco, o un provider, mentre un invio è in volo non parte.
    let _ = app.update(Message::AuthSubmit);
    let _ = app.update(Message::AuthProvider(crate::auth::Provider::GitHub));
    assert!(social_jobs(&mut app).is_empty());

    // Il tentativo fallisce: l'errore sta sul modulo, finché non si riprende a scrivere.
    let snapshot = app.local_core.as_mut().and_then(|core| core.sign_in_failed(AppError::AuthFailed));
    let _ = app.update(Message::Core(snapshot.expect("il core pubblica")));
    assert_eq!(app.social.view.auth.error, Some(AppError::AuthFailed));
    assert_eq!(app.social.view.account, Account::SignedOut);
    let _ = app.update(Message::AuthInput(AuthField::Password, "x".into()));
    assert!(app.social.view.auth.error.is_none(), "riprendere a scrivere toglie l'errore");
}

#[tokio::test(start_paused = true)]
async fn a_code_step_shows_the_code_field_and_sends_the_code() {
    let mut app = App::for_tests();
    let _ = app.update(Message::AuthMode(AuthMode::SignUp));
    let _ = app.update(Message::AuthInput(AuthField::Email, "psy@example.com".into()));
    let _ = app.update(Message::AuthInput(AuthField::Password, "abcdefgh".into()));
    let _ = app.update(Message::AuthSubmit);
    assert_eq!(social_jobs(&mut app), [SocialWork::SignUp]);

    let snapshot = app.local_core.as_mut().and_then(|core| core.sign_in_needs_code("p**@example.com"));
    let _ = app.update(Message::Core(snapshot.expect("il core pubblica")));
    assert_eq!(app.social.view.auth.step.as_ref().map(|s| s.sent_to.as_str()), Some("p**@example.com"));
    assert!(!app.social.view.auth.busy);

    let _ = app.update(Message::AuthInput(AuthField::Code, "424242".into()));
    assert!(app.social.form.can_submit(&app.social.view.auth));
    let _ = app.update(Message::AuthSubmit);
    assert!(app.social.form.code.is_empty(), "il codice è partito e non resta nel campo");
    assert_eq!(social_jobs(&mut app), [SocialWork::SubmitCode]);

    // «Indietro» dal codice torna ai campi.
    let snapshot = app.local_core.as_mut().and_then(|core| core.sign_in_failed(AppError::AuthFailed));
    let _ = app.update(Message::Core(snapshot.expect("il core pubblica")));
    let _ = app.update(Message::AuthBack);
    assert!(app.social.view.auth.step.is_none());
}

/// L'accesso riuscito: il core saluta con un evento, l'app lo mostra come avviso, esce dal modulo e
/// riempie i campi del profilo; l'uscita butta ciò che era dell'utente.
#[tokio::test(start_paused = true)]
async fn signing_in_leaves_the_form_and_signing_out_forgets_the_user() {
    let mut app = App::for_tests();
    let _ = app.update(Message::SignIn);
    assert_eq!(app.screen, Screen::SignIn);
    let snapshot = app
        .local_core
        .as_mut()
        .and_then(|core| core.signed_in(psytify_core::auth::Session::for_tests(clerk()), me_named("Prova")));
    let _ = app.update(Message::Core(snapshot.expect("il core pubblica")));
    assert_eq!(app.notice, Some(Notice::Info("Ciao Prova!".into())));
    assert_ne!(app.screen, Screen::SignIn, "dal modulo si esce");
    assert_eq!((app.social.edit.handle.as_str(), app.social.edit.display_name.as_str()), ("prova", "Prova"));
    assert_eq!(app.signed_in_profile().map(|p| p.handle.as_str()), Some("prova"));
    assert!(social_jobs(&mut app).contains(&SocialWork::Friends), "gli amici si leggono subito");

    app.social.invitees.push("luca".into());
    let _ = app.update(Message::SignOut);
    assert_eq!(app.social.view.account, Account::SignedOut);
    assert!(app.social.invitees.is_empty() && app.social.edit.handle.is_empty());
    assert_eq!(social_jobs(&mut app), [SocialWork::SignOut]);
    assert!(
        app.local_core.as_ref().is_some_and(|core| !core.social_poll_pending()),
        "uscito, nessun sondaggio"
    );
}

/// Un evento del core si mostra una volta, nell'ordine, e un'altra pubblicazione del dominio non lo
/// rimostra: gli inviti alle jam si annunciano una volta sola, e non quello della stanza in cui si è
/// (il dominio sociale lo sa dalla jam del core, non dal frontend).
#[tokio::test(start_paused = true)]
async fn social_events_are_shown_once_and_the_room_code_reaches_the_social_domain() {
    let mut app = App::for_tests();
    sign_in(&mut app);
    let invite = |code: &str, from: &str| crate::api::social::JamInvite {
        code: code.into(),
        from_handle: from.into(),
        from_name: from.to_uppercase(),
    };
    let invites = |app: &mut App, list: Vec<crate::api::social::JamInvite>| {
        let snapshot = app.local_core.as_mut().and_then(|core| core.invites_loaded(Ok(list)));
        if let Some(snapshot) = snapshot {
            let _ = app.update(Message::Core(snapshot));
        }
    };

    invites(&mut app, vec![invite("K7QXM3", "luca")]);
    assert!(matches!(&app.notice, Some(Notice::Info(text)) if text.contains("LUCA")));
    assert_eq!(app.social.view.jam_invites.len(), 1);
    app.notice = None;
    // Un altro invito, uno vecchio rimostrato: si annuncia solo il nuovo.
    invites(&mut app, vec![invite("K7QXM3", "luca"), invite("ZZZ999", "sara")]);
    assert!(matches!(&app.notice, Some(Notice::Info(text)) if text.contains("SARA")), "{:?}", app.notice);
    app.notice = None;
    invites(&mut app, vec![invite("K7QXM3", "luca"), invite("ZZZ999", "sara")]);
    assert!(app.notice.is_none(), "lo stesso invito non si annuncia di nuovo");

    // In una jam con quel codice, l'invito a quella stanza non si annuncia: il codice lo sa il
    // core, che lo legge dalla vista della jam (passo 4.8, BACKLOG 26).
    let _remote = join(&mut app, true);
    invites(&mut app, vec![invite("K7QXM3", "luca"), invite("ZZZ999", "sara"), invite("K7QXM3X", "anna")]);
    assert!(
        !matches!(&app.notice, Some(Notice::Info(text)) if text.contains("LUCA")),
        "la propria stanza non si annuncia: {:?}",
        app.notice
    );
    // Toglierlo lo toglie subito dalla vista, e il server lo viene a sapere.
    let _ = app.update(Message::JamInviteDismiss("ZZZ999".into()));
    assert!(app.social.view.jam_invites.iter().all(|i| i.code != "ZZZ999"));
    assert!(social_jobs(&mut app).contains(&SocialWork::DismissInvite("ZZZ999".into())));
}

#[tokio::test(start_paused = true)]
async fn opening_a_profile_needs_an_account_and_loads_it_from_the_core() {
    let mut app = App::for_tests();
    let _ = app.update(Message::OpenProfile("luca".into()));
    assert_eq!(app.screen, Screen::SignIn, "senza accesso si chiede di accedere");
    assert!(social_jobs(&mut app).is_empty());

    sign_in(&mut app);
    let _ = app.update(Message::OpenProfile("luca".into()));
    assert_eq!(app.screen, Screen::Profile);
    assert_eq!(social_jobs(&mut app), [SocialWork::OpenProfile("luca".into())]);
    assert!(matches!(&app.social.view.open, Some((handle, Load::Loading)) if handle == "luca"));

    // Le richieste d'amicizia ricevute stanno accanto ad «Amici».
    let friends = crate::api::social::Friends {
        incoming: vec![crate::api::social::Profile {
            handle: "sara".into(),
            display_name: "Sara".into(),
            avatar_url: None,
        }],
        ..Default::default()
    };
    let snapshot = app.local_core.as_mut().and_then(|core| core.friends_loaded(Ok(friends)));
    let _ = app.update(Message::Core(snapshot.expect("il core pubblica")));
    assert_eq!(app.friend_requests(), 1);
}

/// Una stanza della jam, vista dall'app: il collegamento è del core e della sua host, qui gli eventi
/// della stanza si danno a mano al core sincrono, come li darebbe il collegamento, e l'app riceve
/// l'istantanea che ne esce.
fn room(app: &mut App, event: Event) {
    let snapshot = local(app).jam_event(event);
    drop(publish(app, snapshot));
}

/// Quel che il core ha chiesto di mandare alla stanza dall'ultima volta (gli stati locali).
fn sent_to_the_room(app: &mut App) -> Vec<Snapshot> {
    local(app)
        .take_jam()
        .into_iter()
        .filter_map(|op| match op {
            JamOp::Send(snapshot) => Some(snapshot),
            _ => None,
        })
        .collect()
}

/// Entra in una jam già avviata da Luca, ferma sul secondo brano a 12 secondi: l'app chiede di
/// entrare con il codice digitato (il core chiede il collegamento all'host) e l'ingresso nella stanza
/// arriva a mano, con lo stato del gruppo. Restituisce lo stato del gruppo.
fn join(app: &mut App, playing: bool) -> Remote {
    let _ = app.update(Message::JamCode("k7qxm3".into()));
    assert_eq!(app.jam_code, "K7QXM3", "il codice si scrive in maiuscolo");
    let _ = app.update(Message::JamJoin);
    assert!(
        matches!(local(app).take_jam().as_slice(), [JamOp::Start { target: Target::Join(code), .. }] if code == "K7QXM3")
    );
    assert!(app.jam.is_active(), "il collegamento è partito: la stanza non ha ancora risposto");
    let remote = Remote {
        snapshot: Snapshot { queue: vec![id(1), id(2)], index: 1, position_ms: 12_000, playing },
        from: "Luca".into(),
    };
    let members = vec![
        Member { id: Uuid::from_u128(77), name: "Prova".into() },
        Member { id: Uuid::from_u128(78), name: "Luca".into() },
    ];
    room(
        app,
        Event::Joined {
            code: "K7QXM3".into(),
            you: Uuid::from_u128(77),
            members,
            state: Some(remote.clone()),
        },
    );
    remote
}

#[tokio::test(start_paused = true)]
async fn a_late_member_aligns_to_the_shared_position() {
    let (mut app, mut mixer) = app_with_audio();
    let _ = join(&mut app, true);

    // L'ingresso: i brani del gruppo mancano, si chiedono al catalogo e intanto alla stanza non si
    // manda nulla. BACKLOG 10: l'avviso «Sei nella jam K7QXM3» resta (il core non dice «ha cambiato
    // brano» per lo stato che arriva con l'ingresso).
    assert_eq!(app.jam.code.as_deref(), Some("K7QXM3"));
    assert_eq!((app.jam.you, app.jam.members.len()), (Some(Uuid::from_u128(77)), 2));
    assert!(local(&mut app).take_jobs().contains(&PendingJob::JamTracks { ids: vec![id(1), id(2)] }));
    assert!(sent_to_the_room(&mut app).is_empty());
    assert_eq!(app.notice, Some(Notice::Info("Sei nella jam K7QXM3".into())));
    assert!(!app.needs_tick(), "una jam ferma non vuole il Tick");

    // Arrivati i brani, in un ordine qualsiasi, la coda prende quello del gruppo e parte il brano
    // condiviso. Mentre si carica, la stanza non riceve nulla.
    let snapshot = local(&mut app).jam_tracks_loaded(Ok(vec![track(2, 20), track(1, 20)]));
    drop(publish(&mut app, snapshot));
    assert_eq!(app.queue.entries().map(|(_, t)| t.id).collect::<Vec<_>>(), [id(1), id(2)]);
    assert_eq!(app.queue.position(), Some(1));
    assert_eq!(app.playback.state, PlaybackState::Loading { track_id: id(2) });
    assert!(sent_to_the_room(&mut app).is_empty(), "un brano del gruppo in caricamento non si annuncia");

    // L'audio pronto parte dalla posizione del gruppo, non da capo.
    let generation = app.playback.generation;
    let _ = loaded(&mut app, generation, Ok(tone(20)));
    assert_eq!(app.playback.state, PlaybackState::Playing);
    assert!(near(app.position(), Duration::from_secs(12)), "{:?}", app.position());
    listen(&mut mixer, Duration::from_millis(100));
    let _ = tick(&mut app);
    // Dal punto del gruppo, più al massimo ciò che si è appena sentito (il seek finisce sul thread
    // audio: fino ad allora la posizione è il punto chiesto).
    let heard = app.position();
    assert!((Duration::from_secs(12)..=Duration::from_millis(12_250)).contains(&heard), "{heard:?}");
    assert!(sent_to_the_room(&mut app).is_empty(), "seguire il gruppo non si rimanda al gruppo");
}

#[tokio::test(start_paused = true)]
async fn a_late_member_of_a_paused_jam_waits_at_the_shared_position() {
    let (mut app, _mixer) = app_with_audio();
    let _ = join(&mut app, false);
    let snapshot = local(&mut app).jam_tracks_loaded(Ok(vec![track(1, 20), track(2, 20)]));
    drop(publish(&mut app, snapshot));
    let generation = app.playback.generation;
    let _ = loaded(&mut app, generation, Ok(tone(20)));
    assert_eq!(app.playback.state, PlaybackState::Paused, "il gruppo è in pausa: ci si ferma lì");
    assert!(near(app.position(), Duration::from_secs(12)), "{:?}", app.position());

    // Lo stato è quello appena ricevuto: non si rimanda alla stanza come se fosse nuovo.
    assert!(sent_to_the_room(&mut app).is_empty());
    // Il primo cambio locale, invece, si annuncia.
    let _ = app.update(Message::TogglePlay);
    let sent = sent_to_the_room(&mut app);
    let [snapshot] = sent.as_slice() else { panic!("la ripresa si annuncia alla stanza: {sent:?}") };
    assert_eq!((&snapshot.queue, snapshot.index, snapshot.playing), (&vec![id(1), id(2)], 1, true));
    assert!(snapshot.position_ms.abs_diff(12_000) <= 50, "{}", snapshot.position_ms);
}

/// I messaggi della schermata Jam diventano comandi del core: creare con il nome scritto, entrare
/// con il codice (un codice sbagliato lo rifiuta il core con il suo avviso), uscire; la vista del
/// core è ciò che la schermata legge, e chi esce dimentica la scelta degli amici da invitare.
#[tokio::test(start_paused = true)]
async fn the_jam_messages_become_core_commands_and_the_view_comes_back() {
    let mut app = App::for_tests();
    let _ = app.update(Message::JamName("Luca".into()));
    let _ = app.update(Message::JamCreate);
    assert!(matches!(
        local(&mut app).take_jam().as_slice(),
        [JamOp::Start { target: Target::Create, name, .. }] if name == "Luca"
    ));
    assert!(app.jam.is_active() && app.jam.code.is_none(), "«Connessione in corso…»");

    // Una seconda jam non parte; il core lo dice.
    let _ = app.update(Message::JamCreate);
    assert!(local(&mut app).take_jam().is_empty());
    assert_eq!(app.notice, Some(Notice::Info("Sei già in una jam".into())));

    room(
        &mut app,
        Event::Joined { code: "K7QXM3".into(), you: Uuid::from_u128(77), members: Vec::new(), state: None },
    );
    assert_eq!(app.jam.code.as_deref(), Some("K7QXM3"));
    assert_eq!(app.notice, Some(Notice::Info("Sei nella jam K7QXM3".into())));
    app.social.invitees = vec!["luca".into()];

    // Uscire: il collegamento si chiude, la vista torna a «nessuna jam», la scelta si dimentica.
    let _ = app.update(Message::JamLeave);
    assert_eq!(local(&mut app).take_jam(), [JamOp::Leave]);
    assert!(!app.jam.is_active() && app.social.invitees.is_empty());
    assert_eq!(app.notice, Some(Notice::Info("Sei uscito dalla jam".into())));

    // Un codice senza sei lettere o cifre non arriva al collegamento.
    let _ = app.update(Message::JamCode("ab".into()));
    let _ = app.update(Message::JamJoin);
    assert!(local(&mut app).take_jam().is_empty());
    assert_eq!(app.notice, Some(Notice::Error("Il codice ha 6 lettere o cifre".into())));

    // La stanza che si chiude da sé: l'errore col suo motivo, e di nuovo fuori.
    let _ = app.update(Message::JamCode("K7QXM3".into()));
    let _ = app.update(Message::JamJoin);
    assert!(app.jam.is_active());
    app.social.invitees = vec!["sara".into()];
    room(&mut app, Event::Closed("Jam chiusa dal server".into()));
    assert!(!app.jam.is_active() && app.social.invitees.is_empty());
    assert_eq!(app.notice, Some(Notice::Error("Jam chiusa dal server".into())));
}

/// Accettare l'invito di un amico entra nella sua stanza con quel codice e toglie l'invito; già in una
/// jam, si esce prima.
#[tokio::test(start_paused = true)]
async fn accepting_an_invite_joins_that_room_and_not_while_in_another() {
    let mut app = App::for_tests();
    sign_in(&mut app);
    let _ = app.update(Message::JamInviteAccept("ZZZ999".into()));
    assert!(matches!(
        local(&mut app).take_jam().as_slice(),
        [JamOp::Start { target: Target::Join(code), .. }] if code == "ZZZ999"
    ));
    assert!(social_jobs(&mut app).contains(&SocialWork::DismissInvite("ZZZ999".into())));
    assert!(app.jam.is_active());

    let _ = app.update(Message::JamInviteAccept("AAA111".into()));
    assert!(local(&mut app).take_jam().is_empty());
    assert_eq!(app.notice, Some(Notice::Info("Sei già in una jam: esci prima da quella".into())));
}
