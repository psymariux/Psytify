//! Dov'è il mouse nella finestra, senza un messaggio a ogni movimento.
//!
//! Il menu contestuale si apre dove si è fatto clic, ma il widget che riceve il clic dentro una
//! lista che scorre conosce la posizione nel proprio spazio, non in quello della finestra. Un
//! ascoltatore di eventi della finestra scrive la posizione in un'atomica e restituisce `None`:
//! non nasce nessun messaggio, quindi nessuna ricostruzione della vista. Chi gestisce il clic
//! destro legge l'ultima posizione.

use std::sync::atomic::{AtomicU32, Ordering};

use iced::{Event, Point, event, mouse, window};

use crate::app::Message;

static X: AtomicU32 = AtomicU32::new(0);
static Y: AtomicU32 = AtomicU32::new(0);

/// Da passare a `event::listen_with`: ricorda il mouse e non produce messaggi.
pub fn track(event: Event, _: event::Status, _: window::Id) -> Option<Message> {
    if let Event::Mouse(mouse::Event::CursorMoved { position }) = event {
        X.store(position.x.to_bits(), Ordering::Relaxed);
        Y.store(position.y.to_bits(), Ordering::Relaxed);
    }
    None
}

/// L'ultima posizione nota del mouse, in coordinate della finestra.
pub fn position() -> Point {
    Point::new(f32::from_bits(X.load(Ordering::Relaxed)), f32::from_bits(Y.load(Ordering::Relaxed)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_cursor_position_is_remembered_without_a_message() {
        let moved = Event::Mouse(mouse::Event::CursorMoved { position: Point::new(120.5, 64.25) });
        assert!(track(moved, event::Status::Ignored, window::Id::unique()).is_none());
        assert_eq!(position(), Point::new(120.5, 64.25));
    }
}
