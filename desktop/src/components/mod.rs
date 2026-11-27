//! Kit di componenti pixel di Psytify su GPUI. Ogni componente segue
//! `docs/design/PSYTIFY_COMPONENT_GUIDELINES.md`: stati rest, hover, premuto, focus, disabilitato,
//! selezionato, in riproduzione; solo token, nessun colore o misura scritti a mano.

// Il kit espone più costruttori e opzioni di quanti la galleria ne usi: alcune servono solo alle
// schermate che arrivano dopo i domini 4.3 e 4.4 del piano.
#![allow(dead_code)]

pub mod badge;
pub mod button;
pub mod card;
pub mod control;
pub mod frame;
pub mod icon_button;
pub mod indicators;
pub mod like_button;
pub mod list_row;
pub mod menu;
pub mod navigation;
pub mod now_playing;
pub mod overlay;
pub mod page_header;
pub mod person_row;
pub mod player_control;
pub mod playing_spot;
pub mod progress;
pub mod queue_row;
pub mod search_field;
pub mod section;
pub mod select;
pub mod separator;
pub mod setting_row;
pub mod shelf;
pub mod shortcut;
pub mod slider;
pub mod source_row;
pub mod tile;
pub mod toggle;
pub mod tooltip;
pub mod track_row;

pub use button::Button;
pub use control::Preview;
pub use icon_button::IconButton;
