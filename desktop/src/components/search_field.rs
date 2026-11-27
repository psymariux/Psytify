// Campo di testo portato da Zed: `crates/gpui/examples/input.rs` al rev
// d31d1074a07b7bbaa43a52fef7847a00dd491baf del repository zed-industries/zed.
// Copyright (c) Zed Industries, Inc. Licenza Apache-2.0 (https://www.apache.org/licenses/LICENSE-2.0).
// Modifiche di Psytify: riga singola con aspetto pixel (token), caret da 1 AP che lampeggia solo
// con il campo a fuoco, selezione con `LINE_STRONG`, tasti Ctrl (non Cmd), azione Esc, richiamo
// `on_change`/`on_submit`, icona di ricerca, suggerimento `Ctrl K`, pulsante di cancellazione, stato
// disabilitato e di caricamento; variante mascherata (password, codice), alta e con il filtro dei
// caratteri.

//! SearchField: campo di ricerca. Altezza `CONTROL_H`, fill `SURFACE_WELL`, bordo di 1 AP
//! (`CONTROL_BORDER`, hover `CONTROL_BORDER_HOVER`, a fuoco `PURE_WHITE`), tacca 1. Esc cancella e poi
//! sfoca; Invio chiama `on_submit`. La variante `plain` è il campo dei dialoghi (linee guida §22: senza
//! icone), dove Esc chiude il dialogo.
//!
//! La variante `masked` è il campo di una password o di un codice: mostra un pallino per carattere, non
//! copia né taglia negli appunti, non dà il testo vero al sistema di input (le richieste di contesto
//! ricevono i pallini) e il suo `Debug` non lo stampa. Il testo si prende con [`SearchField::take`], che
//! svuota il campo nello stesso momento: chi lo manda al core lo avvolge subito in `Secret`. `large` è il
//! campo da 40 dp dei moduli (`CONTROL_H_LARGE`); `sanitize` e `max_chars` filtrano ciò che si scrive o si
//! incolla (il nome utente: minuscole, cifre e `_`).

use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, Focusable, GlobalElementId, InteractiveElement, IntoElement, KeyBinding,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, ParentElement, Pixels,
    Point, Render, ShapedLine, SharedString, StatefulInteractiveElement, Style, Styled, TextRun,
    UTF16Selection, Window, actions, div, fill, point, px, relative, size,
};
use psytify_design::sprite;
use psytify_design::tokens::color::{
    CARET, CONTROL_BORDER, CONTROL_BORDER_HOVER, LINE_STRONG, PURE_WHITE, SURFACE_WELL, TEXT_DISABLED,
    TEXT_PRIMARY, TEXT_TERTIARY,
};
use psytify_design::tokens::motion::{CARET_BLINK_OFF_MS, CARET_BLINK_ON_MS};
use psytify_design::tokens::shape::NOTCH_1;
use psytify_design::tokens::size::{CONTROL_H, CONTROL_H_LARGE, SEARCH_FIELD_MAX_W};
use psytify_design::tokens::space::{SPACE_2, SPACE_3};
use psytify_design::tokens::type_::{TYPE_BODY, TYPE_META};
use unicode_segmentation::UnicodeSegmentation;

use super::icon_button::IconButton;
use crate::pixel::anim::request_refresh;
use crate::pixel::shape::notch_box;
use crate::pixel::{PixelSprite, SpriteStore};
use crate::theme::{Ap, TypeStyle, col, dp};

actions!(
    psytify_text_input,
    [
        Backspace,
        Delete,
        Left,
        Right,
        SelectLeft,
        SelectRight,
        SelectAll,
        Home,
        End,
        Paste,
        Cut,
        Copy,
        Escape,
        Submit
    ]
);

const CONTEXT: &str = "PsytifyTextInput";

