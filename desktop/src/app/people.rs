//! Testi e filtri comuni alle pagine sociali (accesso, Amici, profilo, Impostazioni): che cosa ascolta una
//! persona, quanti amici ha, che caratteri accetta un nome utente. Logica pura, come nel client Iced
//! (`client/src/app/social.rs` e `client/src/ui/pages/friends.rs`).

use gpui::SharedString;
use psytify_core::api::social::NowPlaying;

use crate::components::person_row::Status;

/// Il nome utente più lungo (come il server).
pub const HANDLE_MAX: usize = 30;
/// Il nome più lungo che si mostra agli altri.
pub const NAME_MAX: usize = 64;
/// Email, nome utente o email per l'accesso: oltre non è un indirizzo.
pub const TEXT_MAX: usize = 256;

/// Il filtro del nome utente: lettere e cifre ASCII e `_`, in minuscolo.
pub fn handle_chars(text: &str) -> String {
    text.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').map(|c| c.to_ascii_lowercase()).collect()
}

/// Il filtro di un testo su una riga: niente caratteri di controllo (a capo, tabulazioni incollate).
pub fn line_chars(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Il filtro di un codice via email: solo cifre.
pub fn code_chars(text: &str) -> String {
    text.chars().filter(char::is_ascii_digit).collect()
}

/// «Sta ascoltando Titolo · Artista» con il punto dal vivo; in pausa «In pausa: …» senza punto.
pub fn listening(now: &NowPlaying) -> Status {
    let track = format!("{} · {}", now.track.title, now.track.artist.name);
    if now.paused {
        Status::Plain(format!("In pausa: {track}").into())
    } else {
        Status::Live(format!("Sta ascoltando {track}").into())
    }
}

/// Lo stato di una persona nelle righe: il brano se ascolta, altrimenti `@nome`.
pub fn status(handle: &str, now: Option<&NowPlaying>) -> Status {
    now.map_or_else(|| Status::Plain(at(handle)), listening)
}

/// `@nome`.
pub fn at(handle: &str) -> SharedString {
    format!("@{handle}").into()
}

/// «1 amico», «12 amici».
pub fn friends(count: u32) -> String {
    if count == 1 { "1 amico".to_owned() } else { format!("{count} amici") }
}

/// «1 ascolto», «42 ascolti».
pub fn plays(count: u32) -> String {
    if count == 1 { "1 ascolto".to_owned() } else { format!("{count} ascolti") }
}

#[cfg(test)]
mod tests {
    use psytify_core::api::models::fixtures;

    use super::*;

    #[test]
    fn a_handle_keeps_only_what_the_server_accepts() {
        assert_eq!(handle_chars("Luca.Bianchi_99!"), "lucabianchi_99");
        assert_eq!(handle_chars("àé"), "");
        assert_eq!(line_chars("Luca\nBianchi\t"), "LucaBianchi");
        assert_eq!(code_chars(" 12-34 56 "), "123456");
    }

    #[test]
    fn listening_is_live_only_while_it_plays() {
        let track = fixtures::track(1);
        let line = format!("{} · {}", track.title, track.artist.name);
        let playing = NowPlaying { track: track.clone(), position_ms: 0, paused: false };
        assert_eq!(listening(&playing), Status::Live(format!("Sta ascoltando {line}").into()));
        let paused = NowPlaying { paused: true, ..playing };
        assert_eq!(listening(&paused), Status::Plain(format!("In pausa: {line}").into()));
        assert_eq!(status("luca", None), Status::Plain("@luca".into()));
    }

    #[test]
    fn counts_agree_with_their_number() {
        assert_eq!(friends(1), "1 amico");
        assert_eq!(friends(0), "0 amici");
        assert_eq!(plays(1), "1 ascolto");
        assert_eq!(plays(42), "42 ascolti");
    }
}
