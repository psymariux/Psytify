//! Account e amici: la schermata «Accedi a Psytify» (password, codice via email, GitHub, Google e
//! Discord), il profilo con il nome utente, gli amici con le richieste, le pagine degli altri e le
//! loro playlist pubbliche. Dal passo 4.7 la logica sta nel core (`core/src/state/social.rs`): la
//! sessione e i suoi token, i passi di Clerk, l'accesso con il browser, il sondaggio degli amici, il
//! brano in ascolto, gli ascolti e le playlist pubbliche. Qui restano i campi di testo, che sono
//! dell'interfaccia, i messaggi che diventano `SocialCmd`, la vista letta dall'istantanea
//! (`SocialState::view`) e gli eventi del core tradotti negli avvisi di sempre.

use std::sync::Arc;

use iced::Task;
use psytify_core::Command;
use psytify_core::auth::Secret;
use psytify_core::command::{LibraryCmd, SocialCmd};
use psytify_core::snapshot::{Account, AuthView, OpenProfile, SocialEvent, SocialView};
use uuid::Uuid;

use super::{App, Load, Message, Notice, Screen};
use crate::api::social::{MyProfile, PublicPlaylist};
use crate::auth::Provider;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMode {
    #[default]
    SignIn,
    SignUp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthField {
    Identifier,
    Email,
    Username,
    Password,
    Code,
}

/// I campi del modulo di accesso. La password e il codice restano solo qui e spariscono appena si
/// manda il passo: il core li riceve come `Secret`, che non si stampa.
#[derive(Debug, Default)]
pub struct SignInForm {
    pub mode: AuthMode,
    pub identifier: String,
    pub email: String,
    pub username: String,
    pub password: String,
    pub code: String,
}

impl SignInForm {
    /// Il passo si può mandare: ci sono i campi giusti e un altro non è già in volo. Dove si è
    /// (il codice da scrivere, l'invio in corso) lo dice il core in `auth`.
    pub fn can_submit(&self, auth: &AuthView) -> bool {
        if auth.busy {
            return false;
        }
        if auth.step.is_some() {
            return self.code.trim().len() >= 6;
        }
        match self.mode {
            AuthMode::SignIn => !self.identifier.trim().is_empty() && !self.password.is_empty(),
            AuthMode::SignUp => self.email.contains('@') && self.password.chars().count() >= 8,
        }
    }

    fn clear_secrets(&mut self) {
        self.password.clear();
        self.code.clear();
    }
}

/// Le modifiche al proprio profilo, nelle impostazioni.
#[derive(Debug, Clone, Default)]
pub struct ProfileEdit {
    pub handle: String,
    pub display_name: String,
}

#[derive(Debug, Default)]
pub struct SocialState {
    /// Ciò che il core pubblica di account, amici, profili e inviti: l'ultima istantanea.
    pub view: Arc<SocialView>,
    pub form: SignInForm,
    pub edit: ProfileEdit,
    pub query: String,
    /// Amici scelti nella schermata Jam, da invitare.
    pub invitees: Vec<String>,
    /// Il numero dell'ultimo evento del core già mostrato.
    event: u64,
}

impl App {
    pub fn signed_in_profile(&self) -> Option<&MyProfile> {
        self.social.view.account.profile()
    }

    /// Richieste d'amicizia ricevute: il numero accanto ad «Amici».
    pub fn friend_requests(&self) -> usize {
        self.social.view.friends.ready().map_or(0, |friends| friends.incoming.len())
    }

    /// Un comando al dominio sociale del core. Ciò che l'utente deve sapere lo dicono gli eventi, in
    /// [`App::social_published`].
    pub(super) fn send_social(&mut self, command: SocialCmd) -> Task<Message> {
        self.send_core(Command::Social(command))
    }

    /// Copia nell'app la vista sociale dell'ultima istantanea, senza mostrare niente: lo stato
    /// iniziale. Gli eventi già nella vista non si rimostrano.
    pub(super) fn copy_social(&mut self) {
        self.social.view = Arc::clone(&self.core_snapshot.social);
        self.social.event = self.social.view.events.back().map_or(0, |raised| raised.seq);
    }

    /// Il dominio sociale è cambiato: si tiene la vista nuova, si dice ciò che gli eventi dicono (in
    /// ordine) e si reagisce a ciò che è cambiato davvero: l'accesso riuscito o finito, il profilo
    /// salvato, le copertine delle persone arrivate.
    pub(super) fn social_published(&mut self, previous: &Arc<SocialView>) -> Task<Message> {
        let view = Arc::clone(&self.core_snapshot.social);
        self.social.view = Arc::clone(&view);
        let mut tasks = Vec::new();

        let shown = self.social.event;
        for raised in view.events.iter().filter(|raised| raised.seq > shown) {
            self.social.event = raised.seq;
            let text = raised.event.message();
            self.notice =
                Some(if raised.event.is_error() { Notice::Error(text) } else { Notice::Info(text) });
            if matches!(raised.event, SocialEvent::InvitesSent(_)) {
                self.social.invitees.clear();
            }
        }

        let (was_in, is_in) =
            (matches!(previous.account, Account::SignedIn(_)), matches!(view.account, Account::SignedIn(_)));
        if is_in && !was_in {
            self.social.form = SignInForm::default();
            if self.screen == Screen::SignIn {
                tasks.push(self.go_back_or_home());
            }
        } else if was_in && !is_in {
            // Chi esce lascia tutto ciò che era suo: il modulo resta com'è.
            let form = std::mem::take(&mut self.social.form);
            let event = self.social.event;
            let view = Arc::clone(&self.social.view);
            self.social = SocialState { view, form, event, ..SocialState::default() };
        }
        if previous.account.profile() != view.account.profile() {
            if let Some(profile) = view.account.profile() {
                self.social.edit = ProfileEdit {
                    handle: profile.handle.clone(),
                    display_name: profile.display_name.clone(),
                };
            }
            let avatar = self.signed_in_profile().and_then(|p| p.avatar_url.clone());
            tasks.push(self.fetch_covers(avatar.into_iter().collect()));
        }
        // Il codice appena arrivato: il campo parte vuoto.
        if view.auth.step.is_some() && previous.auth.step.is_none() {
            self.social.form.code.clear();
        }

        tasks.push(self.fetch_social_covers(previous, &view));
        Task::batch(tasks)
    }

    /// Le copertine e gli avatar di ciò che è arrivato: amici, ricerca, profilo e playlist aperti.
    /// Si scaricano una volta sola per risposta: solo se il dato pronto è un'altra allocazione.
    fn fetch_social_covers(&mut self, previous: &SocialView, view: &SocialView) -> Task<Message> {
        let mut urls: Vec<String> = Vec::new();
        if let Some(friends) = arrived(ready(&previous.friends), ready(&view.friends)) {
            urls.extend(
                friends
                    .friends
                    .iter()
                    .flat_map(|f| {
                        f.profile
                            .avatar_url
                            .iter()
                            .chain(f.now_playing.iter().filter_map(|n| n.track.cover_url.as_ref()))
                    })
                    .chain(
                        friends
                            .incoming
                            .iter()
                            .chain(&friends.outgoing)
                            .filter_map(|p| p.avatar_url.as_ref()),
                    )
                    .cloned(),
            );
        }
        if let Some(found) = arrived(ready(&previous.found), ready(&view.found)) {
            urls.extend(found.iter().filter_map(|p| p.avatar_url.clone()));
        }
        if let Some(open) = arrived(opened(previous), opened(view)) {
            let page = &open.page;
            urls.extend(page.profile.avatar_url.iter().cloned());
            urls.extend(page.mutual_friends.iter().filter_map(|p| p.avatar_url.clone()));
            if let Some(activity) = &page.activity {
                urls.extend(activity.now_playing.iter().filter_map(|n| n.track.cover_url.clone()));
                urls.extend(activity.recent.iter().filter_map(|l| l.track.cover_url.clone()));
            }
        }
        if let Some(playlist) = arrived(open_playlist(previous), open_playlist(view)) {
            urls.extend(playlist.tracks.iter().filter_map(|t| t.cover_url.clone()));
            urls.extend(playlist.owner.avatar_url.clone());
        }
        if urls.is_empty() { Task::none() } else { self.fetch_covers(urls) }
    }

    // ---------- Accesso ----------

    pub(super) fn open_sign_in(&mut self) -> Task<Message> {
        if self.social.view.account == Account::Unavailable {
            self.notice = Some(Notice::Error(crate::error::AppError::AuthNotConfigured.to_string()));
            return Task::none();
        }
        let clear = self.send_social(SocialCmd::DismissError);
        Task::batch([clear, self.open(Screen::SignIn)])
    }

    pub(super) fn auth_input(&mut self, field: AuthField, value: String) -> Task<Message> {
        let form = &mut self.social.form;
        let target = match field {
            AuthField::Identifier => &mut form.identifier,
            AuthField::Email => &mut form.email,
            AuthField::Username => &mut form.username,
            AuthField::Password => &mut form.password,
            AuthField::Code => &mut form.code,
        };
        // Niente a capo né caratteri di controllo: arrivano solo incollando.
        *target = value.chars().filter(|c| !c.is_control()).take(256).collect();
        // Riprendere a scrivere toglie l'errore del tentativo di prima.
        if self.social.view.auth.error.is_some() {
            return self.send_social(SocialCmd::DismissError);
        }
        Task::none()
    }

    pub(super) fn auth_mode(&mut self, mode: AuthMode) -> Task<Message> {
        let form = &mut self.social.form;
        form.mode = mode;
        form.clear_secrets();
        self.send_social(SocialCmd::CancelSignIn)
    }

    pub(super) fn auth_submit(&mut self) -> Task<Message> {
        let auth = &self.social.view.auth;
        let form = &mut self.social.form;
        if !form.can_submit(auth) {
            return Task::none();
        }
        let command = match (&auth.step, form.mode) {
            (Some(_), _) => SocialCmd::SubmitCode(Secret::new(std::mem::take(&mut form.code))),
            (None, AuthMode::SignIn) => SocialCmd::SignIn {
                identifier: form.identifier.clone(),
                password: Secret::new(std::mem::take(&mut form.password)),
            },
            (None, AuthMode::SignUp) => SocialCmd::SignUp {
                email: form.email.clone(),
                username: form.username.clone(),
                password: Secret::new(std::mem::take(&mut form.password)),
            },
        };
        self.send_social(command)
    }

    pub(super) fn auth_provider(&mut self, provider: Provider) -> Task<Message> {
        if self.social.view.auth.busy {
            return Task::none();
        }
        self.send_social(SocialCmd::SignInWith(provider))
    }

    pub(super) fn auth_resend(&mut self) -> Task<Message> {
        self.send_social(SocialCmd::ResendCode)
    }

    /// Torna dal codice ai campi, o rinuncia ad aspettare il browser.
    pub(super) fn auth_back(&mut self) -> Task<Message> {
        self.social.form.clear_secrets();
        self.send_social(SocialCmd::CancelSignIn)
    }

    pub(super) fn sign_out(&mut self) -> Task<Message> {
        self.send_social(SocialCmd::SignOut)
    }

    fn go_back_or_home(&mut self) -> Task<Message> {
        if self.can_go_back() { self.update(Message::GoBack) } else { self.open(Screen::Home) }
    }

    // ---------- Profilo ----------

    pub(super) fn profile_input(&mut self, handle: Option<String>, name: Option<String>) -> Task<Message> {
        if let Some(handle) = handle {
            self.social.edit.handle = handle
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                .take(30)
                .collect::<String>()
                .to_lowercase();
        }
        if let Some(name) = name {
            self.social.edit.display_name = name.chars().filter(|c| !c.is_control()).take(64).collect();
        }
        Task::none()
    }

    /// Salva nome utente, nome e condivisione dell'attività.
    pub(super) fn save_profile(&mut self, share: Option<bool>) -> Task<Message> {
        let edit = &self.social.edit;
        let command = SocialCmd::SaveProfile {
            handle: edit.handle.clone(),
            display_name: edit.display_name.clone(),
            share_activity: share,
        };
        self.send_social(command)
    }

    // ---------- Amici e profili ----------

    /// Rilegge gli amici e le richieste (la pagina Amici si apre, o «Riprova»).
    pub(super) fn load_friends(&mut self) -> Task<Message> {
        self.send_social(SocialCmd::RefreshFriends)
    }

    /// Aggiunge o toglie un amico dalla scelta di chi invitare.
    pub(super) fn toggle_invitee(&mut self, handle: String) {
        if let Some(at) = self.social.invitees.iter().position(|h| *h == handle) {
            self.social.invitees.remove(at);
        } else if self.social.invitees.len() < crate::jam::MAX_MEMBERS {
            self.social.invitees.push(handle);
        }
    }

    /// Manda l'invito agli amici scelti, per la jam in cui si è.
    pub(super) fn send_jam_invites(&mut self) -> Task<Message> {
        if self.social.invitees.is_empty() || self.social.view.inviting {
            return Task::none();
        }
        let handles = self.social.invitees.clone();
        self.send_social(SocialCmd::InviteToJam(handles))
    }

    /// Entra nella jam di un invito e lo toglie dall'elenco.
    pub(super) fn accept_jam_invite(&mut self, code: String) -> Task<Message> {
        if self.jam.is_active() {
            self.notice = Some(Notice::Info("Sei già in una jam: esci prima da quella".into()));
            return Task::none();
        }
        let dismiss = self.dismiss_jam_invite(code.clone());
        Task::batch([dismiss, self.join_jam(code)])
    }

    /// Toglie l'invito subito dalla vista e lo fa togliere anche dal server.
    pub(super) fn dismiss_jam_invite(&mut self, code: String) -> Task<Message> {
        self.send_social(SocialCmd::DismissInvite(code))
    }

    pub(super) fn user_query(&mut self, query: String) -> Task<Message> {
        self.social.query = query.chars().filter(|c| !c.is_control()).take(100).collect();
        if self.social.query.trim().trim_start_matches('@').is_empty() {
            return self.send_social(SocialCmd::SearchUsers(String::new()));
        }
        Task::none()
    }

    pub(super) fn search_users(&mut self) -> Task<Message> {
        let query = self.social.query.trim().to_owned();
        if query.trim_start_matches('@').is_empty() {
            return Task::none();
        }
        self.send_social(SocialCmd::SearchUsers(query))
    }

    pub(super) fn open_profile(&mut self, handle: String) -> Task<Message> {
        if !matches!(self.social.view.account, Account::SignedIn(_)) {
            return self.open_sign_in();
        }
        let navigate = self.open(Screen::Profile);
        Task::batch([navigate, self.send_social(SocialCmd::OpenProfile(handle))])
    }

    /// Chiede o accetta l'amicizia (`add`), oppure la toglie, la ritira o la rifiuta.
    pub(super) fn friend_action(&mut self, handle: String, add: bool) -> Task<Message> {
        self.send_social(SocialCmd::FriendAction { handle, add })
    }

    pub(super) fn open_friend_playlist(&mut self, handle: String, id: Uuid) -> Task<Message> {
        let navigate = self.open(Screen::FriendPlaylist);
        Task::batch([navigate, self.send_social(SocialCmd::OpenFriendPlaylist { handle, id })])
    }

    /// Rende pubblica (o di nuovo privata) una playlist locale. Al server ci pensa il core, che vede
    /// la libreria cambiare.
    pub(super) fn set_playlist_public(&mut self, id: Uuid, public: bool) -> Task<Message> {
        let Some(playlist) = self.playlist(id) else { return Task::none() };
        if playlist.public == public {
            return Task::none();
        }
        let set = self.send_library(LibraryCmd::SetPlaylistPublic { id, public });
        self.notice = Some(Notice::Info(
            if public { "La playlist ora è visibile sul tuo profilo" } else { "La playlist ora è privata" }
                .into(),
        ));
        set
    }

    /// I messaggi di questo dominio, smistati da `App::handle`.
    pub(super) fn handle_social(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::SignIn => self.open_sign_in(),
            Message::AuthMode(mode) => self.auth_mode(mode),
            Message::AuthInput(field, value) => self.auth_input(field, value),
            Message::AuthSubmit => self.auth_submit(),
            Message::AuthProvider(provider) => self.auth_provider(provider),
            Message::AuthResend => self.auth_resend(),
            Message::AuthBack => self.auth_back(),
            Message::SignOut => self.sign_out(),
            Message::ProfileHandle(handle) => self.profile_input(Some(handle), None),
            Message::ProfileName(name) => self.profile_input(None, Some(name)),
            Message::ProfileSave(share) => self.save_profile(share),
            Message::JamInviteToggle(handle) => {
                self.toggle_invitee(handle);
                Task::none()
            }
            Message::JamInviteSend => self.send_jam_invites(),
            Message::JamInviteAccept(code) => self.accept_jam_invite(code),
            Message::JamInviteDismiss(code) => self.dismiss_jam_invite(code),
            Message::UserQuery(query) => self.user_query(query),
            Message::UserSearch => self.search_users(),
            Message::OpenProfile(handle) => self.open_profile(handle),
            Message::FriendAction(handle, add) => self.friend_action(handle, add),
            Message::OpenFriendPlaylist(handle, id) => self.open_friend_playlist(handle, id),
            Message::PlaylistPublic(id, public) => self.set_playlist_public(id, public),
            // `handle` manda qui solo i messaggi di questo dominio.
            _ => Task::none(),
        }
    }
}

