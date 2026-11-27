//! I messaggi dei gesti del telefono (`ui::gesture`) e il poco stato che serve: il foglio del
//! player a schermo intero (entra, si trascina, esce), le azioni a scorrimento delle righe e il
//! riordino della coda. Il movimento vero sta nei widget (`ui/gesture/*`): qui arrivano solo gli
//! esiti, tre messaggi per gesto del foglio e uno per azione.

use std::sync::Arc;

use iced::Task;
use iced::widget::scrollable::{self, AbsoluteOffset};
use psytify_core::command::PlaybackCmd;

use super::{App, Message, Screen, phone_tab_of};
use crate::api::Track;
use crate::ui::gesture::memory;
use crate::ui::phone::{page_key, queue::QUEUE_SCROLL};

/// Lo stato dei gesti del telefono.
#[derive(Debug, Default)]
pub struct PhoneState {
    /// La pagina sta sotto il foglio del player (entra, è trascinato, esce): si disegna anche lei.
    pub underlay: bool,
    /// Il foglio deve uscire con la sua animazione (tasto «giù» del player).
    pub dismiss: bool,
}

#[derive(Debug, Clone)]
pub enum PhoneMsg {
    /// Il mini-player: un tocco o uno scorrimento in su apre il player col movimento del foglio.
    OpenPlayer,
    /// Il tasto «giù» del player: il foglio esce con l'animazione.
    DismissPlayer,
    /// Il dito ha preso il foglio: serve la pagina sotto.
    SheetDragging,
    /// Il foglio è del tutto aperto: la pagina sotto non serve più.
    SheetSettled,
    /// Il foglio è uscito: il player si chiude.
    SheetClosed,
    /// Scorrimento a destra su una riga: la aggiunge alla coda.
    EnqueueAt(Arc<[Track]>, usize),
    /// Scorrimento a sinistra su una riga: mi piace o non mi piace più.
    LikeAt(Arc<[Track]>, usize),
    /// Riordino della coda: da una posizione dell'ordine di ascolto a un'altra.
    QueueMove(usize, usize),
    /// Scorrimento automatico della coda mentre una riga è trascinata ai bordi (dp).
    QueueScroll(f32),
}

impl App {
    pub(super) fn handle_phone(&mut self, message: PhoneMsg) -> Task<Message> {
        match message {
            PhoneMsg::OpenPlayer => {
                if self.focus || self.queue.current().is_none() {
                    return Task::none();
                }
                self.phone.underlay = true;
                self.phone.dismiss = false;
                self.handle(Message::ToggleFocus)
            }
            PhoneMsg::DismissPlayer => {
                // Dal foglio aperto: se la pagina sotto manca, il foglio la richiede da sé.
                self.phone.underlay = true;
                self.phone.dismiss = true;
                Task::none()
            }
            PhoneMsg::SheetDragging => {
                self.phone.underlay = true;
                Task::none()
            }
            PhoneMsg::SheetSettled => {
                self.phone.underlay = false;
                self.phone.dismiss = false;
                Task::none()
            }
            PhoneMsg::SheetClosed => {
                self.phone.underlay = false;
                self.phone.dismiss = false;
                if self.focus { self.handle(Message::ToggleFocus) } else { Task::none() }
            }
            PhoneMsg::EnqueueAt(tracks, index) => match tracks.get(index).cloned() {
                Some(track) => self.handle(Message::Enqueue(track)),
                None => Task::none(),
            },
            PhoneMsg::LikeAt(tracks, index) => match tracks.get(index).cloned() {
                Some(track) => self.handle(Message::ToggleLike(track)),
                None => Task::none(),
            },
            PhoneMsg::QueueMove(from, to) => self.send_playback(PlaybackCmd::Move { from, to }),
            PhoneMsg::QueueScroll(dy) => {
                scrollable::scroll_by(scrollable::Id::new(QUEUE_SCROLL), AbsoluteOffset { x: 0.0, y: dy })
            }
        }
    }

    /// Cambia la schermata: una pagina aperta in avanti (non tornando indietro) riparte dall'alto;
    /// le schede e le pagine a cui si torna riprendono dov'erano (`ui::gesture::memory`).
    pub(super) fn phone_page_changed(&mut self, from: Screen, to: Screen) {
        let went_back = self.forward.last() == Some(&from);
        let root = phone_tab_of(to) == Some(to);
        if !went_back && !root {
            memory::forget(page_key(to));
        }
    }
}
