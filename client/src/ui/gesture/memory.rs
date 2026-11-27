//! La memoria dello scorrimento: tornando a una scheda o a una pagina da cui si è tornati indietro,
//! la lista riparte da dov'era.
//!
//! `remember` avvolge lo `scrollable` della pagina. Dopo ogni rotella (e ogni rilascio, per la barra
//! col mouse) legge l'offset dello `scrollable` con un'operazione di Iced e lo scrive in una piccola
//! tabella dietro un `Mutex` (massimo `CAPACITY` pagine, la meno recente esce): niente messaggi,
//! niente stato in `App`, niente lavoro mentre non si scorre. Quando il widget nasce con un offset
//! da ripristinare lo applica al primo ridisegno in cui il contenuto è abbastanza alto (le pagine
//! caricano i dati dopo) e smette se la persona scorre o dopo `GIVE_UP`.
//!
//! La chiave della pagina la sceglie il chiamante (`key_of`).

use std::hash::{Hash, Hasher};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::operation::{Operation, Outcome, Scrollable};
use iced::advanced::widget::{Id, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay, renderer};
use iced::event::{self, Event};
use iced::widget::scrollable::AbsoluteOffset;
use iced::{Element, Length, Rectangle, Size, Vector, mouse, window};

/// Quante pagine si ricordano.
const CAPACITY: usize = 32;
/// Dopo quanto si rinuncia a ripristinare (il contenuto non è mai diventato abbastanza alto).
const GIVE_UP: Duration = Duration::from_secs(3);
/// Sotto questo offset (dp) non vale la pena ricordare: la pagina è in cima.
const MIN: f32 = 1.0;

static TABLE: Mutex<Vec<(u64, f32)>> = Mutex::new(Vec::new());

/// L'offset ricordato per `key`.
pub fn recall(key: u64) -> Option<f32> {
    let table = TABLE.lock().unwrap_or_else(PoisonError::into_inner);
    table.iter().find(|(k, _)| *k == key).map(|(_, y)| *y)
}

/// Ricorda l'offset di `key`. Un offset in cima cancella la voce.
pub fn store(key: u64, offset: f32) {
    put(&mut TABLE.lock().unwrap_or_else(PoisonError::into_inner), key, offset);
}

fn put(table: &mut Vec<(u64, f32)>, key: u64, offset: f32) {
    table.retain(|(k, _)| *k != key);
    if offset >= MIN {
        table.push((key, offset));
        if table.len() > CAPACITY {
            table.remove(0);
        }
    }
}

/// Dimentica la pagina `key` (una pagina aperta di nuovo parte dall'alto).
pub fn forget(key: u64) {
    store(key, 0.0);
}

/// Una chiave per una pagina: un'etichetta e, se serve, un identificatore.
pub fn key_of(label: &str, id: Option<u128>) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    label.hash(&mut hasher);
    id.hash(&mut hasher);
    hasher.finish()
}

/// Legge l'offset del primo `scrollable` che incontra (quello della pagina) senza scendere oltre.
struct Read {
    offset: Option<f32>,
    /// Se c'è, porta lo `scrollable` a questo offset appena il contenuto è alto abbastanza.
    apply: Option<f32>,
    applied: bool,
}

impl Operation for Read {
    fn container(
        &mut self,
        _id: Option<&Id>,
        _bounds: Rectangle,
        _children: &mut dyn FnMut(&mut dyn Operation<()>),
    ) {
        // Si ferma al primo livello: gli scaffali orizzontali dentro la pagina non interessano.
    }

    fn scrollable(
        &mut self,
        state: &mut dyn Scrollable,
        _id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
    ) {
        if self.offset.is_some() {
            return;
        }
        if let Some(target) = self.apply {
            let reachable = (content_bounds.height - bounds.height).max(0.0);
            if reachable + 0.5 >= target {
                state.scroll_to(AbsoluteOffset { x: 0.0, y: target });
                self.applied = true;
                self.offset = Some(target);
                return;
            }
        }
        self.offset = Some(translation.y);
    }

    fn finish(&self) -> Outcome<()> {
        Outcome::None
    }
}

pub struct Remember<'a, Message> {
    key: u64,
    restore: bool,
    content: Element<'a, Message>,
}

/// Avvolge lo `scrollable` di una pagina: ne ricorda l'offset sotto `key` e, se `restore`, lo
/// ripristina.
pub fn remember<'a, Message>(
    key: u64,
    restore: bool,
    content: impl Into<Element<'a, Message>>,
) -> Remember<'a, Message> {
    Remember { key, restore, content: content.into() }
}

