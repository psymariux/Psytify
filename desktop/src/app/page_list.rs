//! La lista di una pagina d'oggetto (album, artista, playlist): la testa (`PageHeader`) è la voce 0 e
//! scorre con le righe, che vengono dopo. È la `list` di GPUI: costruisce solo le voci visibili (più un
//! margine) e ne ricorda l'altezza, così una testa di 260 dp e righe da 48 dp stanno nella stessa lista.
//! La testa non si rimisura finché non lo si chiede ([`remeasure_header`]); le righe cambiate si
//! annunciano con [`set_rows`], che tiene il punto di scorrimento.

use gpui::{ListAlignment, ListOffset, ListState, px};
use psytify_design::tokens::size::TRACK_ROW_H;

/// Quanto si costruisce oltre il bordo visibile: quattro righe, per non vedere comparire le voci.
const OVERDRAW: f32 = 4.0 * TRACK_ROW_H;

/// Una lista con la testa e `rows` righe.
pub fn new_list(rows: usize) -> ListState {
    ListState::new(1 + rows, ListAlignment::Top, px(OVERDRAW))
}

/// Le righe sono cambiate (ora sono `rows`): la testa resta misurata, lo scorrimento resta dov'è.
pub fn set_rows(list: &ListState, rows: usize) {
    let old = list.item_count();
    if old == 0 {
        list.splice(0..0, 1 + rows);
    } else {
        // `splice` riporta all'inizio del tratto sostituito un punto di scorrimento che vi cadeva dentro:
        // si rimette la stessa voce allo stesso scarto (o la fine, se le righe sono meno).
        let top = list.logical_scroll_top();
        list.splice(1..old, rows);
        if top.item_ix > 0 {
            list.scroll_to(top);
        }
    }
}

/// La testa ha cambiato contenuto (e forse altezza).
pub fn remeasure_header(list: &ListState) {
    if list.item_count() > 0 {
        list.remeasure_items(0..1);
    }
}

/// Torna in cima (un'altra pagina nella stessa vista).
pub fn to_top(list: &ListState) {
    list.scroll_to(ListOffset { item_ix: 0, offset_in_item: px(0.0) });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rows_keep_the_scroll_position() {
        let list = new_list(0);
        set_rows(&list, 20);
        list.scroll_to(ListOffset { item_ix: 8, offset_in_item: px(7.0) });
        set_rows(&list, 30);
        let top = list.logical_scroll_top();
        assert_eq!((top.item_ix, top.offset_in_item), (8, px(7.0)));
        // Meno righe di prima: si resta in fondo, non si torna in cima.
        set_rows(&list, 3);
        assert_eq!(list.logical_scroll_top().item_ix, 4);
        // In cima si resta in cima.
        to_top(&list);
        set_rows(&list, 10);
        assert_eq!(list.logical_scroll_top().item_ix, 0);
    }
}
