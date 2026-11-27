//! Galleria dei componenti: ogni componente in ogni stato (forzato con `Preview`, così si vedono
//! affiancati senza tenere premuto il mouse), la lista da 10 000 righe e tutti gli sprite alla scala
//! corrente. Si apre con `psytify --gallery` (oggi anche senza argomenti).

mod sections;

use std::time::Duration;

use gpui::{
    AppContext, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Point, Render, StatefulInteractiveElement, Styled, Task, Window, deferred, div, px,
    uniform_list,
};
use psytify_design::tokens::color::SURFACE_CANVAS;
use psytify_design::tokens::motion::{PROGRESS_TICK_MS, TOAST_HOLD_MS};
use psytify_design::tokens::space::{SPACE_5, SPACE_6};

use crate::actions::{FocusNext, FocusPrev, FocusSearch, Quit};
use crate::components::menu::popup;
use crate::components::overlay::{Dialog, Toast, scrim};
use crate::components::search_field::SearchField;
use crate::components::track_row::{TrackRow, sample};
use crate::components::{Button, Preview};
use crate::theme::{col, dp};
use crate::titlebar::{TitleBarState, title_bar, window_frame};

/// Durata del brano di prova in secondi (il seek simula un brano da 3:30).
pub const DEMO_SECONDS: f32 = 210.0;

pub struct Gallery {
    focus: FocusHandle,
    search: gpui::Entity<SearchField>,
    search_live: gpui::Entity<SearchField>,
    search_text: gpui::Entity<SearchField>,
    search_loading: gpui::Entity<SearchField>,
    search_disabled: gpui::Entity<SearchField>,
    /// I campi da 40 dp dei moduli: uno in chiaro, uno mascherato pieno e uno vuoto.
    form_email: gpui::Entity<SearchField>,
    form_password: gpui::Entity<SearchField>,
    form_code: gpui::Entity<SearchField>,
    playing: bool,
    position: f32,
    volume: f32,
    toggles: [bool; 3],
    /// La scelta del Select di prova e le bande del banco di slider.
    choice: usize,
    bands: [f32; 10],
    tab: usize,
    nav: usize,
    selected_row: Option<usize>,
    playing_row: Option<usize>,
    menu: Option<Point<gpui::Pixels>>,
    /// La prima copertina dello scaffale di prova (le frecce e la rotella la spostano).
    shelf_start: usize,
    shelf_wheel: crate::components::shelf::Wheel,
    dialog: bool,
    toast: Option<gpui::SharedString>,
    toast_task: Option<Task<()>>,
    tick_task: Option<Task<()>>,
}

impl Gallery {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| SearchField::new("title-search", "Cerca brani, album, artisti", cx));
        let search_live = cx.new(|cx| SearchField::new("search-live", "Cerca brani, album, artisti", cx));
        let search_text = cx.new(|cx| SearchField::new("search-text", "Cerca", cx).with_text("Neon Harbour"));
        let search_loading = cx.new(|cx| SearchField::new("search-load", "Cerca", cx).loading(true));
        let search_disabled = cx.new(|cx| SearchField::new("search-dis", "Cerca", cx).disabled(true));
        let form_email = cx.new(|cx| {
            SearchField::new("form-email", "Email", cx).plain().large().with_text("luca@example.com")
        });
        // Un testo di prova (non è una password di nessuno): mostra i pallini.
        let form_password = cx.new(|cx| {
            SearchField::new("form-password", "Password", cx).masked().large().with_text("prova-123")
        });
        let form_code = cx.new(|cx| SearchField::new("form-code", "Codice di 6 cifre", cx).masked().large());
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        Self {
            form_email,
            form_password,
            form_code,
            focus,
            search,
            search_live,
            search_text,
            search_loading,
            search_disabled,
            playing: false,
            position: 0.18,
            volume: 0.6,
            toggles: [true, false, true],
            choice: 1,
            bands: [0.75, 0.7, 0.62, 0.55, 0.5, 0.5, 0.45, 0.55, 0.6, 0.5],
            tab: 0,
            nav: 0,
            selected_row: None,
            playing_row: Some(3),
            menu: None,
            shelf_start: 0,
            shelf_wheel: crate::components::shelf::Wheel::default(),
            dialog: false,
            toast: None,
            toast_task: None,
            tick_task: None,
        }
    }

    /// Avvia o ferma il conteggio del brano di prova: un timer da `PROGRESS_TICK_MS`, attivo solo
    /// mentre «suona». Da fermo non esiste nessun timer, quindi nessun lavoro a riposo.
    fn set_playing(&mut self, playing: bool, cx: &mut Context<Self>) {
        self.playing = playing;
        self.tick_task = None;
        if playing {
            let step = PROGRESS_TICK_MS as f32 / 1000.0 / DEMO_SECONDS;
            self.tick_task = Some(cx.spawn(async move |this, cx| {
                loop {
                    cx.background_executor().timer(Duration::from_millis(u64::from(PROGRESS_TICK_MS))).await;
                    let alive = this
                        .update(cx, |this, cx| {
                            this.position = (this.position + step) % 1.0;
                            cx.notify();
                        })
                        .is_ok();
                    if !alive {
                        break;
                    }
                }
            }));
        }
        cx.notify();
    }

    fn show_toast(&mut self, text: &str, cx: &mut Context<Self>) {
        self.toast = Some(text.to_owned().into());
        self.toast_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_millis(u64::from(TOAST_HOLD_MS))).await;
            let _ = this.update(cx, |this, cx| {
                this.toast = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" && (self.dialog || self.menu.is_some()) {
            self.dialog = false;
            self.menu = None;
            cx.notify();
        }
    }
}

