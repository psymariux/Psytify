//! Giro automatico delle viste, per controllare l'interfaccia senza cliccarla (feature `tour`, fuori
//! dalle build normali). Con `PSYTIFY_TOUR=<cartella>` l'app passa per ogni vista, aspetta che i
//! dati arrivino, salva uno screenshot PNG della propria finestra e alla fine si chiude. Va lanciata
//! con una configurazione a parte (`XDG_CONFIG_HOME`) e il volume a zero: suona davvero.

use std::path::PathBuf;
use std::time::Duration;

use iced::{Size, Task, window};

use super::menu::Dialog;
use super::{App, AuthField, AuthMode, Load, Message, Screen, SettingsTab, SideTab};

/// A che punto è il giro.
pub struct Tour {
    dir: PathBuf,
    step: usize,
    /// Con `PSYTIFY_TOUR_STEPS=playing,focus-video` solo quei passi.
    only: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
pub enum TourEvent {
    /// Esegui il passo corrente.
    Step,
    /// La finestra del passo corrente, da salvare.
    Shot(window::Screenshot),
}

/// Un passo: il nome dello screenshot, quanto aspettare prima di scattarlo e cosa fare.
type Step = (&'static str, u64, fn(&mut App) -> Task<Message>);

const STEPS: &[Step] = &[
    ("home", 5000, |app| go(app, [Message::ClosePanel, Message::Navigate(Screen::Home)])),
    ("search", 7000, |app| {
        go(app, [Message::Navigate(Screen::Search), Message::SearchInput("eminem".into())])
    }),
    ("artist", 8000, |app| match app.search.artists.first().map(|artist| artist.id) {
        Some(id) => app.update(Message::OpenMetaArtist(id)),
        None => Task::none(),
    }),
    ("album", 5000, |app| match app.saved_albums.first().map(|album| album.id) {
        Some(id) => app.update(Message::Navigate(Screen::Album(id))),
        None => Task::none(),
    }),
    ("library", 5000, |app| app.update(Message::Navigate(Screen::Library))),
    ("collection", 3000, |app| app.update(Message::Navigate(Screen::Collection))),
    ("liked", 3000, |app| app.update(Message::Navigate(Screen::Liked))),
    ("playlist", 3000, |app| match app.playlists.first().map(|playlist| playlist.id) {
        Some(id) => app.update(Message::Navigate(Screen::Playlist(id))),
        None => Task::none(),
    }),
    ("news", 4000, |app| app.update(Message::Navigate(Screen::News))),
    ("settings-playback", 2000, |app| app.update(Message::OpenSettingsTab(SettingsTab::Playback))),
    ("settings-sound", 2000, |app| app.update(Message::OpenSettingsTab(SettingsTab::Sound))),
    ("settings-account", 2000, |app| app.update(Message::OpenSettingsTab(SettingsTab::Account))),
    ("settings-about", 2000, |app| app.update(Message::OpenSettingsTab(SettingsTab::About))),
    ("jam", 2000, |app| app.update(Message::Navigate(Screen::Jam))),
    ("sign-in", 2000, |app| go(app, [Message::SignIn, Message::AuthMode(AuthMode::SignIn)])),
    ("sign-up", 2000, |app| app.update(Message::AuthMode(AuthMode::SignUp))),
    // Con `PSYTIFY_TOUR_LOGIN=<nome utente>` e `PSYTIFY_TOUR_PASSWORD` di un account di test di
    // Clerk (il codice è sempre 424242) il giro entra davvero e mostra amici e profilo.
    ("sign-in-code", 7000, |app| {
        let (Ok(login), Ok(password)) =
            (std::env::var("PSYTIFY_TOUR_LOGIN"), std::env::var("PSYTIFY_TOUR_PASSWORD"))
        else {
            return Task::none();
        };
        go(
            app,
            [
                Message::AuthMode(AuthMode::SignIn),
                Message::AuthInput(AuthField::Identifier, login),
                Message::AuthInput(AuthField::Password, password),
                Message::AuthSubmit,
            ],
        )
    }),
    ("friends", 8000, |app| {
        let enter = if app.social.form.step.is_some() {
            vec![Message::AuthInput(AuthField::Code, "424242".into()), Message::AuthSubmit]
        } else {
            Vec::new()
        };
        let tasks = go(app, enter);
        Task::batch([tasks, app.update(Message::Navigate(Screen::Friends))])
    }),
    // `PSYTIFY_TOUR_PROFILE` apre il profilo di un altro; senza, il proprio.
    ("profile", 5000, |app| {
        match std::env::var("PSYTIFY_TOUR_PROFILE")
            .ok()
            .or_else(|| app.signed_in_profile().map(|p| p.handle.clone()))
        {
            Some(handle) => app.update(Message::OpenProfile(handle)),
            None => Task::none(),
        }
    }),
    ("friend-playlist", 4000, |app| {
        let Some((handle, Load::Ready(open))) = app.social.open.as_ref() else { return Task::none() };
        match open.page.playlists.first().map(|p| (handle.clone(), p.id)) {
            Some((handle, id)) => app.update(Message::OpenFriendPlaylist(handle, id)),
            None => Task::none(),
        }
    }),
    ("shortcuts", 1500, |app| app.update(Message::ToggleShortcuts)),
    ("search-history", 2000, |app| {
        go(
            app,
            [
                Message::ToggleShortcuts,
                Message::Navigate(Screen::Search),
                Message::SearchInput(String::new()),
            ],
        )
    }),
    ("playing", 10000, |app| {
        // Con una configurazione nuova non ci sono brani che piacciono: suonano gli ultimi arrivati.
        let start = match &app.home.recent {
            _ if !app.queue.is_empty() => Message::TogglePlay,
            _ if !app.liked.is_empty() => Message::ShufflePlay(app.liked.clone().into()),
            Load::Ready(tracks) => Message::PlayTracks(tracks.clone(), 0),
            _ => Message::TogglePlay,
        };
        go(app, [Message::Navigate(Screen::Home), start, Message::OpenPanel(SideTab::NowPlaying)])
    }),
    ("panel-queue", 2500, |app| app.update(Message::SideTab(SideTab::Queue))),
    ("panel-lyrics", 4000, |app| app.update(Message::SideTab(SideTab::Lyrics))),
    // Fermo sul testo del pannello per 40 s: il tempo di campionare CPU e RAM da fuori
    // (`PSYTIFY_TOUR_STEPS=playing,panel-lyrics,hold-lyrics`, vedi docs/PERFORMANCE.md).
    ("hold-lyrics", 40000, |app| {
        if app.panel_open && app.side_tab == SideTab::Lyrics {
            Task::none()
        } else {
            app.update(Message::OpenPanel(SideTab::Lyrics))
        }
    }),
    ("focus-lyrics", 4000, |app| go(app, [Message::SideTab(SideTab::NowPlaying), Message::ToggleFocus])),
    ("focus-video", 6000, |app| app.update(Message::ToggleVideoFocus)),
    ("dialog", 1500, |app| {
        go(
            app,
            [
                Message::Escape,
                Message::OpenDialog(Dialog::NewPlaylist { name: String::new(), seed: Vec::new() }),
            ],
        )
    }),
    ("small-home", 3000, |app| {
        let resize = app.window.map_or_else(Task::none, |id| window::resize(id, Size::new(1100.0, 700.0)));
        Task::batch([go(app, [Message::DialogCancel, Message::Navigate(Screen::Home)]), resize])
    }),
    ("small-artist", 4000, |app| match app.search.artists.first().map(|artist| artist.id) {
        Some(id) => app.update(Message::OpenMetaArtist(id)),
        None => Task::none(),
    }),
];

fn go(app: &mut App, messages: impl IntoIterator<Item = Message>) -> Task<Message> {
    let tasks: Vec<Task<Message>> = messages.into_iter().map(|message| app.update(message)).collect();
    Task::batch(tasks)
}

impl Tour {
    /// Il giro, se `PSYTIFY_TOUR` lo chiede, e il primo passo dopo l'avvio.
    pub fn from_env() -> Option<(Self, Task<Message>)> {
        let dir = PathBuf::from(std::env::var_os("PSYTIFY_TOUR")?);
        std::fs::create_dir_all(&dir).ok()?;
        let start =
            Task::perform(tokio::time::sleep(Duration::from_secs(3)), |()| Message::Tour(TourEvent::Step));
        let only = std::env::var("PSYTIFY_TOUR_STEPS")
            .ok()
            .map(|steps| steps.split(',').map(|step| step.trim().to_owned()).collect());
        Some((Self { dir, step: 0, only }, start))
    }
}

impl App {
    pub(super) fn tour_event(&mut self, event: TourEvent) -> Task<Message> {
        let Some(tour) = &self.tour else { return Task::none() };
        let step = tour.step;
        match event {
            TourEvent::Step => {
                let Some(&(name, wait, act)) = STEPS.get(step) else { return iced::exit() };
                if tour.only.as_ref().is_some_and(|only| !only.iter().any(|wanted| wanted == name)) {
                    if let Some(tour) = self.tour.as_mut() {
                        tour.step += 1;
                    }
                    return Task::done(Message::Tour(TourEvent::Step));
                }
                let acted = act(self);
                let window = self.window;
                let shoot = Task::perform(tokio::time::sleep(Duration::from_millis(wait)), |()| ())
                    .then(move |()| window.map_or_else(Task::none, window::screenshot))
                    .map(|shot| Message::Tour(TourEvent::Shot(shot)));
                Task::batch([acted, shoot])
            }
            TourEvent::Shot(shot) => {
                let name = STEPS.get(step).map_or("extra", |&(name, _, _)| name);
                let path = tour.dir.join(format!("{step:02}-{name}.png"));
                let saved =
                    image::RgbaImage::from_raw(shot.size.width, shot.size.height, shot.bytes.to_vec())
                        .map(|frame| frame.save(&path));
                if !matches!(saved, Some(Ok(()))) {
                    eprintln!("tour: cannot save {}", path.display());
                }
                if let Some(tour) = self.tour.as_mut() {
                    tour.step += 1;
                }
                Task::done(Message::Tour(TourEvent::Step))
            }
        }
    }
}
