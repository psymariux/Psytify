//! I gesti del telefono: un riconoscitore puro (`recognizer`), l'adattatore che lo alimenta con gli
//! eventi di Iced (`tracker`) e i widget che lo usano: la riga che si trascina per un'azione
//! (`swipe`), il foglio del player che segue il dito (`sheet`), la lista che si riordina con la
//! pressione lunga (`reorder`) e l'area generica (`area`).
//!
//! Tutto lavora sugli eventi: niente timer e niente frame se nessun dito è giù e nessuna animazione
//! sta finendo (`RedrawRequest` solo mentre una riga o un foglio si muove).

#[cfg(test)]
mod harness;

pub mod area;
pub mod memory;
pub mod recognizer;
pub mod reorder;
pub mod sheet;
pub mod swipe;
pub mod tracker;