/// Scorciatoie del campo; da registrare una volta all'avvio.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, Some(CONTEXT)),
        KeyBinding::new("delete", Delete, Some(CONTEXT)),
        KeyBinding::new("left", Left, Some(CONTEXT)),
        KeyBinding::new("right", Right, Some(CONTEXT)),
        KeyBinding::new("shift-left", SelectLeft, Some(CONTEXT)),
        KeyBinding::new("shift-right", SelectRight, Some(CONTEXT)),
        KeyBinding::new("ctrl-a", SelectAll, Some(CONTEXT)),
        KeyBinding::new("ctrl-v", Paste, Some(CONTEXT)),
        KeyBinding::new("ctrl-c", Copy, Some(CONTEXT)),
        KeyBinding::new("ctrl-x", Cut, Some(CONTEXT)),
        KeyBinding::new("home", Home, Some(CONTEXT)),
        KeyBinding::new("end", End, Some(CONTEXT)),
        KeyBinding::new("escape", Escape, Some(CONTEXT)),
        KeyBinding::new("enter", Submit, Some(CONTEXT)),
    ]);
}

type OnText = Rc<dyn Fn(&str, &mut Window, &mut App)>;
type OnKey = Rc<dyn Fn(&mut Window, &mut App)>;

/// Il carattere che un campo mascherato mostra al posto di ognuno dei suoi.
const MASK: char = '•';

/// Il testo da disegnare: quello vero, o un pallino per carattere.
fn shown_text(content: &str, masked: bool) -> SharedString {
    if masked { MASK.to_string().repeat(content.chars().count()).into() } else { content.to_owned().into() }
}

/// Da una posizione nel testo vero (byte) alla stessa posizione nel testo disegnato.
fn to_display(content: &str, offset: usize, masked: bool) -> usize {
    if !masked {
        return offset;
    }
    content.get(..offset).map_or(0, |head| head.chars().count()) * MASK.len_utf8()
}

/// Da una posizione nel testo disegnato (byte) a quella nel testo vero, sempre su un confine di carattere.
fn from_display(content: &str, offset: usize, masked: bool) -> usize {
    if !masked {
        return offset;
    }
    let chars = offset / MASK.len_utf8();
    content.char_indices().nth(chars).map_or(content.len(), |(at, _)| at)
}

/// Il testo da inserire dopo il filtro del campo e il limite di caratteri (contando quelli che restano).
fn admitted(
    new_text: &str,
    sanitize: Option<fn(&str) -> String>,
    max_chars: Option<usize>,
    kept: usize,
) -> String {
    let text = match sanitize {
        Some(filter) => filter(new_text),
        None => new_text.to_owned(),
    };
    match max_chars {
        Some(max) => text.chars().take(max.saturating_sub(kept)).collect(),
        None => text,
    }
}

/// Ciò che il `Debug` di un campo dice del suo testo: un campo mascherato dice solo quanti caratteri ha.
fn describe(content: &str, masked: bool) -> String {
    if masked { format!("[{} caratteri nascosti]", content.chars().count()) } else { format!("{content:?}") }
}

pub struct SearchField {
    id: ElementId,
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    is_selecting: bool,
    hovered: bool,
    disabled: bool,
    loading: bool,
    show_hint: bool,
    on_change: Option<OnText>,
    on_submit: Option<OnText>,
    /// Campo di un dialogo: niente icona di ricerca né suggerimento `Ctrl K`, ed Esc chiama `on_escape`.
    plain: bool,
    on_escape: Option<OnKey>,
    /// Password o codice: pallini al posto dei caratteri, niente appunti, `Debug` senza il testo.
    masked: bool,
    /// Alto `CONTROL_H_LARGE` (i moduli di accesso e del profilo).
    large: bool,
    sanitize: Option<fn(&str) -> String>,
    max_chars: Option<usize>,
}

impl std::fmt::Debug for SearchField {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchField")
            .field("id", &self.id)
            .field("text", &describe(&self.content, self.masked))
            .field("masked", &self.masked)
            .finish_non_exhaustive()
    }
}

