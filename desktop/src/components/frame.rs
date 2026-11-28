//! The frame (desktop v2 §2): the canvas fills the window between the sidebar and the side panel, with a
//! 32 dp margin each side (24 dp under 1200 dp of window). Two reusable layouts on it:
//!
//! - [`TileGrid`]: tile rows whose column count comes from the width and whose tiles stretch to fill the
//!   row exactly (`size::tile_grid`, `TILE_MIN` 168 and `TILE_GAP` 24);
//! - [`Flow`]: row lists that are not one ordered list flow into columns of at least `FLOW_COL_MIN`, one to
//!   three, filled top to bottom then left to right ([`Flow::cells`]).
//!
//! Pages measure their width once per draw (a `canvas` prepaint), keep a [`Frame`] and rebuild their rows
//! only when a column count changes; the view itself computes nothing.

use gpui::{AnyElement, Div, IntoElement, ParentElement, Styled, div};
use psytify_design::tokens::size::{
    FLOW_GAP, MEASURE_CH, TILE_GAP, TILE_MIN, flow_columns, frame_margin, tile_grid,
};
use psytify_design::tokens::type_::TypeRole;

use crate::theme::dp;

/// Inter's average advance is about half its size: 72 characters of a role.
const CH_PER_EM: f32 = 0.5;

/// The measure of running text in `role` (`MEASURE_CH` characters), in logical px.
pub fn measure(role: TypeRole) -> gpui::Pixels {
    dp(MEASURE_CH as f32 * role.size * CH_PER_EM)
}

/// Shortcut grids take two to four columns of at least this width.
const SHORTCUT_MIN: f32 = 280.0;

/// The layout of a page for its width, in dp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// Canvas margin each side.
    pub margin: f32,
    /// The width between the margins.
    pub inner: f32,
    pub grid: TileGrid,
    pub flow: Flow,
    /// Columns of a shortcut grid (2 to 4).
    pub shortcuts: usize,
}

impl Frame {
    /// For a canvas `canvas_w` dp wide in a window `window_w` dp wide.
    pub fn of(canvas_w: f32, window_w: f32) -> Self {
        let margin = frame_margin(window_w);
        let inner = (canvas_w - 2.0 * margin).max(TILE_MIN);
        let fits = ((inner + FLOW_GAP) / (SHORTCUT_MIN + FLOW_GAP)).floor();
        // A few columns: the truncation is the point.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let shortcuts = if fits.is_finite() { (fits as usize).clamp(2, 4) } else { 2 };
        Self { margin, inner, grid: TileGrid::fit(inner), flow: Flow::fit(inner), shortcuts }
    }

    /// The column counts: when they change, a page rebuilds its rows (a tile size change only redraws).
    pub fn columns(&self) -> (usize, usize, usize) {
        (self.grid.columns, self.flow.columns, self.shortcuts)
    }
}

impl Default for Frame {
    fn default() -> Self {
        Self::of(1024.0, 1280.0)
    }
}

/// A fluid tile grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TileGrid {
    pub columns: usize,
    /// The tile side in dp.
    pub tile: f32,
}

impl TileGrid {
    pub fn fit(inner: f32) -> Self {
        let (columns, tile) = tile_grid(inner, TILE_MIN, TILE_GAP);
        Self { columns, tile }
    }

    /// One row of tiles between the margins.
    pub fn row(margin: f32, tiles: impl IntoIterator<Item = AnyElement>) -> Div {
        div().w_full().flex().gap(dp(TILE_GAP)).px(dp(margin)).children(tiles)
    }
}

/// Flowing columns for a row list.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flow {
    pub columns: usize,
}

impl Flow {
    pub fn fit(inner: f32) -> Self {
        Self { columns: flow_columns(inner) }
    }

    /// Rows needed for `count` items.
    pub fn rows(&self, count: usize) -> usize {
        count.div_ceil(self.columns.max(1))
    }

    /// The items of row `row` out of `count`, column by column: a column is filled top to bottom before the
    /// next one starts, so the list reads down like a page.
    pub fn cells(&self, row: usize, count: usize) -> impl Iterator<Item = usize> {
        let rows = self.rows(count);
        (0..self.columns.max(1)).map(move |col| col * rows + row).filter(move |ix| *ix < count && row < rows)
    }

