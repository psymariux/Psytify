//! I messaggi della jam (l'ascolto insieme), smistati da `App::handle`. Dal passo 4.8 la jam sta nel
//! core (`core/src/state/jam.rs`): qui i messaggi diventano `JamCmd` e l'app legge la copia della sua
//! vista (`App::jam`), che [`App::jam_published`] rinnova a ogni pubblicazione. Il collegamento con
//! la stanza, i membri, l'allineamento alla coda del gruppo e lo stato locale mandato alla stanza
//! non sono più qui; restano i campi di testo della schermata.

use std::sync::Arc;

use iced::Task;

use psytify_core::command::{Command, JamCmd};
use psytify_core::snapshot::JamView;

use super::{App, Message, Notice, Screen};
use crate::jam;

impl App {
    /// I messaggi di questo dominio, smistati da `App::handle`.
    pub(super) fn handle_jam(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::JamName(name) => {
                self.jam_name = name.chars().take(jam::MAX_NAME_CHARS).collect();
                Task::none()
            }
            Message::JamCode(code) => {
                self.jam_code = code
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .take(jam::CODE_LEN)
                    .map(|c| c.to_ascii_uppercase())
                    .collect();
                Task::none()
            }
            Message::JamCreate => {
                self.send_core(Command::Jam(JamCmd::Create { name: self.jam_name.clone() }))
            }
            Message::JamJoin => self.join_jam(self.jam_code.clone()),
            Message::JamLeave => self.send_core(Command::Jam(JamCmd::Leave)),
            Message::OpenJam => {
                self.focus = false;
                self.focus_video = false;
                self.open(Screen::Jam)
            }
            // `handle` manda qui solo i messaggi di questo dominio.
            _ => Task::none(),
        }
    }

    /// Entra nella jam con quel codice (un codice che non ha sei lettere o cifre lo rifiuta il core,
    /// con il suo avviso). Il nome è quello scritto nella schermata.
    pub(super) fn join_jam(&mut self, code: String) -> Task<Message> {
        self.send_core(Command::Jam(JamCmd::Join { code, name: self.jam_name.clone() }))
    }

    /// Copia nell'app la vista della jam dell'ultima istantanea, senza mostrare niente: lo stato
    /// iniziale. Gli eventi già nella vista non si rimostrano.
    pub(super) fn copy_jam(&mut self) {
        self.jam = Arc::clone(&self.core_snapshot.jam);
        self.jam_event = self.jam.events.back().map_or(0, |raised| raised.seq);
    }

    /// La jam è cambiata: si tiene la vista nuova, si dice ciò che gli eventi dicono (in ordine) e,
    /// uscendo dalla stanza (da soli o perché si è chiusa), si dimentica la scelta di chi invitare.
    pub(super) fn jam_published(&mut self, previous: &JamView) {
        let view = Arc::clone(&self.core_snapshot.jam);
        self.jam = Arc::clone(&view);
        let shown = self.jam_event;
        for raised in view.events.iter().filter(|raised| raised.seq > shown) {
            self.jam_event = raised.seq;
            let text = raised.event.message();
            self.notice =
                Some(if raised.event.is_error() { Notice::Error(text) } else { Notice::Info(text) });
        }
        if previous.is_active() && !view.is_active() {
            self.social.invitees.clear();
        }
    }
}
