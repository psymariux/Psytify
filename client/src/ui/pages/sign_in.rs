//! «Accedi a Psytify»: il modulo di accesso dentro l'app, senza pagine di Clerk. In cima il marchio,
//! poi la scelta fra accedere e creare un account, i tre accessi con il browser (Google, GitHub,
//! Discord) e i campi. Quando Clerk manda un codice via email, la colonna mostra solo quello.

use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Alignment, Element, Length, alignment};

use crate::app::{App, AuthField, AuthMode, Message, SignInForm};
use crate::auth::Provider;
use crate::ui::components::{muted, small, text_button};
use crate::ui::icons::{Icon, icon, mark};
use crate::ui::theme;

pub const CODE_INPUT: &str = "sign-in-code";

pub fn view(app: &App, width: f32) -> Element<'_, Message> {
    let form = &app.social.form;
    let column_width = width.min(theme::SIGN_IN_WIDTH);
    let header = column![
        mark(56.0),
        text("Psytify").size(theme::FS_PAGE).font(theme::FONT_SEMIBOLD),
        muted(match (&form.step, form.mode) {
            (Some(_), _) => "Controlla la posta",
            (None, AuthMode::SignIn) => "Accedi per ascoltare con i tuoi amici",
            (None, AuthMode::SignUp) => "Crea il tuo account",
        }),
    ]
    .spacing(10)
    .align_x(Alignment::Center);

    let body: Element<'_, Message> = match &form.step {
        Some(step) => code(form, &step.sent_to),
        None if form.waiting.is_some() => waiting(form.waiting),
        None => fields(form),
    };
    let mut page = column![header, body].spacing(28).width(column_width).align_x(Alignment::Center);
    if let Some(error) = &form.error {
        page =
            page.push(text(error.as_str()).size(theme::FS_BODY).color(theme::NEGATIVE).width(Length::Fill));
    }
    page = page.push(
        small("Catalogo e ascolto restano liberi anche senza account.")
            .width(Length::Fill)
            .align_x(alignment::Horizontal::Center),
    );
    container(page).width(Length::Fill).align_x(alignment::Horizontal::Center).padding([24, 0]).into()
}

/// Accedere o creare un account: due segmenti a tutta larghezza.
fn modes(form: &SignInForm) -> Element<'static, Message> {
    let choice = |label: &'static str, mode: AuthMode| -> Element<'static, Message> {
        button(container(text(label).size(theme::FS_BODY).font(theme::FONT_MEDIUM)).center(Length::Fill))
            .width(Length::Fill)
            .height(theme::CONTROL_HEIGHT)
            .padding(0)
            .style(theme::segment(form.mode == mode))
            .on_press(Message::AuthMode(mode))
            .into()
    };
    container(
        row![choice("Accedi", AuthMode::SignIn), choice("Crea un account", AuthMode::SignUp)].spacing(2),
    )
    .padding(3)
    .width(Length::Fill)
    .style(theme::segment_well)
    .into()
}

fn field<'a>(
    placeholder: &'a str,
    value: &'a str,
    which: AuthField,
    secure: bool,
    submit: bool,
) -> Element<'a, Message> {
    let mut input = text_input(placeholder, value)
        .on_input(move |v| Message::AuthInput(which, v))
        .secure(secure)
        .size(theme::FS_BODY)
        .padding([12, 16])
        .style(theme::search_input);
    if submit {
        input = input.on_submit(Message::AuthSubmit);
    }
    input.into()
}

fn submit<'a>(label: &'a str, enabled: bool, busy: bool) -> Element<'a, Message> {
    let label = if busy { "Un momento…" } else { label };
    button(container(text(label).size(theme::FS_BODY).font(theme::FONT_SEMIBOLD)).center(Length::Fill))
        .width(Length::Fill)
        .height(theme::FIELD_HEIGHT)
        .padding(0)
        .style(theme::primary)
        .on_press_maybe(enabled.then_some(Message::AuthSubmit))
        .into()
}

