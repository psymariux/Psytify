//! Shapes on the art-pixel grid: notched corners (`NOTCH_1`, `NOTCH_2`), 1 AP border, hard ledge
//! and focus ring. Everything is made of solid rectangles (no radius, no curves): each
//! bordo cade su un pixel fisico intero, a qualunque scala.
//!
//! Measures are computed in physical pixels (`Ap::device` per AP) and converted back to logical ones only
//! at the end, so the geometry does not accumulate floating-point errors.
//!
//! A notch of `n` AP takes from the corner the staircase of `n` rows: row `r` (from 0) loses `n - r` AP per
//! side. With a 1 AP border the inner fill has notch `n - 1`: for `NOTCH_1` the border covers
//! `(1,0)` and `(0,1)` and the fill starts at `(1,1)`.

use gpui::{App, Bounds, Canvas, FocusHandle, Hsla, Pixels, Styled, Window, canvas, fill, point, px, size};

use crate::theme::Ap;

/// Rettangolo in pixel fisici.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Dev {
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

impl Dev {
    fn of(bounds: Bounds<Pixels>, scale: f32) -> Self {
        let x0 = (f32::from(bounds.origin.x) * scale).round();
        let y0 = (f32::from(bounds.origin.y) * scale).round();
        let x1 = ((f32::from(bounds.origin.x) + f32::from(bounds.size.width)) * scale).round();
        let y1 = ((f32::from(bounds.origin.y) + f32::from(bounds.size.height)) * scale).round();
        Self { x: x0, y: y0, w: (x1 - x0).max(0.0), h: (y1 - y0).max(0.0) }
    }

    fn paint(self, window: &mut Window, scale: f32, color: Hsla) {
        if self.w <= 0.0 || self.h <= 0.0 || color.a <= 0.0 {
            return;
        }
        let bounds = Bounds::new(
            point(px(self.x / scale), px(self.y / scale)),
            size(px(self.w / scale), px(self.h / scale)),
        );
        window.paint_quad(fill(bounds, color));
    }
}

/// Horizontal segments (x0, x1) per band of rows (y0, y1) of an octagon with notch `n`.
fn octagon_rows(r: Dev, n: u32, ap: f32) -> Vec<(f32, f32, f32, f32)> {
    let n = n as f32;
    // The notch cannot eat more than half a side.
    let cut = |steps: f32| steps.min((r.w / ap / 2.0).floor()).min((r.h / ap / 2.0).floor());
    let n = cut(n);
    let mut rows = Vec::with_capacity(2 * n as usize + 1);
    for i in 0..n as u32 {
        let inset = (n - i as f32) * ap;
        rows.push((r.y + i as f32 * ap, r.y + (i as f32 + 1.0) * ap, r.x + inset, r.x + r.w - inset));
    }
    rows.push((r.y + n * ap, r.y + r.h - n * ap, r.x, r.x + r.w));
    for i in (0..n as u32).rev() {
        let inset = (n - i as f32) * ap;
        rows.push((
            r.y + r.h - (i as f32 + 1.0) * ap,
            r.y + r.h - i as f32 * ap,
            r.x + inset,
            r.x + r.w - inset,
        ));
    }
    rows
}

fn fill_octagon(window: &mut Window, scale: f32, r: Dev, n: u32, ap: f32, color: Hsla) {
    for (y0, y1, x0, x1) in octagon_rows(r, n, ap) {
        Dev { x: x0, y: y0, w: x1 - x0, h: y1 - y0 }.paint(window, scale, color);
    }
}

/// Riempie la forma a tacca `n` dentro `bounds`.
pub fn paint_notch_fill(window: &mut Window, bounds: Bounds<Pixels>, n: u32, ap: Ap, color: Hsla) {
    let r = Dev::of(bounds, ap.scale);
    fill_octagon(window, ap.scale, r, n, ap.device as f32, color);
}