impl SearchField {
    pub fn new(
        id: impl Into<ElementId>,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            id: id.into(),
            // Il Tab di GPUI legge il segno sulla maniglia, non quello dell'elemento (`track_focus` con una
            // maniglia data ignora `.tab_stop` dell'elemento): senza, Tab non arriva mai al campo.
            focus_handle: cx.focus_handle().tab_stop(true),
            content: SharedString::default(),
            placeholder: placeholder.into(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            is_selecting: false,
            hovered: false,
            disabled: false,
            loading: false,
            show_hint: true,
            on_change: None,
            on_submit: None,
            plain: false,
            on_escape: None,
            masked: false,
            large: false,
            sanitize: None,
            max_chars: None,
        }
    }

    /// Senza il suggerimento `Ctrl K`, che appartiene solo alla ricerca della barra del titolo.
    pub fn without_hint(mut self) -> Self {
        self.show_hint = false;
        self
    }

    /// Il campo di una password o di un codice (senza icone, come `plain`): vedi la testa del modulo.
    pub fn masked(mut self) -> Self {
        self.masked = true;
        self.plain = true;
        self.show_hint = false;
        self
    }

    /// Alto `CONTROL_H_LARGE` (40 dp), come i campi dei moduli.
    pub fn large(mut self) -> Self {
        self.large = true;
        self
    }

    /// Un filtro per ciò che si scrive o si incolla (toglie o cambia i caratteri).
    pub fn sanitize(mut self, filter: fn(&str) -> String) -> Self {
        self.sanitize = Some(filter);
        self
    }

    /// Al più `max` caratteri.
    pub fn max_chars(mut self, max: usize) -> Self {
        self.max_chars = Some(max);
        self
    }

    /// Prende il testo e svuota il campo nello stesso momento, senza chiamare `on_change` (chi lo prende sa
    /// che ora è vuoto). Per la password e il codice: chi lo riceve lo avvolge subito in `Secret`.
    pub fn take(&mut self, cx: &mut Context<Self>) -> String {
        let text = self.content.to_string();
        self.content = SharedString::default();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        self.last_layout = None;
        cx.notify();
        text
    }

