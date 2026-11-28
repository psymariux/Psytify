//! «Accedi a Psytify» (`Page::SignIn`), as in `PSYTIFY_ART_DIRECTION.md` («Sign-in / social») and the
//! The column of `SIGN_IN_W` (360 dp) on the canvas, Nastro
//! (`nastro_idle`) above the wordmark and a line that changes with the step; the tabs Accedi / Crea un account;
//! the 40 dp fields; the GitHub, Google and Discord buttons (secondary, with the monochrome mark of the
//! provider at 20 dp, the only vector exception); the errors in `TEXT_ERROR` under the button, never in a
//! notice. The email code step (`AuthView::step`) and the wait for the browser (`AuthView::waiting`, with
//! «Annulla» → `CancelSignIn`) take the place of the fields. No lime: the art direction does not want it
//! on sign-in.
//!
//! The password and the code stay only in their fields (masked): on submit the text is taken and the field
//! is emptied at the same moment (`SearchField::take`), then it goes to the core wrapped in `Secret`. None of it
//! passes through the snapshot, the `Debug`s, the logs or the notices. Enter sends the form, Tab moves to the
//! next field. Typing again removes the error shown (`DismissError`).
//!
//! The page copies from the social view only what it shows (`Mirror`), when the core publishes; `render`
//! does not read the snapshot.

use std::sync::Arc;

use gpui::{
    App, AppContext, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription,
    WeakEntity, Window, div,
};
use psytify_core::auth::{Provider, Secret};
use psytify_core::command::SocialCmd;
use psytify_core::snapshot::{Account, SocialView};
use psytify_design::brand;
use psytify_design::sprite;
use psytify_design::tokens::color::{
    SURFACE_CANVAS, TEXT_ERROR, TEXT_PRIMARY, TEXT_SECONDARY, TEXT_TERTIARY,
};
use psytify_design::tokens::size::SIGN_IN_W;
use psytify_design::tokens::space::{SPACE_3, SPACE_5, SPACE_6, SPACE_9};
use psytify_design::tokens::type_::{TYPE_BODY, TYPE_META};

use super::PageEvent;
use super::page_frame::PageFrame;
use super::people::{TEXT_MAX, code_chars, handle_chars, line_chars};
use crate::components::button::Button;
use crate::components::navigation::{Tab, Tabs};
use crate::components::search_field::SearchField;
use crate::components::separator::Separator;
use crate::core_model::{Changed, CoreModel};
use crate::pixel::PixelSprite;
use crate::theme::{TypeStyle, col, dp};

/// The form's tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    SignIn,
    SignUp,
}

/// The sign-in step, from the core's view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    /// The fields of the tab.
    #[default]
    Form,
    /// The code sent to this address (already masked by the core).
    Code(SharedString),
    /// Waiting for the return from this provider's browser.
    Waiting(Provider),
}

/// What the page shows of the social view.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Mirror {
    stage: Stage,
    busy: bool,
    error: Option<SharedString>,
    /// Sign-in is configured.
    available: bool,
}

impl Mirror {
    fn of(view: &SocialView) -> Self {
        let auth = &view.auth;
        let stage = match (&auth.waiting, &auth.step) {
            (Some(provider), _) => Stage::Waiting(*provider),
            (None, Some(hint)) => Stage::Code(hint.sent_to.clone().into()),
            (None, None) => Stage::Form,
        };
        Self {
            stage,
            busy: auth.busy,
            error: auth.error.as_ref().map(|err| err.to_string().into()),
            available: view.account != Account::Unavailable,
        }
    }
}

/// What is in the fields: the plain texts and, of the masked fields, only how many characters they have.
#[derive(Clone, Copy, Debug, Default)]
struct Typed<'a> {
    identifier: &'a str,
    email: &'a str,
    username: &'a str,
    password_chars: usize,
    code_chars: usize,
}

