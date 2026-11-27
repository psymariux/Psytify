//! Psytify desktop (GPUI). Senza argomenti apre l'app vera, collegata al core (`psytify-core`): ricerca,
//! coda e barra «in riproduzione». Con `--gallery` apre la galleria dei componenti (nessun core, nessuna
//! rete). Home, Libreria e Scaricati arrivano con i passi successivi del piano di migrazione.

mod actions;
mod app;
mod components;
mod core_model;
mod covers;
#[cfg(feature = "fixtures")]
mod fixtures;
mod format;
#[cfg(feature = "profile")]
mod frame_stats;
mod gallery;
mod notices;
mod pixel;
mod theme;
mod titlebar;
#[cfg(feature = "video")]
mod video;

use gpui::{
    App, AppContext, Bounds, WindowBackgroundAppearance, WindowBounds, WindowDecorations, WindowOptions, px,
    size,
};
use gpui_platform::application;
use psytify_design::tokens::size::{WINDOW_MIN_H, WINDOW_MIN_W};

/// Cosa aprire, secondo la riga di comando.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Mode {
    App(app::Launch),
    Gallery,
}

const USAGE: &str = "psytify [--gallery] [--search TESTO] [--queue] [--lyrics] [--video] [--focus] [--play-first-official] [--page PAGINA] [--fixture NOME]\n\n\
    Senza argomenti apre l'app, con la stessa configurazione del client Iced.\n\
    --gallery        apre la galleria dei componenti (nessun core, nessuna rete)\n\
    --search TESTO   apre la ricerca già compilata e la lancia\n\
    --queue          apre il pannello della coda\n\
    --lyrics         apre il pannello del testo\n\
    --video          solo con la feature `video`: apre il pannello sul videoclip; con --focus, la vista\n\
                     immersiva sul videoclip\n\
    --focus          apre la vista immersiva (se c'è un brano in coda)\n\
    --play-first-official  con --search: quando arrivano i risultati del catalogo ufficiale suona il\n\
                     primo (serve a provare lo stato «Apro…» senza toccare niente)\n\
    --page PAGINA    apre home, news, search, library, liked, downloads, playlist (la prima della libreria),\n\
                     album (il primo album salvato), artist (il primo artista seguito), settings, signin,\n\
                     friends, profile (il proprio profilo) o jam\n\
    --fixture NOME   solo con la feature `fixtures`: dati sociali, della jam o dell'aggiornamento di prova al\n\
                     posto di quelli del core (signin-code, signin-waiting, signin-error, signin-busy, social,\n\
                     social-own, social-private, social-empty, jam-invites, jam-connecting, jam-host,\n\
                     jam-guest, jam-error, update-offered, update-downloading, update-ready, update-error,\n\
                     update-check-failed; con la feature `video` anche video-loading, video-ready (il file\n\
                     di PSYTIFY_VIDEO_FIXTURE) e video-failed); i comandi sociali, della jam,\n\
                     dell'aggiornamento e del videoclip non partono\n\n\
    PSYTIFY_API_URL cambia il backend; PSYTIFY_REDUCE_MOTION=1 riduce il movimento (sprite fermi, nessuno\n\
    scorrimento), come l'interruttore delle impostazioni, che però non lo toglie.";

fn parse_args(args: impl IntoIterator<Item = String>) -> Result<Mode, String> {
    let mut args = args.into_iter();
    let mut launch = app::Launch::default();
    let mut gallery = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--gallery" => gallery = true,
            "--search" => {
                let text = args.next().ok_or("--search vuole un testo")?;
                launch.search = Some(text);
            }
            "--queue" => launch.queue = true,
            "--lyrics" => launch.lyrics = true,
            #[cfg(feature = "video")]
            "--video" => launch.video = true,
            "--focus" => launch.focus = true,
            "--play-first-official" => launch.play_first_official = true,
            "--page" => {
                let name = args.next().ok_or("--page vuole il nome di una pagina")?;
                let page = app::Page::from_arg(&name).ok_or_else(|| format!("pagina sconosciuta: {name}"))?;
                launch.page = Some(page);
            }
            #[cfg(feature = "fixtures")]
            "--fixture" => {
                let name = args.next().ok_or("--fixture vuole un nome")?;
                if !fixtures::known(&name) {
                    return Err(format!("dati di prova sconosciuti: {name} ({})", fixtures::names()));
                }
                launch.fixture = Some(name);
            }
            "--help" | "-h" => {
                println!("{USAGE}");
                std::process::exit(0);
            }
            other => return Err(format!("argomento sconosciuto: {other} (prova --help)")),
        }
    }
    Ok(if gallery { Mode::Gallery } else { Mode::App(launch) })
}

fn window_options(cx: &App) -> WindowOptions {
    let bounds = Bounds::centered(None, size(px(1280.0), px(800.0)), cx);
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: None,
        window_decorations: Some(WindowDecorations::Client),
        window_background: WindowBackgroundAppearance::Opaque,
        window_min_size: Some(size(px(WINDOW_MIN_W), px(WINDOW_MIN_H))),
        // L'`app_id` di Wayland (e la classe di X11) coincide con `client/psytify.desktop` (`StartupWMClass`):
        // così il menu e la barra delle applicazioni associano icona e finestra.
        app_id: Some("psytify".to_owned()),
        // Senza fuoco GPUI disegna al più ogni 33 ms (il suo default): con il passo di 16 ms di X11 un
        // videoclip a 30 fps ne mostrava 20,8 al secondo mentre si lavora in un'altra finestra (BACKLOG 44).
        // Qui ogni ridisegno viene da un timer o da un dato che cambia, mai da un ciclo per fotogramma, quindi
        // senza limite una finestra ferma resta ferma e una che anima va al ritmo del suo contenuto
        // (docs/PERFORMANCE.md, «QA 1»).
        inactive_frame_interval: None,
        ..WindowOptions::default()
    }
}

