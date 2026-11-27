//! The account as the app sees it: «Accedi a Psytify» (password, the email code, GitHub, Google and
//! Discord through the browser) and who is signed in. The rules of the form are the desktop's
//! (`desktop/src/app/sign_in_page.rs`): when it can be sent, which characters a field takes.
//!
//! The password and the code cross the bridge only as arguments of `PsytifyCore::sign_in`,
//! `sign_up` and `submit_code`, never inside a record (whose generated Kotlin `toString` would print
//! them), and become the core's `Secret` at once.

use std::sync::{Arc, Mutex, PoisonError, Weak};

use psytify_core::auth::Provider;
use psytify_core::platform;
use psytify_core::snapshot::{Account, SocialView};

use crate::CoreListener;

/// The listener of the latest bridge, which opens the browser for the core (`Effect::OpenUrl`).
static BROWSER: Mutex<Option<Weak<dyn CoreListener>>> = Mutex::new(None);

/// Makes `listener` the one that opens addresses in the browser. The core's hook is process-wide and
/// set once; it reads the latest listener, held weakly.
pub(crate) fn install_browser(listener: &Arc<dyn CoreListener>) {
    *BROWSER.lock().unwrap_or_else(PoisonError::into_inner) = Some(Arc::downgrade(listener));
    platform::set_url_opener(|url| {
        let listener =
            BROWSER.lock().unwrap_or_else(PoisonError::into_inner).as_ref().and_then(Weak::upgrade);
        listener.is_some_and(|listener| listener.open_browser(url.to_owned()))
    });
}

/// A provider that signs in through the browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SignInProvider {
    Google,
    GitHub,
    Discord,
}

impl SignInProvider {
    pub(crate) fn provider(self) -> Provider {
        match self {
            Self::Google => Provider::Google,
            Self::GitHub => Provider::GitHub,
            Self::Discord => Provider::Discord,
        }
    }

    fn of(provider: Provider) -> Self {
        match provider {
            Provider::Google => Self::Google,
            Provider::GitHub => Self::GitHub,
            Provider::Discord => Self::Discord,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum AccountState {
    /// Sign-in is not configured in this build: no button.
    Unavailable,
    SignedOut,
    /// A sign-in on its way, or the saved session being restored at start.
    Working,
    SignedIn,
}

/// Where the form is.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum SignInStep {
    Form,
    /// Clerk sent a code to this address (already partly hidden by Clerk).
    Code {
        sent_to: String,
    },
    /// The browser is open on this provider; the core waits for it to come back.
    Browser {
        provider: SignInProvider,
    },
}

/// The account at one publication of the social domain.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AccountSnapshot {
    pub state: AccountState,
    pub step: SignInStep,
    /// A request is on its way: the form takes no other.
    pub busy: bool,
    /// Why the last attempt failed, shown under the button.
    pub error: Option<String>,
    /// Who is signed in; `None` also while signed in without a username yet.
    pub handle: Option<String>,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
    /// Signed in, but the profile (username) is still to be chosen.
    pub needs_profile: bool,
    /// The profile is being saved.
    pub saving: bool,
}

impl AccountSnapshot {
    pub(crate) fn of(view: &SocialView) -> Self {
        let auth = &view.auth;
        let step = match (&auth.waiting, &auth.step) {
            (Some(provider), _) => SignInStep::Browser { provider: SignInProvider::of(*provider) },
            (None, Some(hint)) => SignInStep::Code { sent_to: hint.sent_to.clone() },
            (None, None) => SignInStep::Form,
        };
        let state = match &view.account {
            Account::Unavailable => AccountState::Unavailable,
            Account::SignedOut => AccountState::SignedOut,
            Account::Working => AccountState::Working,
            Account::SignedIn(_) => AccountState::SignedIn,
        };
        let profile = view.account.profile();
        Self {
            state,
            step,
            busy: auth.busy,
            error: auth.error.as_ref().map(ToString::to_string),
            handle: profile.map(|p| p.handle.clone()),
            name: profile.map(|p| p.display_name.clone()),
            avatar_url: profile.and_then(|p| p.avatar_url.clone()),
            needs_profile: view.account.me().is_some() && profile.is_none(),
            saving: view.profile_saving,
        }
    }
}

/// The tab of the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum SignInMode {
    SignIn,
    SignUp,
}