/// The form can be sent (the rules of `SignInForm::can_submit`): for sign-in a username and
/// a password, for sign-up an email and a password of at least 8 characters, for the code 6 digits;
/// never while a submit is in progress or the browser is awaited.
fn can_submit(mode: Mode, stage: &Stage, busy: bool, typed: &Typed) -> bool {
    if busy {
        return false;
    }
    match stage {
        Stage::Waiting(_) => false,
        Stage::Code(_) => typed.code_chars >= 6,
        Stage::Form => match mode {
            Mode::SignIn => !typed.identifier.trim().is_empty() && typed.password_chars > 0,
            Mode::SignUp => typed.email.contains('@') && typed.password_chars >= 8,
        },
    }
}

/// The masked field that submit empties.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MaskedField {
    Password,
    NewPassword,
    Code,
}

fn secret_field(mode: Mode, stage: &Stage) -> Option<MaskedField> {
    match (stage, mode) {
        (Stage::Waiting(_), _) => None,
        (Stage::Code(_), _) => Some(MaskedField::Code),
        (Stage::Form, Mode::SignIn) => Some(MaskedField::Password),
        (Stage::Form, Mode::SignUp) => Some(MaskedField::NewPassword),
    }
}

/// The submit command, with the masked field's text already taken (and at once wrapped in `Secret`).
fn command(mode: Mode, stage: &Stage, typed: &Typed, secret: String) -> Option<SocialCmd> {
    let secret = Secret::new(secret);
    Some(match (stage, mode) {
        (Stage::Waiting(_), _) => return None,
        (Stage::Code(_), _) => SocialCmd::SubmitCode(secret),
        (Stage::Form, Mode::SignIn) => {
            SocialCmd::SignIn { identifier: typed.identifier.trim().to_owned(), password: secret }
        }
        (Stage::Form, Mode::SignUp) => SocialCmd::SignUp {
            email: typed.email.trim().to_owned(),
            username: typed.username.to_owned(),
            password: secret,
        },
    })
}

/// The line under the wordmark, which changes with the step.
fn line(mode: Mode, stage: &Stage) -> &'static str {
    match (stage, mode) {
        (Stage::Code(_), _) => "Controlla la posta",
        (Stage::Waiting(_), _) => "Ti aspettiamo nel browser",
        (Stage::Form, Mode::SignIn) => "Accedi per ascoltare con i tuoi amici",
        (Stage::Form, Mode::SignUp) => "Crea il tuo account",
    }
}

/// A provider's mark.
fn mark(provider: Provider) -> &'static [u8] {
    match provider {
        Provider::GitHub => brand::GITHUB,
        Provider::Google => brand::GOOGLE,
        Provider::Discord => brand::DISCORD,
    }
}

/// The form's fields.
struct Fields {
    identifier: Entity<SearchField>,
    password: Entity<SearchField>,
    email: Entity<SearchField>,
    username: Entity<SearchField>,
    new_password: Entity<SearchField>,
    code: Entity<SearchField>,
}

pub struct SignInPage {
    core: Entity<CoreModel>,
    mode: Mode,
    mirror: Mirror,
    fields: Fields,
    /// The main button is lit (recomputed on every key; the page redraws only if it changes).
    ready: bool,
    /// At the next chance focus goes to the step's first field (the page has opened, the step has
    /// changed).
    focus_first: bool,
    focus: FocusHandle,
    /// Only the enter: the 360 dp column stays centred on any width.
    frame: PageFrame,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for SignInPage {}

/// A form field: 40 dp, no icons; every key and Enter return to the page after the field has
/// finished its update (`App::defer`: the field is still borrowed while it calls).
fn field(
    page: &WeakEntity<SignInPage>,
    id: &'static str,
    placeholder: &'static str,
    cx: &mut Context<SignInPage>,
    shape: impl FnOnce(SearchField) -> SearchField,
) -> Entity<SearchField> {
    let (edited, submitted) = (page.clone(), page.clone());
    cx.new(|cx| {
        shape(SearchField::new(id, placeholder, cx).plain().large())
            .on_change(move |_, _, cx| {
                let page = edited.clone();
                cx.defer(move |cx| {
                    page.update(cx, |page, cx| page.edited(cx)).ok();
                });
            })
            .on_submit(move |_, _, cx| {
                let page = submitted.clone();
                cx.defer(move |cx| {
                    page.update(cx, |page, cx| page.submit(cx)).ok();
                });
            })
    })
}

impl SignInPage {
    pub fn new(core: Entity<CoreModel>, cx: &mut Context<Self>) -> Self {
        let page = cx.entity().downgrade();
        let fields = Fields {
            identifier: field(&page, "signin-identifier", "Email o nome utente", cx, |f| {
                f.sanitize(line_chars).max_chars(TEXT_MAX)
            }),
            password: field(&page, "signin-password", "Password", cx, |f| f.masked().max_chars(TEXT_MAX)),
            email: field(&page, "signup-email", "Email", cx, |f| f.sanitize(line_chars).max_chars(TEXT_MAX)),
            username: field(&page, "signup-username", "Nome utente (facoltativo)", cx, |f| {
                f.sanitize(handle_chars).max_chars(super::people::HANDLE_MAX)
            }),
            new_password: field(&page, "signup-password", "Password, almeno 8 caratteri", cx, |f| {
                f.masked().max_chars(TEXT_MAX)
            }),
            code: field(&page, "signin-code", "Codice di 6 cifre", cx, |f| {
                f.masked().sanitize(code_chars).max_chars(6)
            }),
        };
        let changes = cx.subscribe(&core, |this, core, changed: &Changed, cx| {
            if changed.social {
                let view = core.read(cx).social_view();
                this.take(&view, cx);
            }
        });
        let view = core.read(cx).social_view();
        Self {
            mirror: Mirror::of(&view),
            core,
            mode: Mode::SignIn,
            fields,
            ready: false,
            focus_first: true,
            frame: PageFrame::default(),
            focus: cx.focus_handle(),
            _changes: changes,
        }
    }