    /// Vuota il campo senza chiamare `on_change` (un modulo che cambia scheda).
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        let _ = self.take(cx);
    }

    /// Il campo ha del testo (senza darlo a chi chiede: per un campo mascherato basta sapere questo).
    pub fn is_empty(&self) -> bool {
        self.content.is_empty()
    }

    /// Quanti caratteri ha il campo.
    pub fn char_count(&self) -> usize {
        self.content.chars().count()
    }

    /// Il campo di testo di un dialogo (il nome di una playlist): senza icona di ricerca, senza
    /// suggerimento e senza il tasto per cancellare; Esc chiama `on_escape` (chiude il dialogo).
    pub fn plain(mut self) -> Self {
        self.plain = true;
        self.show_hint = false;
        self
    }

    pub fn on_escape(mut self, f: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_escape = Some(Rc::new(f));
        self
    }

    /// Il testo intero selezionato: scrivere lo sostituisce (il nome proposto di una playlist).
    pub fn select_all_text(mut self) -> Self {
        self.selected_range = 0..self.content.len();
        self.selection_reversed = false;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self.focus_handle = self.focus_handle.clone().tab_stop(!disabled);
        self
    }

    pub fn loading(mut self, loading: bool) -> Self {
        self.loading = loading;
        self
    }

    pub fn with_text(mut self, text: impl Into<SharedString>) -> Self {
        self.content = text.into();
        let end = self.content.len();
        self.selected_range = end..end;
        self
    }

    /// Accende o spegne l'indicatore di attesa (la ricerca è in corso) anche a campo già creato.
    pub fn set_loading(&mut self, loading: bool, cx: &mut Context<Self>) {
        if self.loading != loading {
            self.loading = loading;
            cx.notify();
        }
    }

    /// Sostituisce il testo a campo già creato (cursore in fondo), senza chiamare `on_change`: chi lo
    /// cambia manda da sé la ricerca.
    pub fn set_text(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = text.into();
        let end = self.content.len();
        self.selected_range = end..end;
        self.marked_range = None;
        cx.notify();
    }

    pub fn on_change(mut self, f: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(Rc::new(f));
        self
    }

    pub fn on_submit(mut self, f: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(f));
        self
    }

    /// Il testo del campo; un campo mascherato non lo dà mai (vuoto): si prende solo con [`Self::take`].
    pub fn text(&self) -> &str {
        if self.masked { "" } else { &self.content }
    }

    /// `on_change` riceve il testo; da un campo mascherato riceve sempre una stringa vuota (chi ascolta sa
    /// solo che è cambiato: [`Self::char_count`] dice quanto è lungo).
    fn changed(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(f) = self.on_change.clone() {
            f(self.text(), window, cx);
        }
    }

    fn left(&mut self, _: &Left, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), window, cx);
        } else {
            self.move_to(self.selected_range.start, window, cx)
        }
    }

    fn right(&mut self, _: &Right, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), window, cx);
        } else {
            self.move_to(self.selected_range.end, window, cx)
        }
    }

    fn select_left(&mut self, _: &SelectLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), window, cx);
    }

    fn select_right(&mut self, _: &SelectRight, window: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), window, cx);
    }

    fn select_all(&mut self, _: &SelectAll, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, window, cx);
        self.select_to(self.content.len(), window, cx)
    }

    fn home(&mut self, _: &Home, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, window, cx);
    }

    fn end(&mut self, _: &End, window: &mut Window, cx: &mut Context<Self>) {
        self.move_to(self.content.len(), window, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let prev = self.previous_boundary(self.cursor_offset());
            if self.cursor_offset() == prev {
                return;
            }
            self.select_to(prev, window, cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            let next = self.next_boundary(self.cursor_offset());
            if self.cursor_offset() == next {
                return;
            }
            self.select_to(next, window, cx)
        }
        self.replace_text_in_range(None, "", window, cx)
    }

    /// Esc: prima cancella il testo, poi (campo già vuoto) toglie il focus.
    fn escape(&mut self, _: &Escape, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(f) = self.on_escape.clone() {
            f(window, cx);
        } else if self.content.is_empty() {
            window.blur(cx);
        } else {
            self.clear(window, cx);
        }
    }

    fn submit(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(f) = self.on_submit.clone() {
            f(self.text(), window, cx);
        }
    }

    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.content = SharedString::default();
        self.selected_range = 0..0;
        self.selection_reversed = false;
        self.marked_range = None;
        window.notify_text_input_changed(&self.focus_handle);
        self.changed(window, cx);
        cx.notify();
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled {
            return;
        }
        window.focus(&self.focus_handle, cx);
        self.is_selecting = true;
        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), window, cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), window, cx)
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _window: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), window, cx);
        }
    }

    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.replace_text_in_range(None, &text.replace('\n', " "), window, cx);
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        // Una password non va negli appunti.
        if !self.selected_range.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
        }
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if !self.selected_range.is_empty() && !self.masked {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.content[self.selected_range.clone()].to_string(),
            ));
            self.replace_text_in_range(None, "", window, cx)
        }
    }

    fn move_to(&mut self, offset: usize, window: &Window, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        self.selection_reversed = false;
        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify()
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed { self.selected_range.start } else { self.selected_range.end }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }
        let (Some(bounds), Some(line)) = (self.last_bounds.as_ref(), self.last_layout.as_ref()) else {
            return 0;
        };
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.content.len();
        }
        from_display(&self.content, line.closest_index_for_x(position.x - bounds.left()), self.masked)
    }

    fn select_to(&mut self, offset: usize, window: &Window, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset
        } else {
            self.selected_range.end = offset
        };
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify()
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;
        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }
        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;
        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }
        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }
}