/// 1 AP outline that follows the notch `n` (inside `bounds`), without fill.
pub fn paint_notch_ring(window: &mut Window, bounds: Bounds<Pixels>, n: u32, ap: Ap, color: Hsla) {
    let scale = ap.scale;
    let d = ap.device as f32;
    let r = Dev::of(bounds, scale);
    let max_n = ((r.w.min(r.h) / d / 2.0).floor() as u32).max(1);
    let n = n.min(max_n);
    let m = n.max(1) as f32;
    // Barre: sopra e sotto fra le tacche, sinistra e destra fra le tacche.
    let top_x = r.x + n as f32 * d;
    let top_w = r.w - 2.0 * n as f32 * d;
    Dev { x: top_x, y: r.y, w: top_w, h: d }.paint(window, scale, color);
    Dev { x: top_x, y: r.y + r.h - d, w: top_w, h: d }.paint(window, scale, color);
    let side_h = r.h - 2.0 * m * d;
    Dev { x: r.x, y: r.y + m * d, w: d, h: side_h }.paint(window, scale, color);
    Dev { x: r.x + r.w - d, y: r.y + m * d, w: d, h: side_h }.paint(window, scale, color);
    // Points of the diagonal (n >= 2): row i, column n - i, in the four corners.
    for i in 1..n {
        let off = (n - i) as f32 * d;
        let row = i as f32 * d;
        for (x, y) in [
            (r.x + off, r.y + row),
            (r.x + r.w - off - d, r.y + row),
            (r.x + off, r.y + r.h - row - d),
            (r.x + r.w - off - d, r.y + r.h - row - d),
        ] {
            Dev { x, y, w: d, h: d }.paint(window, scale, color);
        }
    }
}

/// Fill and 1 AP border together (either one may be missing).
pub fn paint_notch_box(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    n: u32,
    ap: Ap,
    fill_color: Option<Hsla>,
    border: Option<Hsla>,
) {
    let scale = ap.scale;
    let d = ap.device as f32;
    let r = Dev::of(bounds, scale);
    match (fill_color, border) {
        (Some(fill_color), Some(border)) => {
            fill_octagon(window, scale, r, n, d, border);
            let inner = Dev { x: r.x + d, y: r.y + d, w: r.w - 2.0 * d, h: r.h - 2.0 * d };
            fill_octagon(window, scale, inner, n.saturating_sub(1), d, fill_color);
        }
        (Some(fill_color), None) => fill_octagon(window, scale, r, n, d, fill_color),
        (None, Some(border)) => paint_notch_ring(window, bounds, n, ap, border),
        (None, None) => {}
    }
}

/// Hard ledge: the shape moved by (1, 1) AP, but only the part the body does not cover, so a
/// body without fill (secondary button) does not let the ledge show inside itself.
pub fn paint_ledge(window: &mut Window, bounds: Bounds<Pixels>, n: u32, ap: Ap, color: Hsla) {
    paint_offset_shadow(window, bounds, n, ap, 1, color);
}

/// Hard shadow (blur 0) moved by `k` AP down and to the right: the buttons' ledge (k = 1)
/// e l'ombra di menu, dialoghi e toast (`SHADOW_FLOAT_OFFSET`, k = 2).
pub fn paint_offset_shadow(window: &mut Window, bounds: Bounds<Pixels>, n: u32, ap: Ap, k: u32, color: Hsla) {
    let scale = ap.scale;
    let d = ap.device as f32;
    let body = Dev::of(bounds, scale);
    let shifted = Dev { x: body.x + d * k as f32, y: body.y + d * k as f32, ..body };
    let body_rows = octagon_rows(body, n, d);
    let shifted_rows = octagon_rows(shifted, n, d);
    // Scan by bands: all the horizontal edges of both shapes.
    let mut ys: Vec<f32> =
        body_rows.iter().chain(shifted_rows.iter()).flat_map(|(y0, y1, _, _)| [*y0, *y1]).collect();
    ys.sort_by(f32::total_cmp);
    ys.dedup();
    let span_at = |rows: &[(f32, f32, f32, f32)], y: f32| {
        rows.iter().find(|(y0, y1, _, _)| y >= *y0 && y < *y1).map(|(_, _, x0, x1)| (*x0, *x1))
    };
    for pair in ys.windows(2) {
        let (ya, yb) = (pair[0], pair[1]);
        let Some((sx0, sx1)) = span_at(&shifted_rows, ya) else { continue };
        let (x0, x1) = match span_at(&body_rows, ya) {
            Some((_, bx1)) => (sx0.max(bx1), sx1),
            None => (sx0, sx1),
        };
        Dev { x: x0, y: ya, w: x1 - x0, h: yb - ya }.paint(window, scale, color);
    }
}