    /// The page opens: focus will go to the first field.
    pub fn show(&mut self, cx: &mut Context<Self>) {
        self.focus_first = true;
        cx.notify();
    }

    /// The page comes into view (`Shell::page_shown`).
    pub fn enter(&mut self, rise: bool, cx: &mut Context<Self>) {
        self.frame.enter(rise, cx);
        cx.notify();
    }

    fn take(&mut self, view: &Arc<SocialView>, cx: &mut Context<Self>) {
        let mirror = Mirror::of(view);
        if mirror == self.mirror {
            return;
        }
        if mirror.stage != self.mirror.stage {
            // A new step starts with its empty field and with focus.
            self.fields.code.update(cx, |field, cx| field.reset(cx));
            self.focus_first = true;
        }
        self.mirror = mirror;
        self.ready = self.compute_ready(cx);
        cx.notify();
    }

    fn typed<'a>(&self, cx: &'a App) -> Typed<'a> {
        Typed {
            identifier: self.fields.identifier.read(cx).text(),
            email: self.fields.email.read(cx).text(),
            username: self.fields.username.read(cx).text(),
            password_chars: match self.mode {
                Mode::SignIn => self.fields.password.read(cx).char_count(),
                Mode::SignUp => self.fields.new_password.read(cx).char_count(),
            },
            code_chars: self.fields.code.read(cx).char_count(),
        }
    }

    fn compute_ready(&self, cx: &App) -> bool {
        can_submit(self.mode, &self.mirror.stage, self.mirror.busy, &self.typed(cx))
    }

    /// A field changed: the error shown goes away, the button lights up or goes off.
    fn edited(&mut self, cx: &mut Context<Self>) {
        if self.mirror.error.is_some() {
            self.core.read(cx).social(SocialCmd::DismissError);
        }
        let ready = self.compute_ready(cx);
        if ready != self.ready {
            self.ready = ready;
            cx.notify();
        }
    }

    /// Enter or the main button: takes the masked text (the field empties at once) and sends it.
    fn submit(&mut self, cx: &mut Context<Self>) {
        if !self.compute_ready(cx) {
            return;
        }
        let Some(which) = secret_field(self.mode, &self.mirror.stage) else { return };
        let field = match which {
            MaskedField::Password => &self.fields.password,
            MaskedField::NewPassword => &self.fields.new_password,
            MaskedField::Code => &self.fields.code,
        };
        let secret = field.update(cx, |field, cx| field.take(cx));
        let typed = self.typed(cx);
        if let Some(command) = command(self.mode, &self.mirror.stage, &typed, secret) {
            self.core.read(cx).social(command);
        }
        self.ready = false;
        cx.notify();
    }

    /// Changes the tab: the passwords go away, like an email code step left half done (`CancelSignIn`).
    fn select(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if mode == self.mode {
            return;
        }
        self.mode = mode;
        self.fields.password.update(cx, |field, cx| field.reset(cx));
        self.fields.new_password.update(cx, |field, cx| field.reset(cx));
        if self.mirror.error.is_some() || self.mirror.stage != Stage::Form {
            self.core.read(cx).social(SocialCmd::CancelSignIn);
        }
        self.focus_first = true;
        self.ready = self.compute_ready(cx);
        cx.notify();
    }

    fn first_field(&self) -> &Entity<SearchField> {
        match (&self.mirror.stage, self.mode) {
            (Stage::Code(_), _) => &self.fields.code,
            (_, Mode::SignIn) => &self.fields.identifier,
            (_, Mode::SignUp) => &self.fields.email,
        }
    }

    fn primary(&self, label: &'static str, cx: &mut Context<Self>) -> Button {
        let label = if self.mirror.busy { "Un momento…" } else { label };
        Button::new("signin-submit", label)
            .primary()
            .large()
            .width(SIGN_IN_W)
            .loading(self.mirror.busy)
            .disabled(!self.ready && !self.mirror.busy)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.submit(cx)))
    }

    fn error(&self) -> Option<gpui::Div> {
        self.mirror
            .error
            .clone()
            .map(|error| div().type_role(TYPE_BODY).text_color(col(TEXT_ERROR)).child(error))
    }

    fn form(&self, cx: &mut Context<Self>) -> gpui::Div {
        let selected = usize::from(self.mode == Mode::SignUp);
        let tabs = Tabs::new(
            vec![Tab::new("signin-tab-in", "Accedi"), Tab::new("signin-tab-up", "Crea un account")],
            selected,
        )
        .on_select({
            let page = cx.entity().downgrade();
            move |ix, _, cx| {
                let mode = if ix == 0 { Mode::SignIn } else { Mode::SignUp };
                page.update(cx, |page, cx| page.select(mode, cx)).ok();
            }
        });
        let (fields, label): (Vec<Entity<SearchField>>, _) = match self.mode {
            Mode::SignIn => (vec![self.fields.identifier.clone(), self.fields.password.clone()], "Accedi"),
            Mode::SignUp => (
                vec![
                    self.fields.email.clone(),
                    self.fields.username.clone(),
                    self.fields.new_password.clone(),
                ],
                "Crea l'account",
            ),
        };
        let busy = self.mirror.busy;
        let providers = Provider::ALL.iter().copied().enumerate().map(|(ix, provider)| {
            Button::new(("signin-provider", ix), format!("Continua con {}", provider.label()))
                .secondary()
                .large()
                .width(SIGN_IN_W)
                .mark(mark(provider))
                .disabled(busy)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.core.read(cx).social(SocialCmd::SignInWith(provider));
                }))
        });
        let providers: Vec<Button> = providers.collect();
        let mut column = div()
            .flex()
            .flex_col()
            .gap(dp(SPACE_5))
            .child(tabs)
            .child(div().flex().flex_col().gap(dp(SPACE_3)).children(fields))
            .child(self.primary(label, cx))
            .children(self.error());
        if self.mode == Mode::SignUp {
            column = column.child(
                div()
                    .type_role(TYPE_META)
                    .text_color(col(TEXT_TERTIARY))
                    .child("Ti mandiamo un codice per confermare l'email."),
            );
        }
        column
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(dp(SPACE_3))
                    .child(div().flex_1().child(Separator::horizontal()))
                    .child(div().type_role(TYPE_META).text_color(col(TEXT_TERTIARY)).child("oppure"))
                    .child(div().flex_1().child(Separator::horizontal())),
            )
            .child(div().flex().flex_col().gap(dp(SPACE_3)).children(providers))
    }

    fn code(&self, sent_to: &SharedString, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap(dp(SPACE_5))
            .child(
                div()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_SECONDARY))
                    .child(format!("Abbiamo mandato un codice a {sent_to}. Scrivilo qui sotto.")),
            )
            .child(self.fields.code.clone())
            .child(self.primary("Conferma", cx))
            .children(self.error())
            .child(
                div()
                    .flex()
                    .gap(dp(SPACE_3))
                    .child(
                        Button::new("signin-resend", "Invia di nuovo")
                            .secondary()
                            .disabled(self.mirror.busy)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                this.core.read(cx).social(SocialCmd::ResendCode);
                            })),
                    )
                    .child(Button::new("signin-back", "Indietro").ghost().on_click(cx.listener(
                        |this, _: &ClickEvent, _, cx| this.core.read(cx).social(SocialCmd::CancelSignIn),
                    ))),
            )
    }

    fn waiting(&self, provider: Provider, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_col()
            .gap(dp(SPACE_5))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(dp(SPACE_3))
                    .child(div().flex_none().child(PixelSprite::new(sprite::LOADER)))
                    .child(
                        div().flex_1().min_w_0().type_role(TYPE_BODY).text_color(col(TEXT_PRIMARY)).child(
                            format!(
                                "Completa l'accesso con {} nel browser: l'app ti aspetta qui.",
                                provider.label()
                            ),
                        ),
                    ),
            )
            .child(
                div()
                    .type_role(TYPE_BODY)
                    .text_color(col(TEXT_SECONDARY))
                    .child("Hai chiuso il browser? Annulla qui sotto, o torna fra qualche minuto."),
            )
            .child(Button::new("signin-cancel", "Annulla").secondary().large().width(SIGN_IN_W).on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| this.core.read(cx).social(SocialCmd::CancelSignIn)),
            ))
            .children(self.error())
    }
}