impl Focusable for Gallery {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for Gallery {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let bar = title_bar(
            TitleBarState {
                search: self.search.clone(),
                playing: self.playing,
                history: crate::titlebar::HistoryButtons::none(),
                account: crate::titlebar::AccountChip::SignedIn { avatar: None, name: "Psy".into() },
                on_account: None,
                on_close: std::rc::Rc::new(|_, cx| cx.quit()),
            },
            window,
            cx,
        );
        let body = sections::body(self, window, cx);
        let list = div().h(dp(480.0)).w_full().child(
            uniform_list(
                "tracks",
                10_000,
                cx.processor(|this: &mut Self, range: std::ops::Range<usize>, _w, cx| {
                    range
                        .map(|ix| {
                            let mut row = TrackRow::new(("track", ix), sample(ix));
                            if this.playing_row == Some(ix) {
                                row = row.playing(if this.playing {
                                    crate::components::indicators::PlayState::Playing
                                } else {
                                    crate::components::indicators::PlayState::Paused
                                });
                            }
                            row.selected(this.selected_row == Some(ix)).on_click(cx.listener(
                                move |this, event: &gpui::ClickEvent, _, cx| {
                                    if event.click_count() >= 2 {
                                        this.playing_row = Some(ix);
                                        this.set_playing(true, cx);
                                    } else {
                                        this.selected_row = Some(ix);
                                        cx.notify();
                                    }
                                },
                            ))
                        })
                        .collect::<Vec<_>>()
                }),
            )
            .size_full(),
        );
        let content = div()
            .id("gallery-scroll")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(dp(SPACE_6))
            .py(dp(SPACE_6))
            .child(body)
            .child(sections::section(
                "TrackRow · uniform_list da 10 000 righe (doppio clic = riproduci)",
                list,
            ));

        let mut root = div()
            .id("root")
            .size_full()
            .flex()
            .flex_col()
            .bg(col(SURFACE_CANVAS))
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(|_, _: &FocusNext, window, cx| window.focus_next(cx)))
            .on_action(cx.listener(|_, _: &FocusPrev, window, cx| window.focus_prev(cx)))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                let handle = this.search.read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }))
            .on_action(cx.listener(|_, _: &Quit, _, cx| cx.quit()))
            .child(bar)
            .child(content);

        if let Some(at) = self.menu {
            root = root.child(deferred(
                div()
                    .absolute()
                    .size_full()
                    .top(px(0.0))
                    .left(px(0.0))
                    .occlude()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.menu = None;
                            cx.notify();
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _, _, cx| {
                            this.menu = None;
                            cx.notify();
                        }),
                    ),
            ));
            root = root.child(popup(at, sections::context_menu(cx)));
        }
        if self.dialog {
            let close = cx.listener(|this: &mut Self, _: &gpui::ClickEvent, _, cx| {
                this.dialog = false;
                cx.notify();
            });
            let close2 = cx.listener(|this: &mut Self, _: &gpui::ClickEvent, _, cx| {
                this.dialog = false;
                cx.notify();
            });
            root = root.child(deferred(
                div()
                    .absolute()
                    .top(px(0.0))
                    .left(px(0.0))
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(scrim().on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            this.dialog = false;
                            cx.notify();
                        }),
                    ))
                    .child(
                        div().relative().occlude().child(
                            Dialog::new(
                                "Eliminare la playlist?",
                                "La playlist «Notte» sparirà da questo dispositivo.",
                            )
                            .button(Button::new("dlg-cancel", "Annulla").secondary().on_click(close))
                            .button(Button::new("dlg-ok", "Elimina").destructive().on_click(close2)),
                        ),
                    ),
            ));
        }
        if let Some(text) = self.toast.clone() {
            root = root.child(
                div()
                    .absolute()
                    .bottom(dp(SPACE_5))
                    .left(px(0.0))
                    .w_full()
                    .flex()
                    .justify_center()
                    .child(Toast::new(text)),
            );
        }
        let _ = Preview::Live;
        let _ = cx.entity();
        window_frame(root)
    }
}
