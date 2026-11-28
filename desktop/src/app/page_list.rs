//! The list of an object page (album, artist, playlist): the head (`PageHeader`) is item 0 and
//! scrolls with the rows, which come after. It is GPUI's `list`: it builds only the visible items (plus a
//! margin) and remembers their height, so a 260 dp head and 48 dp rows fit in the same list.
//! The head is not measured again until asked ([`remeasure_header`]); changed rows are
//! announced with [`set_rows`], which keeps the scroll point.

use gpui::{ListAlignment, ListOffset, ListState, px};
use psytify_design::tokens::size::TRACK_ROW_H;

/// How far it builds beyond the visible edge: four rows, so entries do not pop in.
const OVERDRAW: f32 = 4.0 * TRACK_ROW_H;

/// A list with the head and `rows` rows.
pub fn new_list(rows: usize) -> ListState {
    ListState::new(1 + rows, ListAlignment::Top, crate::theme::dp(OVERDRAW))
}

/// The rows changed (now they are `rows`): the head stays measured, the scroll stays where it is.
pub fn set_rows(list: &ListState, rows: usize) {
    let old = list.item_count();
    if old == 0 {
        list.splice(0..0, 1 + rows);
    } else {
        // `splice` brings back to the start of the replaced stretch a scroll point that fell inside it:
        // the same entry is put back at the same offset (or the end, if there are fewer rows).
        let top = list.logical_scroll_top();
        list.splice(1..old, rows);
        if top.item_ix > 0 {
            list.scroll_to(top);
        }
    }
}

/// The head changed content (and perhaps height).
pub fn remeasure_header(list: &ListState) {
    if list.item_count() > 0 {
        list.remeasure_items(0..1);
    }
}

/// Back to the top (another page in the same view).
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
        // Fewer rows than before: stay at the bottom, do not go back to the top.
        set_rows(&list, 3);
        assert_eq!(list.logical_scroll_top().item_ix, 4);
        // At the top, stay at the top.
        to_top(&list);
        set_rows(&list, 10);
        assert_eq!(list.logical_scroll_top().item_ix, 0);
    }
}