impl EntityInputHandler for SearchField {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        let text = self.content.get(range)?;
        // Al sistema di input un campo mascherato dà i pallini, mai il testo vero.
        Some(if self.masked { shown_text(text, true).to_string() } else { text.to_owned() })
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn set_selected_text_range(
        &mut self,
        range_utf16: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.selected_range = self.range_from_utf16(&range_utf16);
        self.selection_reversed = false;
        window.notify_text_selection_changed(&self.focus_handle);
        cx.notify();
    }

    fn text_length_utf16(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<usize> {
        Some(self.content.encode_utf16().count())
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_range.as_ref().map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.marked_range = None;
        window.notify_text_input_changed(&self.focus_handle);
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let kept = self.content.chars().count() - self.content[range.clone()].chars().count();
        let new_text = &admitted(new_text, self.sanitize, self.max_chars, kept);
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..]).into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.selection_reversed = false;
        self.marked_range.take();
        window.notify_text_input_changed(&self.focus_handle);
        self.changed(window, cx);
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }
        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());
        let kept = self.content.chars().count() - self.content[range.clone()].chars().count();
        let new_text = &admitted(new_text, self.sanitize, self.max_chars, kept);
        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..]).into();
        if !new_text.is_empty() {
            self.marked_range = Some(range.start..range.start + new_text.len());
        } else {
            self.marked_range = None;
        }
        let insertion_start_utf16 = self.offset_to_utf16(range.start);
        let inserted_length_utf16 = new_text.encode_utf16().count();
        self.selected_range = new_selected_range_utf16
            .map(|selected| {
                self.range_from_utf16(
                    &(insertion_start_utf16 + selected.start.min(inserted_length_utf16)
                        ..insertion_start_utf16 + selected.end.min(inserted_length_utf16)),
                )
            })
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());
        self.selection_reversed = false;
        window.notify_text_input_changed(&self.focus_handle);
        self.changed(window, cx);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let last_layout = self.last_layout.as_ref()?;
        if last_layout.text != shown_text(&self.content, self.masked) {
            return None;
        }
        let range = self.range_from_utf16(&range_utf16);
        let x = |offset| last_layout.x_for_index(to_display(&self.content, offset, self.masked));
        Some(Bounds::from_corners(
            point(bounds.left() + x(range.start), bounds.top()),
            point(bounds.left() + x(range.end), bounds.bottom()),
        ))
    }

    fn selection_bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Bounds<Pixels>> {
        self.bounds_for_range(range_utf16, bounds, window, cx).into_iter().collect()
    }

    fn character_index_for_point(
        &mut self,
        point: gpui::Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let line_point = self.last_bounds?.localize(&point)?;
        let last_layout = self.last_layout.as_ref()?;
        if last_layout.text != shown_text(&self.content, self.masked) {
            return None;
        }
        let utf8_index =
            from_display(&self.content, last_layout.closest_index_for_x(line_point.x), self.masked);
        Some(self.offset_to_utf16(utf8_index))
    }
}

/// Elemento che disegna testo, selezione e caret.
struct TextElement {
    input: Entity<SearchField>,
}

struct PrepaintState {
    line: Option<ShapedLine>,
    cursor: Option<PaintQuad>,
    selection: Option<PaintQuad>,
}