    /// One row of cells: equal columns between the margins. Short rows keep the column width.
    pub fn row(&self, margin: f32, cells: impl IntoIterator<Item = AnyElement>) -> Div {
        let mut cells: Vec<AnyElement> = cells.into_iter().collect();
        while cells.len() < self.columns.max(1) {
            cells.push(div().into_any_element());
        }
        div()
            .w_full()
            .grid()
            .grid_cols(u16::try_from(self.columns.max(1)).unwrap_or(1))
            .gap_x(dp(FLOW_GAP))
            .px(dp(margin))
            .children(cells)
    }
}

/// A row of a list that flows some of its runs into columns ([`flow_lines`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Line<T> {
    /// A row across the width (a heading, a note).
    One(T),
    /// One line of a flowing run: an item per column.
    Cells(Vec<T>),
}

/// `rows` with every run of consecutive items of the same `key` flowed into lines of `flow.columns`,
/// filled column by column; rows without a key stay as they are.
pub fn flow_lines<T: Clone, K: PartialEq>(
    rows: impl IntoIterator<Item = T>,
    flow: Flow,
    key: impl Fn(&T) -> Option<K>,
) -> Vec<Line<T>> {
    let mut out = Vec::new();
    let mut run: Vec<T> = Vec::new();
    let mut run_key: Option<K> = None;
    let flush = |run: &mut Vec<T>, out: &mut Vec<Line<T>>| {
        let count = run.len();
        for line in 0..flow.rows(count) {
            out.push(Line::Cells(flow.cells(line, count).filter_map(|ix| run.get(ix).cloned()).collect()));
        }
        run.clear();
    };
    for row in rows {
        match key(&row) {
            Some(k) => {
                if run_key.as_ref() != Some(&k) {
                    flush(&mut run, &mut out);
                    run_key = Some(k);
                }
                run.push(row);
            }
            None => {
                flush(&mut run, &mut out);
                run_key = None;
                out.push(Line::One(row));
            }
        }
    }
    flush(&mut run, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_frame_fills_the_canvas_with_its_margins() {
        let wide = Frame::of(2560.0 - 264.0, 2560.0);
        assert_eq!(wide.margin, 32.0);
        assert_eq!(wide.inner, 2560.0 - 264.0 - 64.0);
        assert_eq!(wide.flow.columns, 3);
        assert_eq!(wide.shortcuts, 4);
        let narrow = Frame::of(1000.0 - 72.0, 1000.0);
        assert_eq!(narrow.margin, 24.0);
        assert_eq!(narrow.flow.columns, 1);
        assert_eq!(narrow.shortcuts, 2);
        // The tiles fill the row: columns times the tile plus the gaps is the inner width.
        let grid = wide.grid;
        let used = grid.columns as f32 * grid.tile + (grid.columns as f32 - 1.0) * TILE_GAP;
        assert!((used - wide.inner).abs() < 1e-2);
    }

    #[test]
    fn runs_flow_into_lines_and_other_rows_stay() {
        // 'h' is a heading; digits are items keyed by their parity group ('a' for 1..=3, 'b' for 7..=8).
        let rows = ['h', '1', '2', '3', 'h', '7', '8'];
        let key = |c: &char| match c {
            '1'..='3' => Some('a'),
            '7' | '8' => Some('b'),
            _ => None,
        };
        let lines = flow_lines(rows, Flow { columns: 2 }, key);
        assert_eq!(
            lines,
            vec![
                Line::One('h'),
                Line::Cells(vec!['1', '3']),
                Line::Cells(vec!['2']),
                Line::One('h'),
                Line::Cells(vec!['7', '8']),
            ]
        );
        assert_eq!(flow_lines(['1', '2'], Flow { columns: 1 }, key).len(), 2);
    }

    #[test]
    fn flowing_columns_fill_top_to_bottom_first() {
        let flow = Flow { columns: 3 };
        assert_eq!(flow.rows(7), 3);
        assert_eq!(flow.cells(0, 7).collect::<Vec<_>>(), vec![0, 3, 6]);
        assert_eq!(flow.cells(2, 7).collect::<Vec<_>>(), vec![2, 5]);
        assert_eq!(flow.cells(3, 7).count(), 0);
        assert_eq!(Flow { columns: 1 }.cells(4, 7).collect::<Vec<_>>(), vec![4]);
        assert_eq!(flow.cells(0, 2).collect::<Vec<_>>(), vec![0, 1], "fewer items than columns: one row");
    }
}