#[derive(Debug)]
struct State {
    /// Offset da ripristinare, finché non ci si riesce o si rinuncia.
    pending: Option<f32>,
    born: Instant,
}

impl<'a, Message: 'a> Widget<Message, iced::Theme, iced::Renderer> for Remember<'a, Message> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        let pending = if self.restore { recall(self.key) } else { None };
        tree::State::new(State { pending, born: Instant::now() })
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(&self, tree: &mut Tree, renderer: &iced::Renderer, limits: &layout::Limits) -> layout::Node {
        self.content.as_widget().layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.content.as_widget().operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn on_event(
        &mut self,
        tree: &mut Tree,
        event: Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) -> event::Status {
        let scrolled = matches!(
            event,
            Event::Mouse(
                mouse::Event::WheelScrolled { .. } | mouse::Event::ButtonReleased(mouse::Button::Left)
            )
        );
        let redraw = matches!(event, Event::Window(window::Event::RedrawRequested(_)));
        let status = self.content.as_widget_mut().on_event(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
        );

        let state = tree.state.downcast_mut::<State>();
        if scrolled && state.pending.is_some() {
            // La persona ha scorso: non si ripristina più.
            state.pending = None;
        }
        if redraw && let Some(target) = state.pending {
            if state.born.elapsed() > GIVE_UP {
                state.pending = None;
            } else {
                let mut read = Read { offset: None, apply: Some(target), applied: false };
                self.content.as_widget().operate(&mut tree.children[0], layout, renderer, &mut read);
                let state = tree.state.downcast_mut::<State>();
                if read.applied {
                    state.pending = None;
                    shell.invalidate_layout();
                    shell.request_redraw(window::RedrawRequest::NextFrame);
                } else {
                    // Il contenuto non è ancora alto abbastanza: si riprova fra poco.
                    shell.request_redraw(window::RedrawRequest::At(
                        Instant::now() + Duration::from_millis(100),
                    ));
                }
            }
        }
        if scrolled {
            let mut read = Read { offset: None, apply: None, applied: false };
            self.content.as_widget().operate(&mut tree.children[0], layout, renderer, &mut read);
            if let Some(offset) = read.offset {
                store(self.key, offset);
            }
        }
        status
    }

    fn draw(
        &self,
        tree: &tree::Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(&tree.children[0], renderer, theme, style, layout, cursor, viewport);
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(&tree.children[0], layout, cursor, viewport, renderer)
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        self.content.as_widget_mut().overlay(&mut tree.children[0], layout, renderer, translation)
    }
}

impl<'a, Message: 'a> From<Remember<'a, Message>> for Element<'a, Message> {
    fn from(remember: Remember<'a, Message>) -> Self {
        Element::new(remember)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // La tabella è globale: ogni test usa chiavi proprie.

    #[test]
    fn a_stored_offset_is_recalled_and_zero_forgets_it() {
        let key = key_of("test-store", Some(1));
        assert_eq!(recall(key), None);
        store(key, 420.0);
        assert_eq!(recall(key), Some(420.0));
        store(key, 0.0);
        assert_eq!(recall(key), None);
    }

    #[test]
    fn forgetting_resets_a_page() {
        let key = key_of("test-forget", None);
        store(key, 99.0);
        forget(key);
        assert_eq!(recall(key), None);
    }

    #[test]
    fn keys_differ_by_label_and_id() {
        assert_ne!(key_of("album", Some(1)), key_of("album", Some(2)));
        assert_ne!(key_of("album", Some(1)), key_of("artist", Some(1)));
        assert_eq!(key_of("album", Some(1)), key_of("album", Some(1)));
    }

    #[test]
    fn the_table_is_bounded_and_drops_the_oldest() {
        let mut table = Vec::new();
        for i in 0..(CAPACITY as u64 + 8) {
            put(&mut table, i + 1, 10.0 + i as f32);
        }
        assert_eq!(table.len(), CAPACITY);
        assert!(!table.iter().any(|(k, _)| *k == 1));
        assert!(table.iter().any(|(k, _)| *k == CAPACITY as u64 + 8));
        // Riscrivere una pagina la porta in coda.
        put(&mut table, 9, 77.0);
        assert_eq!(table.last(), Some(&(9, 77.0)));
        assert_eq!(table.len(), CAPACITY);
    }
}
