//! «Accedi a Psytify» (`Page::SignIn`), come da `PSYTIFY_ART_DIRECTION.md` («Sign-in / social») e dal
//! client Iced (`client/src/ui/pages/sign_in.rs`): una colonna di `SIGN_IN_W` (360 dp) sulla tela, Nastro
//! (`nastro_idle`) sopra il wordmark e una riga che cambia con il passo; le schede Accedi / Crea un account;
//! i campi da 40 dp; i pulsanti di GitHub, Google e Discord (secondari, con il marchio monocromo del
//! provider a 20 dp, l'unica eccezione vettoriale); gli errori in `TEXT_ERROR` sotto il pulsante, mai in un
//! avviso. Il passo del codice via email (`AuthView::step`) e l'attesa del browser (`AuthView::waiting`, con
//! «Annulla» → `CancelSignIn`) prendono il posto dei campi. Niente lime: la direzione artistica non ne vuole
//! sull'accesso.
//!
//! La password e il codice stanno solo nei loro campi (mascherati): all'invio il testo si prende e il campo
//! si svuota nello stesso momento (`SearchField::take`), poi va al core avvolto in `Secret`. Niente di loro
//! passa dall'istantanea, dai `Debug`, dai log o dagli avvisi. Invio manda il modulo, Tab passa al campo
//! dopo. Scrivere di nuovo toglie l'errore mostrato (`DismissError`), come nel client Iced.
//!
//! La pagina copia dalla vista sociale solo ciò che mostra (`Mirror`), quando il core pubblica; `render`
//! non legge l'istantanea.

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
use super::people::{TEXT_MAX, code_chars, handle_chars, line_chars};
use crate::components::button::Button;
use crate::components::navigation::{Tab, Tabs};
use crate::components::search_field::SearchField;
use crate::components::separator::Separator;
use crate::core_model::{Changed, CoreModel};
use crate::pixel::PixelSprite;
use crate::theme::{Ap, TypeStyle, col, dp};

/// La scheda del modulo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    #[default]
    SignIn,
    SignUp,
}

/// Il passo dell'accesso, dalla vista del core.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Stage {
    /// I campi della scheda.
    #[default]
    Form,
    /// Il codice mandato a questo indirizzo (già mascherato dal core).
    Code(SharedString),
    /// Si aspetta il ritorno dal browser di questo provider.
    Waiting(Provider),
}

/// Ciò che la pagina mostra della vista sociale.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Mirror {
    stage: Stage,
    busy: bool,
    error: Option<SharedString>,
    /// Il login è configurato.
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

/// Quanto c'è nei campi: i testi in chiaro e, dei campi mascherati, solo quanti caratteri hanno.
#[derive(Clone, Copy, Debug, Default)]
struct Typed<'a> {
    identifier: &'a str,
    email: &'a str,
    username: &'a str,
    password_chars: usize,
    code_chars: usize,
}

/// Il modulo si può mandare (le regole del client Iced, `SignInForm::can_submit`): per l'accesso un nome e
/// una password, per la registrazione un'email e una password di almeno 8 caratteri, per il codice 6 cifre;
/// mai mentre un invio è in corso o si aspetta il browser.
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

/// Il campo mascherato che l'invio svuota.
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

/// Il comando dell'invio, con il testo del campo mascherato già preso (e subito avvolto in `Secret`).
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

/// La riga sotto il wordmark, che cambia con il passo.
fn line(mode: Mode, stage: &Stage) -> &'static str {
    match (stage, mode) {
        (Stage::Code(_), _) => "Controlla la posta",
        (Stage::Waiting(_), _) => "Ti aspettiamo nel browser",
        (Stage::Form, Mode::SignIn) => "Accedi per ascoltare con i tuoi amici",
        (Stage::Form, Mode::SignUp) => "Crea il tuo account",
    }
}

/// Il marchio di un provider.
fn mark(provider: Provider) -> &'static [u8] {
    match provider {
        Provider::GitHub => brand::GITHUB,
        Provider::Google => brand::GOOGLE,
        Provider::Discord => brand::DISCORD,
    }
}

/// I campi del modulo.
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
    /// Il pulsante principale è acceso (ricalcolato a ogni tasto, la pagina si ridisegna solo se cambia).
    ready: bool,
    /// Alla prossima occasione il focus va al primo campo del passo (la pagina si è aperta, il passo è
    /// cambiato).
    focus_first: bool,
    focus: FocusHandle,
    _changes: Subscription,
}

impl EventEmitter<PageEvent> for SignInPage {}

/// Un campo del modulo: 40 dp, senza icone; ogni tasto e Invio tornano alla pagina dopo che il campo ha
/// finito il suo aggiornamento (`App::defer`: il campo è ancora in prestito mentre chiama).
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
            focus: cx.focus_handle(),
            _changes: changes,
        }
    }

    /// La pagina si apre: il focus andrà al primo campo.
    pub fn show(&mut self, cx: &mut Context<Self>) {
        self.focus_first = true;
        cx.notify();
    }

    fn take(&mut self, view: &Arc<SocialView>, cx: &mut Context<Self>) {
        let mirror = Mirror::of(view);
        if mirror == self.mirror {
            return;
        }
        if mirror.stage != self.mirror.stage {
            // Un passo nuovo comincia con il suo campo vuoto e con il focus.
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

    /// Un campo è cambiato: l'errore mostrato se ne va, il pulsante si accende o si spegne.
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

    /// Invio o il pulsante principale: prende il testo mascherato (il campo si svuota subito) e lo manda.
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

    /// Cambia scheda: le password se ne vanno, come un passo del codice lasciato a metà (`CancelSignIn`).
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
        let ap = Ap::of(window);
        if self.focus_first {
            // Il focus al primo campo del passo, quando la pagina si apre o il passo cambia (mentre si aspetta
            // il browser non c'è un campo).
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
                    .child(div().pixel_type(ap, false).text_color(col(TEXT_PRIMARY)).child("Psytify"))
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
            .child(column)
    }
}

#[cfg(test)]
mod tests {
    use psytify_core::error::AppError;
    use psytify_core::snapshot::{AuthView, CodeHint};

    use super::*;

    const SECRET: &str = "hunter2-segreta";

    fn typed(identifier: &'static str, email: &'static str, password: usize, code: usize) -> Typed<'static> {
        Typed { identifier, email, username: "", password_chars: password, code_chars: code }
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
        // Mai mentre un invio è in corso o si aspetta il browser.
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
        let commands = [
            command(Mode::SignIn, &form, &typed(" luca ", "", 15, 0), SECRET.to_owned()),
            command(Mode::SignUp, &form, &typed("", "a@b.it", 15, 0), SECRET.to_owned()),
            command(Mode::SignIn, &Stage::Code("x".into()), &typed("", "", 0, 6), SECRET.to_owned()),
        ];
        for command in commands {
            let command = command.expect("un comando");
            let printed = format!("{command:?}");
            assert!(!printed.contains(SECRET), "{printed}");
            assert!(printed.contains("redacted"), "{printed}");
        }
        match command(Mode::SignIn, &form, &typed(" luca ", "", 15, 0), SECRET.to_owned()) {
            Some(SocialCmd::SignIn { identifier, password }) => {
                assert_eq!(identifier, "luca");
                assert_eq!(password.expose(), SECRET);
            }
            other => panic!("{other:?}"),
        }
        assert!(
            command(Mode::SignIn, &Stage::Waiting(Provider::GitHub), &typed("", "", 0, 0), String::new())
                .is_none()
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