/// Whether the form can be sent, the desktop's rule: to sign in a name and a password; to sign up an
/// email and a password of at least 8 characters; for the code 6 digits; never while a request is on
/// its way or the browser is open. Only the lengths of the masked fields cross.
#[uniffi::export]
pub fn can_submit_sign_in(
    mode: SignInMode,
    account: &AccountSnapshot,
    identifier: &str,
    email: &str,
    password_chars: u32,
    code_chars: u32,
) -> bool {
    if account.busy {
        return false;
    }
    match &account.step {
        SignInStep::Browser { .. } => false,
        SignInStep::Code { .. } => code_chars >= 6,
        SignInStep::Form => match mode {
            SignInMode::SignIn => !identifier.trim().is_empty() && password_chars > 0,
            SignInMode::SignUp => email.contains('@') && password_chars >= 8,
        },
    }
}

/// A username as typed: ASCII letters, digits and `_`, lower case.
#[uniffi::export]
pub fn handle_chars(text: &str) -> String {
    text.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').map(|c| c.to_ascii_lowercase()).collect()
}

/// An email code as typed: digits only, at most 6.
#[uniffi::export]
pub fn code_chars(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).take(6).collect()
}

#[cfg(test)]
mod tests {
    use psytify_core::auth::Secret;
    use psytify_core::command::{Command, SocialCmd};
    use psytify_core::config::Config;
    use psytify_core::testing::LocalCore;

    use super::*;

    fn core() -> LocalCore {
        let mut config = Config::default();
        config.core.clerk_issuer = "https://clerk.example.test".into();
        LocalCore::new(config, 1)
    }

    #[test]
    fn the_form_follows_the_core_from_the_fields_to_the_code() {
        let mut core = core();
        let shown = AccountSnapshot::of(&core.snapshot().social);
        assert_eq!((shown.state.clone(), shown.step.clone()), (AccountState::SignedOut, SignInStep::Form));
        assert!(can_submit_sign_in(SignInMode::SignIn, &shown, "ada", "", 3, 0));
        assert!(!can_submit_sign_in(SignInMode::SignIn, &shown, "  ", "", 3, 0));
        assert!(!can_submit_sign_in(SignInMode::SignUp, &shown, "", "ada@example.test", 7, 0), "8 at least");

        let sign_in = SocialCmd::SignIn { identifier: "ada".into(), password: Secret::new("x") };
        core.send(Command::Social(sign_in));
        let shown = AccountSnapshot::of(&core.snapshot().social);
        assert!(shown.busy && !can_submit_sign_in(SignInMode::SignIn, &shown, "ada", "", 3, 0));

        core.sign_in_needs_code("a***@example.test");
        let shown = AccountSnapshot::of(&core.snapshot().social);
        assert_eq!(shown.step, SignInStep::Code { sent_to: "a***@example.test".into() });
        assert!(can_submit_sign_in(SignInMode::SignIn, &shown, "", "", 0, 6));
        assert!(!can_submit_sign_in(SignInMode::SignIn, &shown, "", "", 0, 5));

        core.sign_in_failed(psytify_core::error::AppError::AuthFailed);
        let shown = AccountSnapshot::of(&core.snapshot().social);
        assert!(shown.error.is_some());
    }

    #[test]
    fn the_browser_step_waits_and_the_fields_filter_their_characters() {
        let mut core = core();
        core.send(Command::Social(SocialCmd::SignInWith(Provider::GitHub)));
        let shown = AccountSnapshot::of(&core.snapshot().social);
        let busy_or_waiting = shown.busy || matches!(shown.step, SignInStep::Browser { .. });
        assert!(busy_or_waiting, "{shown:?}");
        assert!(!can_submit_sign_in(SignInMode::SignIn, &shown, "ada", "", 3, 6));
        assert_eq!(handle_chars("Ada Lovelace_1!"), "adalovelace_1");
        assert_eq!(code_chars(" 12a3-4567 "), "123456");
    }

    #[test]
    fn without_clerk_there_is_no_sign_in() {
        let mut config = Config::default();
        config.core.clerk_issuer = String::new();
        let core = LocalCore::new(config, 1);
        let state = AccountSnapshot::of(&core.snapshot().social).state;
        // An empty issuer falls back to the build's default, which a test build may or may not have.
        assert!(matches!(state, AccountState::Unavailable | AccountState::SignedOut));
    }
}