/// «Continua con Google»: un pulsante a tutta larghezza con il marchio del provider.
fn provider(provider: Provider, enabled: bool) -> Element<'static, Message> {
    let glyph = match provider {
        Provider::Google => Icon::Google,
        Provider::GitHub => Icon::GitHub,
        Provider::Discord => Icon::Discord,
    };
    let label = format!("Continua con {}", provider.label());
    button(
        container(
            row![icon(glyph, 18.0, theme::TEXT), text(label).size(theme::FS_BODY).font(theme::FONT_MEDIUM)]
                .spacing(10)
                .align_y(Alignment::Center),
        )
        .center(Length::Fill),
    )
    .width(Length::Fill)
    .height(theme::FIELD_HEIGHT)
    .padding(0)
    .style(theme::outlined)
    .on_press_maybe(enabled.then_some(Message::AuthProvider(provider)))
    .into()
}

/// «oppure» fra due righe sottili.
fn divider<'a>() -> Element<'a, Message> {
    let rule = || container(Space::new(Length::Fill, 1.0)).style(theme::bar(theme::CONTROL, 0.0));
    row![rule(), small("oppure"), rule()].spacing(12).align_y(Alignment::Center).into()
}

fn fields(form: &SignInForm) -> Element<'_, Message> {
    let idle = !form.busy;
    let socials = column(Provider::ALL.into_iter().map(|p| provider(p, idle))).spacing(10);
    let inputs: Element<'_, Message> = match form.mode {
        AuthMode::SignIn => column![
            field("Email o nome utente", &form.identifier, AuthField::Identifier, false, true),
            field("Password", &form.password, AuthField::Password, true, true),
            submit("Accedi", form.can_submit(), form.busy),
        ]
        .spacing(12)
        .into(),
        AuthMode::SignUp => column![
            field("Email", &form.email, AuthField::Email, false, true),
            field("Nome utente (facoltativo)", &form.username, AuthField::Username, false, true),
            field("Password, almeno 8 caratteri", &form.password, AuthField::Password, true, true),
            submit("Crea l'account", form.can_submit(), form.busy),
            small("Ti mandiamo un codice per confermare l'email."),
        ]
        .spacing(12)
        .into(),
    };
    column![modes(form), socials, divider(), inputs].spacing(20).width(Length::Fill).into()
}

fn code<'a>(form: &'a SignInForm, sent_to: &'a str) -> Element<'a, Message> {
    let to = if sent_to.is_empty() { "alla tua email".to_owned() } else { format!("a {sent_to}") };
    let input = text_input("Codice di 6 cifre", &form.code)
        .id(text_input::Id::new(CODE_INPUT))
        .on_input(|v| Message::AuthInput(AuthField::Code, v))
        .on_submit(Message::AuthSubmit)
        .size(theme::FS_TITLE)
        .padding([12, 16])
        .style(theme::search_input);
    column![
        muted(format!("Abbiamo mandato un codice {to}. Scrivilo qui sotto.")).width(Length::Fill),
        input,
        submit("Conferma", form.can_submit(), form.busy),
        row![
            text_button("Invia di nuovo", Message::AuthResend),
            Space::with_width(Length::Fill),
            text_button("Indietro", Message::AuthBack)
        ]
        .align_y(Alignment::Center),
    ]
    .spacing(14)
    .width(Length::Fill)
    .into()
}

/// Il browser è aperto sul provider: si aspetta che l'utente torni.
fn waiting<'a>(provider: Option<Provider>) -> Element<'a, Message> {
    let name = provider.map_or("il provider", Provider::label);
    column![
        muted(format!("Completa l'accesso con {name} nel browser: l'app ti aspetta qui."))
            .width(Length::Fill),
        small("Hai chiuso il browser? Torna fra qualche minuto, o riapri l'app."),
    ]
    .spacing(10)
    .width(Length::Fill)
    .into()
}