impl gpui::Element for TextElement {
    type RequestLayoutState = ();
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let input = self.input.read(cx);
        let content = input.content.clone();
        let masked = input.masked;
        // Le posizioni si disegnano sul testo mostrato: per un campo mascherato, i pallini.
        let selected_range = to_display(&content, input.selected_range.start, masked)
            ..to_display(&content, input.selected_range.end, masked);
        let cursor = to_display(&content, input.cursor_offset(), masked);
        let style = window.text_style();
        let (display_text, text_color) = if content.is_empty() {
            (input.placeholder.clone(), col(if input.disabled { TEXT_DISABLED } else { TEXT_TERTIARY }))
        } else {
            (shown_text(&content, masked), col(if input.disabled { TEXT_DISABLED } else { TEXT_PRIMARY }))
        };
        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let marked = input
            .marked_range
            .as_ref()
            .map(|range| to_display(&content, range.start, masked)..to_display(&content, range.end, masked));
        let runs = if let Some(marked_range) = marked.as_ref() {
            vec![
                TextRun { len: marked_range.start, ..run.clone() },
                TextRun {
                    len: marked_range.end - marked_range.start,
                    underline: Some(gpui::UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun { len: display_text.len() - marked_range.end, ..run },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        // A placeholder wider than the field ends in an ellipsis instead of a clipped glyph.
        let (display_text, runs) = if content.is_empty() {
            let mut wrapper = window.text_system().line_wrapper(style.font(), font_size);
            let (text, runs) =
                wrapper.truncate_line(display_text, bounds.size.width, "…", &runs, gpui::TruncateFrom::End);
            (text, runs.into_owned())
        } else {
            (display_text, runs)
        };
        let line = window.text_system().shape_line(display_text, font_size, &runs, None);
        let cursor_pos = line.x_for_index(cursor);
        let ap = Ap::of(window);
        let (selection, cursor) = if selected_range.is_empty() {
            let caret_h = dp(16.0);
            let top = bounds.top() + (bounds.size.height - caret_h) / 2.0;
            (
                None,
                Some(fill(
                    Bounds::new(point(bounds.left() + cursor_pos, top), size(ap.one(), caret_h)),
                    col(CARET),
                )),
            )
        } else {
            (
                Some(fill(
                    Bounds::from_corners(
                        point(bounds.left() + line.x_for_index(selected_range.start), bounds.top()),
                        point(bounds.left() + line.x_for_index(selected_range.end), bounds.bottom()),
                    ),
                    col(LINE_STRONG),
                )),
                None,
            )
        };
        PrepaintState { line: Some(line), cursor, selection }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.input.clone()), cx);
        if let Some(selection) = prepaint.selection.take() {
            window.paint_quad(selection)
        }
        if let Some(line) = prepaint.line.take() {
            // Un errore di disegno del testo non ha rimedio qui: si salta il frame del campo.
            let _ = line.paint(bounds.origin, window.line_height(), gpui::TextAlign::Left, None, window, cx);
            if focus_handle.is_focused(window)
                && let Some(cursor) = prepaint.cursor.take()
            {
                // Caret a scatti: 530 ms acceso, 530 spento. Il ridisegno si programma solo a fuoco. Con il
                // movimento ridotto il caret resta acceso e non c'è nessun timer.
                let period = u64::from(CARET_BLINK_ON_MS + CARET_BLINK_OFF_MS);
                let store = cx.global::<SpriteStore>();
                let (now, steady) = (store.elapsed_ms(), store.reduce_motion());
                let phase = now % period;
                if steady {
                    window.paint_quad(cursor);
                } else if phase < u64::from(CARET_BLINK_ON_MS) {
                    window.paint_quad(cursor);
                    request_refresh(window, cx, u64::from(CARET_BLINK_ON_MS) - phase);
                } else {
                    request_refresh(window, cx, period - phase);
                }
            }
            self.input.update(cx, |input, _cx| {
                input.last_layout = Some(line);
                input.last_bounds = Some(bounds);
            });
        }
    }
}

impl Render for SearchField {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window) && !self.disabled;
        let border = if self.disabled {
            LINE_STRONG
        } else if focused {
            PURE_WHITE
        } else if self.hovered {
            CONTROL_BORDER_HOVER
        } else {
            CONTROL_BORDER
        };
        let icon_tint = if self.disabled {
            psytify_design::tokens::color::ICON_DISABLED
        } else {
            psytify_design::tokens::color::ICON_IDLE
        };
        let leading = if self.plain {
            None
        } else if self.loading {
            Some(PixelSprite::new(sprite::LOADER))
        } else {
            Some(PixelSprite::new(sprite::ICON_SEARCH).tint(icon_tint))
        };
        let trailing = if self.plain {
            None
        } else if !self.content.is_empty() && !self.disabled {
            Some(
                IconButton::new(SharedString::from(format!("{:?}-clear", self.id)), sprite::ICON_CLOSE)
                    .size(24.0)
                    .tooltip("Cancella", Some("Esc"))
                    .on_click(cx.listener(|this, _, window, cx| this.clear(window, cx)))
                    .into_any_element(),
            )
        } else if self.show_hint && !focused && !self.disabled {
            Some(
                div()
                    .relative()
                    .px(dp(SPACE_2))
                    .type_role(TYPE_META)
                    .text_color(col(TEXT_TERTIARY))
                    .child(notch_box(0, None, Some(col(LINE_STRONG))))
                    .child(div().relative().child("Ctrl K"))
                    .into_any_element(),
            )
        } else {
            None
        };
        div()
            .id(self.id.clone())
            .relative()
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle(cx))
            .tab_stop(!self.disabled)
            .cursor(if self.disabled { CursorStyle::Arrow } else { CursorStyle::IBeam })
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::escape))
            .on_action(cx.listener(Self::submit))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_hover(cx.listener(|this, hovering: &bool, _, cx| {
                if this.hovered != *hovering {
                    this.hovered = *hovering;
                    cx.notify();
                }
            }))
            .h(dp(if self.large { CONTROL_H_LARGE } else { CONTROL_H }))
            .w_full()
            .max_w(dp(SEARCH_FIELD_MAX_W))
            .flex()
            .items_center()
            .gap(dp(SPACE_3))
            .px(dp(SPACE_3))
            .type_role(TYPE_BODY)
            .child(notch_box(NOTCH_1, Some(col(SURFACE_WELL)), Some(col(border))))
            .children(leading.map(|leading| div().relative().flex_none().child(leading)))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    .child(TextElement { input: cx.entity() }),
            )
            .children(trailing.map(|t| div().relative().flex_none().child(t)))
    }
}