fn main() {
    let mode = match parse_args(std::env::args().skip(1)) {
        Ok(mode) => mode,
        Err(message) => {
            eprintln!("psytify: {message}");
            std::process::exit(2);
        }
    };
    // Prima di tutto: su Linux si rilancia una volta con un ambiente più leggero (arene di malloc, driver
    // Vulkan). Lo stesso del client Iced (`core/src/launch.rs`).
    #[cfg(target_os = "linux")]
    psytify_core::launch::relaunch_with_lean_env();
    let reduce_motion = std::env::var_os("PSYTIFY_REDUCE_MOTION").is_some();
    application().run(move |cx: &mut App| {
        theme::install_fonts(cx);
        #[cfg(feature = "profile")]
        frame_stats::install(cx);
        pixel::SpriteStore::install(cx, reduce_motion);
        components::search_field::bind_keys(cx);
        actions::bind_global(cx);
        cx.on_window_closed(|cx, _| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();
        let options = window_options(cx);
        let opened = match mode {
            Mode::Gallery => cx
                .open_window(options, |window, cx| cx.new(|cx| gallery::Gallery::new(window, cx)))
                .map(|_| ()),
            Mode::App(launch) => {
                actions::bind_shell(cx);
                let core = core_model::CoreModel::start(cx);
                #[cfg(feature = "fixtures")]
                if let Some(view) = launch.fixture.as_deref().and_then(fixtures::social) {
                    core.update(cx, |model, _| model.set_social_fixture(view));
                }
                #[cfg(feature = "fixtures")]
                if let Some(view) = launch.fixture.as_deref().and_then(fixtures::jam) {
                    core.update(cx, |model, _| model.set_jam_fixture(view));
                }
                #[cfg(feature = "fixtures")]
                if let Some(view) = launch.fixture.as_deref().and_then(fixtures::update) {
                    core.update(cx, |model, _| model.set_update_fixture(view));
                }
                #[cfg(all(feature = "fixtures", feature = "video"))]
                {
                    let current = core.read(cx).snapshot().playback.queue.current().map(|track| track.id);
                    if let Some(view) =
                        launch.fixture.as_deref().and_then(|name| fixtures::video(name, current))
                    {
                        core.update(cx, |model, _| model.set_clip_fixture(view));
                    }
                }
                covers::Covers::install(cx, core.clone());
                // «Riduci il movimento» delle impostazioni, prima del primo disegno, e poi dal vivo quando
                // cambia ciò che il sistema chiede.
                app::settings_page::follow_motion(&core, cx);
                cx.open_window(options, move |window, cx| {
                    let shell = cx.new(|cx| app::Shell::new(core, launch, window, cx));
                    // Chiudere dal gestore di finestre passa dallo stesso salvataggio del pulsante.
                    let close = app::Shell::close_handler(&shell);
                    window.on_window_should_close(cx, move |window, cx| {
                        close(window, cx);
                        false
                    });
                    shell
                })
                .map(|_| ())
            }
        };
        if let Err(error) = opened {
            eprintln!("psytify: apertura finestra fallita: {error:#}");
            cx.quit();
            return;
        }
        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| (*arg).to_owned()).collect()
    }

    #[test]
    fn the_app_opens_by_default_and_the_gallery_on_request() {
        assert_eq!(parse_args(args(&[])), Ok(Mode::App(app::Launch::default())));
        assert_eq!(parse_args(args(&["--gallery"])), Ok(Mode::Gallery));
    }

    #[test]
    fn search_and_queue_prepare_the_app() {
        let mode = parse_args(args(&["--search", "daft punk", "--queue"]));
        let launch =
            app::Launch { search: Some("daft punk".to_owned()), queue: true, ..app::Launch::default() };
        assert_eq!(mode, Ok(Mode::App(launch)));
        assert!(parse_args(args(&["--search"])).is_err());
    }

    #[test]
    fn the_first_official_result_can_be_played_from_the_command_line() {
        let mode = parse_args(args(&["--search", "marea", "--play-first-official"]));
        let launch = app::Launch {
            search: Some("marea".to_owned()),
            play_first_official: true,
            ..app::Launch::default()
        };
        assert_eq!(mode, Ok(Mode::App(launch)));
    }

    #[test]
    fn a_page_can_be_opened_from_the_command_line() {
        let mode = parse_args(args(&["--page", "liked"]));
        let launch =
            app::Launch { page: Some(app::PageArg::Page(app::Page::Liked)), ..app::Launch::default() };
        assert_eq!(mode, Ok(Mode::App(launch)));
        assert!(parse_args(args(&["--page"])).is_err());
        assert!(parse_args(args(&["--page", "nope"])).is_err());
    }

    #[test]
    fn the_lyrics_panel_and_the_focus_view_can_open_at_start() {
        let mode = parse_args(args(&["--lyrics", "--focus"]));
        let launch = app::Launch { lyrics: true, focus: true, ..app::Launch::default() };
        assert_eq!(mode, Ok(Mode::App(launch)));
    }

    #[cfg(feature = "video")]
    #[test]
    fn the_video_panel_can_open_at_start() {
        let mode = parse_args(args(&["--video", "--focus"]));
        let launch = app::Launch { video: true, focus: true, ..app::Launch::default() };
        assert_eq!(mode, Ok(Mode::App(launch)));
    }

    #[test]
    fn an_unknown_argument_is_refused() {
        assert!(parse_args(args(&["--nope"])).is_err());
    }
}