fn fill_parent<T: 'static>(canvas: Canvas<T>) -> Canvas<T> {
    canvas.absolute().top(px(0.0)).left(px(0.0)).size_full()
}

/// Notched body (fill and/or 1 AP border) that fills the parent (which must be `relative`).
pub fn notch_box(n: u32, fill_color: Option<Hsla>, border: Option<Hsla>) -> Canvas<()> {
    fill_parent(canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let ap = Ap::of(window);
            paint_notch_box(window, bounds, n, ap, fill_color, border);
        },
    ))
}

/// Hard ledge under the parent.
pub fn ledge(n: u32, color: Hsla) -> Canvas<()> {
    fill_parent(canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let ap = Ap::of(window);
            paint_ledge(window, bounds, n, ap, color);
        },
    ))
}

/// Hard shadow of `k` AP behind the parent (floating layers).
pub fn hard_shadow(n: u32, k: u32, color: Hsla) -> Canvas<()> {
    fill_parent(canvas(
        |_, _, _| (),
        move |bounds, (), window, _| {
            let ap = Ap::of(window);
            paint_offset_shadow(window, bounds, n, ap, k, color);
        },
    ))
}

/// "Keyboard only" focus ring: 1 AP of `color` at 1 AP distance from the parent (2 AP out
/// from its edges), with the same notch. It is painted only if `handle` has focus and the last input
/// was from the keyboard; the decision is taken at draw time, so it is always up to date.
pub fn focus_ring(n: u32, color: Hsla, handle: FocusHandle, ap: Ap) -> Canvas<()> {
    let out = ap.n(2);
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _: &mut App| {
            if !(handle.is_focused(window) && window.last_input_was_keyboard()) {
                return;
            }
            let ap = Ap::of(window);
            paint_notch_ring(window, bounds, n, ap, color);
        },
    )
    .absolute()
    .top(-out)
    .left(-out)
    .right(-out)
    .bottom(-out)
}

/// As [`focus_ring`] but active for a fixed value (gallery: shows the "focused" state without keyboard).
pub fn static_ring(n: u32, color: Hsla, ap: Ap) -> Canvas<()> {
    let out = ap.n(2);
    canvas(
        |_, _, _| (),
        move |bounds, (), window, _: &mut App| {
            let ap = Ap::of(window);
            paint_notch_ring(window, bounds, n, ap, color);
        },
    )
    .absolute()
    .top(-out)
    .left(-out)
    .right(-out)
    .bottom(-out)
}

/// Inner focus ring (rows and list entries: never clipped by a scrolling container).
/// With `force` it is always shown (gallery), otherwise only with keyboard focus.
pub fn inset_ring(color: Hsla, handle: FocusHandle, force: bool) -> Canvas<()> {
    fill_parent(canvas(
        |_, _, _| (),
        move |bounds, (), window, _: &mut App| {
            if !(force || (handle.is_focused(window) && window.last_input_was_keyboard())) {
                return;
            }
            let ap = Ap::of(window);
            paint_notch_ring(window, bounds, 0, ap, color);
        },
    ))
}

/// 50 % checkerboard dither of 1 AP squares (disabled fields, unavailable covers).
pub fn dither(color: Hsla) -> Canvas<()> {
    fill_parent(canvas(
        |_, _, _| (),
        move |bounds, (), window, _: &mut App| {
            let ap = Ap::of(window);
            let d = ap.device as f32;
            let r = Dev::of(bounds, ap.scale);
            let (cols, rows) = ((r.w / d).ceil() as u32, (r.h / d).ceil() as u32);
            for row in 0..rows {
                for c in 0..cols {
                    if (row + c) % 2 == 0 {
                        let x = r.x + c as f32 * d;
                        let y = r.y + row as f32 * d;
                        Dev { x, y, w: d.min(r.x + r.w - x), h: d.min(r.y + r.h - y) }
                            .paint(window, ap.scale, color);
                    }
                }
            }
        },
    ))
}