impl Focusable for SearchField {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_masked_field_shows_one_dot_per_character_and_maps_positions_back() {
        let secret = "pàss🙂";
        let shown = shown_text(secret, true);
        assert_eq!(shown.chars().count(), 5);
        assert!(shown.chars().all(|c| c == MASK));
        assert_eq!(shown_text(secret, false), secret);
        // Ogni confine di carattere del testo vero ha il suo posto fra i pallini, e torna indietro uguale.
        for (at, _) in secret.char_indices().chain(std::iter::once((secret.len(), ' '))) {
            let shown_at = to_display(secret, at, true);
            assert_eq!(from_display(secret, shown_at, true), at);
        }
        // Un punto a metà di un pallino cade sul carattere che lo comincia.
        assert_eq!(from_display(secret, 1, true), 0);
        assert_eq!(to_display(secret, 3, false), 3);
    }

    #[test]
    fn the_debug_of_a_masked_field_never_holds_the_text() {
        let described = describe("hunter2-segreta", true);
        assert!(!described.contains("hunter2"), "{described}");
        assert!(described.contains("15"));
        assert!(describe("Neon", false).contains("Neon"));
    }

    #[test]
    fn what_is_typed_passes_the_filter_and_the_limit() {
        fn handle(text: &str) -> String {
            text.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>().to_lowercase()
        }
        assert_eq!(admitted("Ciao Mondo!", Some(handle), None, 0), "ciaomondo");
        assert_eq!(admitted("abcdef", None, Some(4), 1), "abc");
        assert_eq!(admitted("abc", None, Some(4), 4), "");
        assert_eq!(admitted("a\nb", None, None, 0), "a\nb");
    }
}