impl Focusable for SignInPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Render for SignInPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.frame.begin(window, None);
        if self.focus_first {
            // Focus on the step's first field, when the page opens or the step changes (while waiting
            // for the browser there is no field).
            self.focus_first = false;
            if self.mirror.available && !matches!(self.mirror.stage, Stage::Waiting(_)) {
                let handle = self.first_field().read(cx).focus_handle(cx);
                window.focus(&handle, cx);
            }
        }
        let body = if !self.mirror.available {
            div()
                .type_role(TYPE_BODY)
                .text_color(col(TEXT_SECONDARY))
                .child("L'accesso non è disponibile: il login non è configurato su questo computer.")
        } else {
            match self.mirror.stage.clone() {
                Stage::Form => self.form(cx),
                Stage::Code(sent_to) => self.code(&sent_to, cx),
                Stage::Waiting(provider) => self.waiting(provider, cx),
            }
        };
        let column = div()
            .w(dp(SIGN_IN_W))
            .flex_none()
            .flex()
            .flex_col()
            .gap(dp(SPACE_6))
            .pt(dp(SPACE_9))
            .pb(dp(SPACE_9))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .gap(dp(SPACE_3))
                    .child(PixelSprite::new(sprite::NASTRO_CHEER))
                    .child(div().pixel_type(false).text_color(col(TEXT_PRIMARY)).child("Psytify"))
                    .child(
                        div()
                            .type_role(TYPE_BODY)
                            .text_color(col(TEXT_SECONDARY))
                            .child(line(self.mode, &self.mirror.stage)),
                    ),
            )
            .child(body)
            .child(
                div()
                    .type_role(TYPE_META)
                    .text_color(col(TEXT_TERTIARY))
                    .text_center()
                    .child("Catalogo e ascolto restano liberi anche senza account."),
            );
        div()
            .id("signin-page")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .bg(col(SURFACE_CANVAS))
            .flex()
            .flex_col()
            .items_center()
            .child(self.frame.wrap(column.into_any_element(), 0, window))
    }
}

