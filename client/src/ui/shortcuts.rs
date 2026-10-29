//! Il riquadro delle scorciatoie da tastiera (tasto `?`, o dalle impostazioni): l'elenco per
//! gruppi, al centro sopra un velo scuro. Un clic sul velo, Esc o di nuovo `?` lo chiudono.

use std::time::Instant;

use iced::widget::{Space, column, container, mouse_area, opaque, row, scrollable, stack, text};
use iced::{Alignment, Element, Length, Padding, alignment};

use super::components::{h2, muted, small, text_button};
use super::{motion, theme};
use crate::app::{App, Message};

const WIDTH: f32 = 640.0;
const RISE: f32 = 10.0;
/// Larghezza della colonna dei tasti.
const KEYS: f32 = 168.0;
/// Spazio fra le descrizioni e la barra di scorrimento.
const SCROLL_GAP: f32 = 18.0;

/// Le scorciatoie, per gruppo: tasti e cosa fanno. È anche la documentazione di `app::shortcut`.
pub const GROUPS: &[(&str, &[(&str, &str)])] = &[
    (
        "Riproduzione",
        &[
            ("Spazio", "Play e pausa"),
            ("← →", "Indietro e avanti di 5 secondi"),
            ("Maiusc ← →", "Indietro e avanti di 15 secondi"),
            ("Ctrl ← →  ·  P N", "Brano precedente e successivo"),
            ("0 … 9", "Salta al punto del brano (3 = 30%)"),
            ("Ctrl ↑ ↓  ·  + −", "Volume"),
            ("M", "Muto"),
            ("S  ·  R", "Casuale e ripeti"),
            ("L", "Mi piace al brano che suona"),
        ],
    ),
    (
        "Viste",
        &[
            ("F", "Testo a tutto schermo"),
            ("V", "Videoclip a tutto schermo"),
            ("I  ·  Q  ·  T", "Pannello: in riproduzione, coda, testo"),
            ("F11", "Finestra a schermo intero"),
            ("Esc", "Chiude ciò che è aperto"),
        ],
    ),
    (
        "Navigazione",
        &[
            ("Ctrl F  ·  /", "Cerca"),
            ("Alt ← →", "Indietro e avanti (anche i tasti laterali del mouse)"),
            ("Ctrl 1 … 6", "Home, Cerca, Catalogo, Mi piace, Novità, Jam"),
            ("Ctrl ,", "Impostazioni"),
            ("Ctrl N", "Nuova playlist"),
            ("?", "Questo elenco"),
        ],
    ),
];

/// Il riquadro, se è aperto.
pub fn view(app: &App) -> Option<Element<'_, Message>> {
    let since: Instant = app.shortcuts?;
    let shown = motion::appear(since, theme::MOTION_PANEL);
    let mut groups = column![].spacing(22);
    for (title, keys) in GROUPS {
        let mut list = column![small(*title)].spacing(10);
        for (keys, what) in *keys {
            list = list.push(
                row![
                    container(
                        container(
                            text(*keys).size(theme::FS_SMALL).font(theme::FONT_MEDIUM).color(theme::TEXT)
                        )
                        .padding([3, 8])
                        .style(theme::key_cap)
                    )
                    .width(KEYS),
                    muted(*what),
                ]
                .spacing(16)
                .align_y(Alignment::Center),
            );
        }
        groups = groups.push(list);
    }
    let header = row![
        h2("Scorciatoie da tastiera"),
        Space::with_width(Length::Fill),
        text_button("Chiudi", Message::ToggleShortcuts)
    ]
    .align_y(Alignment::Center);
    let card = container(
        column![
            header,
            // La barra di scorrimento sul bordo, con un margine perché non copra le descrizioni.
            scrollable(container(groups).padding(Padding { right: SCROLL_GAP, ..Padding::ZERO }))
                .width(Length::Fill)
                .height(Length::Shrink)
                .style(theme::scroll)
        ]
        .spacing(20),
    )
    .width(WIDTH)
    .max_height(640)
    .padding(theme::PAD)
    .style(theme::dialog_box(shown));
    let centered = container(opaque(card))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(alignment::Horizontal::Center)
        .align_y(alignment::Vertical::Center)
        .padding(Padding { top: RISE * (1.0 - shown) * 2.0, ..Padding::ZERO });
    Some(
        stack![
            mouse_area(container(Space::new(Length::Fill, Length::Fill)).style(theme::scrim(shown)))
                .on_press(Message::ToggleShortcuts),
            centered,
        ]
        .into(),
    )
}
