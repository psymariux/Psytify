//! Psytify desktop (GPUI). Without arguments it opens the real app, connected to the core (`psytify-core`): search,
//! queue and the «in riproduzione» bar. With `--gallery` it opens the component gallery (no core, no
//! network). Home, Libreria and Scaricati arrive with the next steps of the migration plan.

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
mod motion;
mod notices;
mod pixel;
mod theme;
mod titlebar;
#[cfg(feature = "video")]
mod video;
mod zoom;

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

const USAGE: &str = "psytify [--gallery] [--search TEXT] [--now] [--queue] [--lyrics] [--video] [--focus] [--play-first-official] [--page PAGE] [--fixture NAME]\n\n\
    Without arguments it opens the app.\n\
    --gallery        opens the component gallery (no core, no network)\n\
    --search TEXT    opens the search already filled in and runs it\n\
    --now            opens the side panel on «In riproduzione»\n\
    --queue          opens the side panel on the queue\n\
    --lyrics         opens the side panel on the lyrics\n\
    --video          only with the `video` feature: turns the clip on in the «In riproduzione» media slot;\n\
                     with --focus, in the stage's\n\
    --focus          opens the stage (if a track is queued)\n\
    --play-first-official  with --search: plays the first official catalogue result when it arrives\n\
                     (to try the «Apro…» state without touching anything)\n\
    --page PAGE      opens home, news, search, library, liked, downloads, playlist (the library's first),\n\
                     album (the first saved album), artist (the first followed artist), settings, signin,\n\
                     friends, profile (your own) or jam\n\
    --fixture NAME   only with the `fixtures` feature: fake social, jam or update data in place of the\n\
                     core's (signin-code, signin-waiting, signin-error, signin-busy, social,\n\
                     social-own, social-private, social-empty, jam-invites, jam-connecting, jam-host,\n\
                     jam-guest, jam-error, update-offered, update-downloading, update-ready, update-error,\n\
                     update-check-failed; with the `video` feature also video-loading, video-ready (the file\n\
                     in PSYTIFY_VIDEO_FIXTURE) and video-failed); social, jam, update and clip commands\n\
                     do not run\n\n\
    PSYTIFY_API_URL changes the backend; PSYTIFY_REDUCE_MOTION=1 reduces motion (still sprites, no\n\
    scrolling), like the settings switch, which does not turn it off.";

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
            "--now" => launch.now = true,
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
        // The Wayland `app_id` (and the X11 class) matches `desktop/psytify.desktop` (`StartupWMClass`):
        // so the menu and the taskbar pair the icon with the window.
        app_id: Some("psytify".to_owned()),
        // Without focus GPUI draws at most every 33 ms (its default): with X11's 16 ms step a
        // video at 30 fps showed 20.8 per second while working in another window (BACKLOG 44).
        // Here every redraw comes from a timer or from data that changes, never from a per-frame loop, so
        // without a limit a still window stays still and an animating one runs at the pace of its content
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
    // First of all: on Linux it is relaunched once with a lighter environment (malloc arenas, driver
    // Vulkan). See `core/src/launch.rs`.
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
                // The settings' «Riduci il movimento», before the first draw, and then live when
                // what the system asks changes.
                app::settings_page::follow_motion(&core, cx);
                cx.open_window(options, move |window, cx| {
                    let shell = cx.new(|cx| app::Shell::new(core, launch, window, cx));
                    // Closing from the window manager goes through the same save as the button.
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
    fn the_clip_can_be_on_at_start() {
        let mode = parse_args(args(&["--video", "--focus"]));
        let launch = app::Launch { video: true, focus: true, ..app::Launch::default() };
        assert_eq!(mode, Ok(Mode::App(launch)));
    }

    #[test]
    fn an_unknown_argument_is_refused() {
        assert!(parse_args(args(&["--nope"])).is_err());
    }
}