#[cfg(test)]
mod tests {
    use psytify_core::error::AppError;
    use psytify_core::snapshot::{AuthView, CodeHint};

    use super::*;

    /// A new value on every run, so no password is written in the source.
    fn secret() -> String {
        uuid::Uuid::new_v4().simple().to_string()
    }

    fn typed(
        identifier: &'static str,
        email: &'static str,
        password_len: usize,
        code_len: usize,
    ) -> Typed<'static> {
        Typed { identifier, email, username: "", password_chars: password_len, code_chars: code_len }
    }

    #[test]
    fn the_form_opens_only_with_what_each_step_needs() {
        let form = Stage::Form;
        assert!(can_submit(Mode::SignIn, &form, false, &typed("luca", "", 1, 0)));
        assert!(!can_submit(Mode::SignIn, &form, false, &typed("  ", "", 5, 0)), "serve un nome");
        assert!(!can_submit(Mode::SignIn, &form, false, &typed("luca", "", 0, 0)), "serve la password");
        assert!(can_submit(Mode::SignUp, &form, false, &typed("", "a@b.it", 8, 0)));
        assert!(!can_submit(Mode::SignUp, &form, false, &typed("", "a@b.it", 7, 0)), "almeno 8");
        assert!(!can_submit(Mode::SignUp, &form, false, &typed("", "ab.it", 9, 0)), "un'email");
        let code = Stage::Code("p***@example.com".into());
        assert!(can_submit(Mode::SignIn, &code, false, &typed("", "", 0, 6)));
        assert!(!can_submit(Mode::SignIn, &code, false, &typed("", "", 0, 5)));
        // Never while a submit is in progress or the browser is awaited.
        assert!(!can_submit(Mode::SignIn, &form, true, &typed("luca", "", 3, 0)));
        assert!(!can_submit(
            Mode::SignIn,
            &Stage::Waiting(Provider::Google),
            false,
            &typed("luca", "", 3, 0)
        ));
    }

    #[test]
    fn the_step_follows_the_core() {
        let mut view = SocialView::default();
        assert_eq!(Mirror::of(&view).stage, Stage::Form);
        view.auth =
            AuthView { step: Some(CodeHint { sent_to: "p***@x.it".to_owned() }), ..AuthView::default() };
        assert_eq!(Mirror::of(&view).stage, Stage::Code("p***@x.it".into()));
        view.auth.waiting = Some(Provider::Discord);
        view.auth.busy = true;
        let mirror = Mirror::of(&view);
        assert_eq!(mirror.stage, Stage::Waiting(Provider::Discord), "il browser vince sul codice");
        assert!(mirror.busy);
        view.auth = AuthView { error: Some(AppError::AuthFailed), ..AuthView::default() };
        assert_eq!(Mirror::of(&view).error.as_deref(), Some("Accesso non riuscito. Riprova."));
        assert!(Mirror::of(&view).available);
        view.account = Account::Unavailable;
        assert!(!Mirror::of(&view).available);
    }

    #[test]
    fn the_sent_command_never_shows_the_secret() {
        let form = Stage::Form;
        let secret = secret();
        let commands = [
            command(Mode::SignIn, &form, &typed(" luca ", "", 15, 0), secret.clone()),
            command(Mode::SignUp, &form, &typed("", "a@b.it", 15, 0), secret.clone()),
            command(Mode::SignIn, &Stage::Code("x".into()), &typed("", "", 0, 6), secret.clone()),
        ];
        for command in commands {
            let command = command.expect("un comando");
            let printed = format!("{command:?}");
            assert!(!printed.contains(&secret), "{printed}");
            assert!(printed.contains("redacted"), "{printed}");
        }
        match command(Mode::SignIn, &form, &typed(" luca ", "", 15, 0), secret.clone()) {
            Some(SocialCmd::SignIn { identifier, password }) => {
                assert_eq!(identifier, "luca");
                assert_eq!(password.expose(), secret);
            }
            other => panic!("{other:?}"),
        }
        assert!(
            command(Mode::SignIn, &Stage::Waiting(Provider::GitHub), &typed("", "", 0, 0), secret).is_none()
        );
    }

    #[test]
    fn each_step_takes_its_own_masked_field() {
        assert_eq!(secret_field(Mode::SignIn, &Stage::Form), Some(MaskedField::Password));
        assert_eq!(secret_field(Mode::SignUp, &Stage::Form), Some(MaskedField::NewPassword));
        assert_eq!(secret_field(Mode::SignUp, &Stage::Code("x".into())), Some(MaskedField::Code));
        assert_eq!(secret_field(Mode::SignIn, &Stage::Waiting(Provider::Google)), None);
        assert_eq!(line(Mode::SignUp, &Stage::Form), "Crea il tuo account");
        assert_eq!(line(Mode::SignIn, &Stage::Code("x".into())), "Controlla la posta");
    }
}