// ---------- Discord ----------

impl App {
    /// L'Application ID di Discord: quello di `config.json`, oppure quello della build.
    fn discord_app_id(&self) -> &str {
        let chosen = self.discord_app_id.trim();
        if chosen.is_empty() { crate::config::DEFAULT_DISCORD_APP_ID } else { chosen }
    }

    /// `true` se la build o la configurazione hanno un'app Discord: senza, l'interruttore non c'è.
    pub fn discord_available(&self) -> bool {
        cfg!(not(target_os = "android")) && !self.discord_app_id().is_empty()
    }
}

/// Il profilo aperto, se è pronto.
fn opened(view: &SocialView) -> Option<&Arc<OpenProfile>> {
    view.open.as_ref().and_then(|(_, load)| ready(load))
}

/// La playlist pubblica aperta, se è pronta.
fn open_playlist(view: &SocialView) -> Option<&Arc<PublicPlaylist>> {
    view.playlist.as_ref().and_then(ready)
}

/// Il dato pronto di un caricamento, se c'è.
fn ready<T: ?Sized>(load: &Load<Arc<T>>) -> Option<&Arc<T>> {
    match load {
        Load::Ready(value) => Some(value),
        _ => None,
    }
}

/// Il dato pronto di adesso, se è un altro da quello di prima (un'altra allocazione).
fn arrived<'a, T: ?Sized>(old: Option<&Arc<T>>, new: Option<&'a Arc<T>>) -> Option<&'a Arc<T>> {
    new.filter(|new| !old.is_some_and(|old| Arc::ptr_eq(old, new)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_form_asks_only_for_what_it_needs() {
        let idle = AuthView::default();
        let mut form = SignInForm { identifier: "psy".into(), password: "x".into(), ..SignInForm::default() };
        assert!(form.can_submit(&idle));
        form.mode = AuthMode::SignUp;
        form.email = "psy@example.com".into();
        assert!(!form.can_submit(&idle), "la password di una registrazione vuole 8 caratteri");
        form.password = "abcdefgh".into();
        assert!(form.can_submit(&idle));
        assert!(!form.can_submit(&AuthView { busy: true, ..AuthView::default() }), "un invio alla volta");
        // Con il codice da scrivere, serve il codice e non più i campi.
        let step = AuthView {
            step: Some(psytify_core::snapshot::CodeHint { sent_to: "p**@example.com".into() }),
            ..AuthView::default()
        };
        assert!(!form.can_submit(&step));
        form.code = "424242".into();
        assert!(form.can_submit(&step));
    }

    #[test]
    fn friends_are_chosen_and_unchosen_for_a_jam() {
        let mut app = App::for_tests();
        app.toggle_invitee("luca".into());
        app.toggle_invitee("sara".into());
        app.toggle_invitee("luca".into());
        assert_eq!(app.social.invitees, ["sara"]);
        for i in 0..20 {
            app.toggle_invitee(format!("u{i}"));
        }
        assert_eq!(app.social.invitees.len(), crate::jam::MAX_MEMBERS, "non più dei posti della stanza");
    }
}