/// The 2 x 2 ordered-dither (Bayer) threshold of a cell: the order in which cells change in a dissolve.
pub fn bayer2(col: u32, row: u32) -> u8 {
    [[0, 2], [3, 1]][(row % 2) as usize][(col % 2) as usize]
}

/// At most this many dissolve cells along a side: a big slot uses cells of several AP.
const VEIL_CELLS: f32 = 40.0;

/// The ordered-dither veil of a dissolve: `covered` of every 4 cells (0..=4, in Bayer order) painted in
/// `color` over `bounds`. Cells are whole APs, a few AP wide on a large slot so a frame stays cheap.
pub fn paint_dither_veil(window: &mut Window, bounds: Bounds<Pixels>, ap: Ap, covered: u8, color: Hsla) {
    if covered == 0 {
        return;
    }
    let r = Dev::of(bounds, ap.scale);
    let unit = ap.device.max(1) as f32;
    let per = (r.w.max(r.h) / unit / VEIL_CELLS).ceil().max(1.0);
    let d = unit * per;
    // Whole cells over a bounded slot: the counts are small.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (cols, rows) = ((r.w / d).ceil() as u32, (r.h / d).ceil() as u32);
    for row in 0..rows {
        for c in 0..cols {
            if bayer2(c, row) < covered.min(4) {
                let x = r.x + c as f32 * d;
                let y = r.y + row as f32 * d;
                Dev { x, y, w: d.min(r.x + r.w - x), h: d.min(r.y + r.h - y) }.paint(window, ap.scale, color);
            }
        }
    }
}

/// Una fascia di celle AP: `rows` righe da `row`, `width` celle da `col`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellSpan {
    pub row: u32,
    pub rows: u32,
    pub col: u32,
    pub width: u32,
}

/// The first AP inside the circle of `cells` AP in diameter, on row `row`: a cell is inside if its
/// centre lies in the circle (`MASK_PIXEL_CIRCLE`). Always at least the central cell, so a
/// tiny circle does not vanish.
fn circle_first(cells: u32, row: u32) -> u32 {
    let r = cells as f32 / 2.0;
    let dy = row as f32 + 0.5 - r;
    let half = (r * r - dy * dy).max(0.0).sqrt();
    let first = (r - half - 0.5).ceil().max(0.0) as u32;
    first.min((cells.saturating_sub(1)) / 2)
}

/// Le fasce di un cerchio rasterizzato a risoluzione AP (`MASK_PIXEL_CIRCLE`): bordi a gradini di 1 AP,
/// symmetric on both axes. Consecutive rows of the same width join into a single band.
pub fn circle_spans(cells: u32) -> Vec<CellSpan> {
    let mut spans: Vec<CellSpan> = Vec::new();
    for row in 0..cells {
        let col = circle_first(cells, row);
        let width = cells - 2 * col;
        match spans.last_mut() {
            Some(last) if last.col == col && last.row + last.rows == row => last.rows += 1,
            _ => spans.push(CellSpan { row, rows: 1, col, width }),
        }
    }
    spans
}

/// The 1 AP outline of the circle of [`circle_spans`]: the inside cells that touch (sideways) a cell
/// outside. One or two bands per row.
pub fn circle_ring(cells: u32) -> Vec<CellSpan> {
    let first = |row: i64| -> u32 {
        if row < 0 || row >= i64::from(cells) {
            cells / 2 + cells % 2
        } else {
            circle_first(cells, row as u32)
        }
    };
    let mut ring = Vec::new();
    for row in 0..cells {
        let col = circle_first(cells, row);
        let width = cells - 2 * col;
        // The cells of this row that lie outside the neighbouring rows are edge; and the first and the last.
        let inner = first(i64::from(row) - 1).max(first(i64::from(row) + 1)).max(col + 1);
        let edge = (inner - col).min(width.div_ceil(2));
        if 2 * edge >= width {
            ring.push(CellSpan { row, rows: 1, col, width });
        } else {
            ring.push(CellSpan { row, rows: 1, col, width: edge });
            ring.push(CellSpan { row, rows: 1, col: col + width - edge, width: edge });
        }
    }
    ring
}

