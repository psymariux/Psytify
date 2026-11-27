//! I messaggi della sola interfaccia, smistati da `App::handle`: navigazione, pannelli, menu,
//! finestre di dialogo, finestra, animazioni e avvisi. Restano nel frontend.

use std::time::Instant;

use iced::widget::text_input;
use iced::{Task, window};
use psytify_core::Core;

use super::{
    App, DIALOG_INPUT, Dialog, Message, Notice, OpenDialog, SEARCH_INPUT, Screen, SidebarFilter, is_phone,
};
use crate::ui::motion::Tween;
use crate::ui::theme as tokens;

impl App {
    /// I messaggi di questo dominio, smistati da `App::handle`.
    pub(super) fn handle_interface(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Navigate(screen) => {
                // Sul telefono, Cerca toccata di nuovo riapre la tastiera chiusa con Indietro.
                if is_phone() && screen == Screen::Search && self.screen == Screen::Search {
                    crate::soft_keyboard(true);
                }
                let task = self.open(screen);
                Task::batch([task, self.persist()])
            }
            Message::ModifiersChanged(modifiers) => {
                self.modifiers = modifiers;
                Task::none()
            }
            Message::GoBack => self.go_back(),
            Message::GoForward => self.go_forward(),
            Message::ToggleShortcuts => {
                self.shortcuts = match self.shortcuts {
                    Some(_) => None,
                    None => Some(Instant::now()),
                };
                Task::none()
            }
            Message::Frame => {
                self.advance_splash();
                // L'avviso è uscito del tutto: si toglie.
                if self.notice.is_none()
                    && self.toast.as_ref().is_some_and(|toast| !toast.motion.is_running())
                {
                    self.toast = None;
                }
                Task::none()
            }
            Message::SkipSplash => {
                self.splash = None;
                self.page_motion = Tween::new(0.0, 1.0, tokens::MOTION_BASE);
                Task::none()
            }
            Message::NoticeExpired(generation) => {
                if self.toast.as_ref().is_some_and(|toast| toast.generation == generation) {
                    self.notice = None;
                }
                Task::none()
            }
            Message::OpenPanel(tab) => {
                let on_it = self.panel_open && self.side_tab == tab;
                self.panel_open = !on_it;
                self.side_tab = tab;
                self.persist()
            }
            Message::ClosePanel => {
                self.panel_open = false;
                self.persist()
            }
            Message::SettingsTab(tab) => {
                self.settings_tab = tab;
                Task::none()
            }
            Message::SideTab(tab) => {
                self.side_tab = tab;
                Task::none()
            }
            Message::SidebarFilter(filter) => {
                // Un secondo clic sul chip acceso lo spegne: si torna a «Tutto».
                self.sidebar_filter = if self.sidebar_filter == filter { SidebarFilter::All } else { filter };
                Task::none()
            }
            Message::HeroGo(index) => {
                self.hero_go(index);
                Task::none()
            }
            Message::HeroNext => {
                self.hero_next();
                Task::none()
            }
            Message::ToggleFocus => {
                // Il testo a tutto schermo e il videoclip a tutto schermo sono due viste: il tasto
                // del testo da una vista aperta la chiude, da quella del video passa al testo.
                let open = !self.focus && self.queue.current().is_some();
                self.focus_video = false;
                self.focus = open;
                Task::none()
            }
            Message::OpenSettingsTab(tab) => {
                self.settings_tab = tab;
                let open = self.open(Screen::Settings);
                Task::batch([open, self.persist()])
            }
            Message::CopyText(text) => {
                self.notice = Some(Notice::Info("Copiato negli appunti".into()));
                iced::clipboard::write(text)
            }
            Message::OpenMenu(target) => self.open_menu(target),
            Message::CloseMenu => {
                self.menu = None;
                Task::none()
            }
            Message::MenuPlaylists(open) => {
                if let Some(menu) = &mut self.menu {
                    menu.playlists = open;
                    menu.downloads &= !open;
                    menu.timers &= !open;
                }
                Task::none()
            }
            Message::MenuDownloads(open) => {
                if let Some(menu) = &mut self.menu {
                    menu.downloads = open;
                    menu.playlists &= !open;
                    menu.timers &= !open;
                }
                Task::none()
            }
            Message::OpenDrawer => {
                self.drawer = Some(Instant::now());
                Task::none()
            }
            Message::CloseDrawer => {
                self.drawer = None;
                Task::none()
            }
            Message::DrawerGo(message) => {
                self.drawer = None;
                self.handle(*message)
            }
            Message::MenuTimers(open) => {
                if let Some(menu) = &mut self.menu {
                    menu.timers = open;
                    menu.playlists &= !open;
                    menu.downloads &= !open;
                }
                Task::none()
            }
            Message::OpenDialog(dialog) => {
                self.menu = None;
                let named = matches!(dialog, Dialog::NewPlaylist { .. } | Dialog::Rename { .. });
                self.dialog = Some(OpenDialog { dialog, since: Instant::now() });
                if named { text_input::focus(text_input::Id::new(DIALOG_INPUT)) } else { Task::none() }
            }
            Message::DialogInput(text) => {
                self.dialog_input(text);
                Task::none()
            }
            Message::DialogCancel => {
                self.dialog = None;
                Task::none()
            }
            Message::Chosen(message) => {
                self.menu = None;
                self.handle(*message)
            }
            Message::Escape => self.escape(),
            Message::SystemBack => {
                if self.menu.is_some()
                    || self.drawer.is_some()
                    || self.dialog.is_some()
                    || self.shortcuts.is_some()
                    || self.focus
                {
                    self.escape()
                } else {
                    self.go_back()
                }
            }
            Message::WindowReady(id) => {
                self.window = id;
                match id {
                    Some(id) => window::get_size(id).map(Message::WindowResized),
                    None => Task::none(),
                }
            }
            Message::WindowResized(size) => {
                self.window_size = size;
                self.insets = crate::system_insets().or(self.insets);
                Task::none()
            }
            Message::TitleBarPressed => self.title_bar_pressed(),
            Message::Minimize => self.minimize(),
            Message::ToggleFullscreen => self.toggle_fullscreen(),
            Message::Quit => self.quit(),
            Message::ResizeStart(edge) => {
                self.resize_started(edge);
                Task::none()
            }
            Message::ResizeMove(at) => self.resize_moved(at),
            Message::ResizeEnd => {
                self.resizing = None;
                Task::none()
            }
            Message::DismissNotice => {
                self.notice = None;
                Task::none()
            }
            Message::FocusSearch => text_input::focus(text_input::Id::new(SEARCH_INPUT)),
            Message::CloseRequested(id) => {
                // L'ultimo stato al core, che lo scrive prima di fermarsi: aspetta la scrittura in
                // corso (mai due insieme) e poi scrive ciò che manca. Il core gira sul runtime di
                // Iced: si esce solo dopo che si è fermato.
                let _ = self.persist();
                let stopped = self.core.take().map(Core::shutdown);
                Task::perform(
                    async move {
                        if let Some(stopped) = stopped
                            && let Ok(Err(err)) = stopped.await
                        {
                            // Si esce lo stesso: il file resta quello di prima, intero.
                            eprintln!("psytify: {err}");
                        }
                    },
                    move |()| Message::Exit(id),
                )
            }
            Message::Exit(id) => window::close(id),
            #[cfg(feature = "tour")]
            Message::Tour(event) => self.tour_event(event),
            // `handle` manda qui solo i messaggi di questo dominio.
            _ => Task::none(),
        }
    }
}