/// The square in physical pixels on which the circle sits: how many whole AP cells fit and where it starts
/// (centred, the leftover divided down).
fn circle_grid(bounds: Bounds<Pixels>, ap: Ap) -> (Dev, u32) {
    let r = Dev::of(bounds, ap.scale);
    let d = ap.device as f32;
    let side = r.w.min(r.h);
    let cells = (side / d).floor().max(0.0) as u32;
    let used = cells as f32 * d;
    let x = r.x + ((r.w - used) / 2.0).floor();
    let y = r.y + ((r.h - used) / 2.0).floor();
    (Dev { x, y, w: used, h: used }, cells)
}

fn span_rect(grid: Dev, span: CellSpan, d: f32) -> Dev {
    Dev {
        x: grid.x + span.col as f32 * d,
        y: grid.y + span.row as f32 * d,
        w: span.width as f32 * d,
        h: span.rows as f32 * d,
    }
}

/// Le fasce del cerchio dentro `bounds` in pixel logici (allineate ai pixel fisici): servono da maschera
/// to paint a photo in the circle.
pub fn circle_masks(bounds: Bounds<Pixels>, ap: Ap) -> Vec<Bounds<Pixels>> {
    let (grid, cells) = circle_grid(bounds, ap);
    let d = ap.device as f32;
    circle_spans(cells)
        .into_iter()
        .map(|span| {
            let r = span_rect(grid, span, d);
            Bounds::new(
                point(px(r.x / ap.scale), px(r.y / ap.scale)),
                size(px(r.w / ap.scale), px(r.h / ap.scale)),
            )
        })
        .collect()
}

/// Fills the pixel circle inside `bounds`.
pub fn paint_circle_fill(window: &mut Window, bounds: Bounds<Pixels>, ap: Ap, color: Hsla) {
    let (grid, cells) = circle_grid(bounds, ap);
    let d = ap.device as f32;
    for span in circle_spans(cells) {
        span_rect(grid, span, d).paint(window, ap.scale, color);
    }
}

/// Il contorno di 1 AP del cerchio pixel dentro `bounds`.
pub fn paint_circle_ring(window: &mut Window, bounds: Bounds<Pixels>, ap: Ap, color: Hsla) {
    let (grid, cells) = circle_grid(bounds, ap);
    let d = ap.device as f32;
    for span in circle_ring(cells) {
        span_rect(grid, span, d).paint(window, ap.scale, color);
    }
}

/// Outline of the pixel circle that fills the parent (hover frame of an ArtistTile, ring of
/// focus).
pub fn circle_outline(color: Hsla) -> Canvas<()> {
    fill_parent(canvas(
        |_, _, _| (),
        move |bounds, (), window, _: &mut App| paint_circle_ring(window, bounds, Ap::of(window), color),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dissolve_covers_one_more_quarter_of_the_cells_per_frame() {
        let mut thresholds: Vec<u8> =
            (0..2).flat_map(|row| (0..2).map(move |col| bayer2(col, row))).collect();
        thresholds.sort_unstable();
        assert_eq!(thresholds, vec![0, 1, 2, 3], "each frame changes a different quarter");
        assert_eq!(bayer2(0, 0), bayer2(2, 2), "the pattern repeats every 2 cells");
    }

    fn cells_of(spans: &[CellSpan]) -> std::collections::HashSet<(u32, u32)> {
        let mut set = std::collections::HashSet::new();
        for s in spans {
            for row in s.row..s.row + s.rows {
                for col in s.col..s.col + s.width {
                    set.insert((col, row));
                }
            }
        }
        set
    }

    #[test]
    fn the_pixel_circle_is_symmetric_and_close_to_a_disc() {
        for cells in [8, 15, 16, 48, 80] {
            let inside = cells_of(&circle_spans(cells));
            for &(x, y) in &inside {
                assert!(inside.contains(&(cells - 1 - x, y)) && inside.contains(&(x, cells - 1 - y)));
                assert!(inside.contains(&(y, x)), "simmetrico sulla diagonale ({cells})");
            }
            let disc = std::f32::consts::PI * (cells as f32 / 2.0).powi(2);
            let error = (inside.len() as f32 - disc).abs() / disc;
            assert!(error < 0.08, "{cells} celle: {} contro {disc}", inside.len());
            // The middle row is full, the first is not.
            let middle =
                circle_spans(cells).into_iter().find(|s| s.row <= cells / 2 && cells / 2 < s.row + s.rows);
            assert_eq!(middle.map(|s| s.width), Some(cells));
            assert!(circle_spans(cells)[0].width < cells);
        }
    }

    #[test]
    fn rows_of_the_same_width_are_merged() {
        let spans = circle_spans(48);
        assert!(spans.len() < 48, "le righe uguali al centro sono una fascia sola");
        assert_eq!(spans.iter().map(|s| s.rows).sum::<u32>(), 48);
        assert!(spans.windows(2).all(|w| w[0].col != w[1].col));
    }

    #[test]
    fn the_ring_is_the_inside_cells_that_touch_the_outside() {
        let cells = 20;
        let inside = cells_of(&circle_spans(cells));
        let ring = cells_of(&circle_ring(cells));
        for (x, y) in inside.iter().copied() {
            let out = |dx: i64, dy: i64| {
                let (nx, ny) = (i64::from(x) + dx, i64::from(y) + dy);
                nx < 0 || ny < 0 || !inside.contains(&(nx as u32, ny as u32))
            };
            let edge = out(-1, 0) || out(1, 0) || out(0, -1) || out(0, 1);
            assert_eq!(ring.contains(&(x, y)), edge, "cella ({x}, {y})");
        }
    }

    #[test]
    fn a_tiny_circle_keeps_its_centre() {
        assert_eq!(cells_of(&circle_spans(1)).len(), 1);
        assert!(!cells_of(&circle_spans(2)).is_empty());
        assert!(circle_spans(0).is_empty());
    }

    fn rect(w: f32, h: f32) -> Dev {
        Dev { x: 0.0, y: 0.0, w, h }
    }

    /// Area coperta dalle fasce di un ottagono.
    fn area(rows: &[(f32, f32, f32, f32)]) -> f32 {
        rows.iter().map(|(y0, y1, x0, x1)| (y1 - y0) * (x1 - x0)).sum()
    }

    #[test]
    fn notch_one_removes_exactly_the_four_corner_pixels() {
        let d = 2.0;
        let rows = octagon_rows(rect(20.0, 20.0), 1, d);
        assert_eq!(area(&rows), 20.0 * 20.0 - 4.0 * d * d);
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn notch_two_removes_a_two_step_staircase_per_corner() {
        let d = 3.0;
        let rows = octagon_rows(rect(30.0, 30.0), 2, d);
        // Per angolo: 2 + 1 pixel d'arte tolti.
        assert_eq!(area(&rows), 30.0 * 30.0 - 4.0 * 3.0 * d * d);
    }

    #[test]
    fn notch_never_eats_more_than_half_the_side() {
        let rows = octagon_rows(rect(12.0, 12.0), 8, 2.0);
        assert!(area(&rows) > 0.0);
        assert!(rows.iter().all(|(y0, y1, x0, x1)| y1 >= y0 && x1 >= x0));
    }

    #[test]
    fn zero_notch_is_a_plain_rectangle() {
        let rows = octagon_rows(rect(16.0, 8.0), 0, 2.0);
        assert_eq!(rows, vec![(0.0, 8.0, 0.0, 16.0)]);
    }
}
