//! La barra del titolo e la barra laterale di `Shell` come viste in `.cached()` (BACKLOG 23).
//!
//! `Shell` è la radice della finestra e GPUI la rifà a ogni disegno: a ogni scatto degli sprite che dipinge lei
//! (l'equalizzatore sopra le righe in corso delle pagine in cache, `PlayingSpot`) e di quelli delle viste che
//! contiene (GPUI segna sporchi gli antenati), e a ogni fotogramma del videoclip (il palco è suo figlio).
//! Prima, a ogni disegno, rifaceva anche barra del titolo e barra laterale. Ora sono due viste che si rifanno
//! solo quando cambia ciò che mostrano (`Key`, confrontata quando `Shell` si notifica: `Shell::sync_chrome`) o
//! quando si notificano da sé (il passaggio del mouse, il marchio che si anima mentre suona). Gli elementi li
//! costruisce sempre `Shell` (`Shell::chrome`), con i suoi gestori, come prima.

use gpui::{Context, IntoElement, Render, WeakEntity, Window, div};

use super::Shell;
use super::nav::Nav;
use crate::titlebar::AccountChip;

/// Quale parte.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    TitleBar,
    Sidebar,
}

/// Ciò che una parte mostra di `Shell`: finché non cambia, la parte si riusa dalla cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Key {
    TitleBar { playing: bool, back: bool, forward: bool, account: AccountChip },
    Sidebar { current: Nav, counts: [usize; 4] },
}

pub struct Chrome {
    shell: WeakEntity<Shell>,
    part: Part,
}

impl Chrome {
    pub fn new(shell: WeakEntity<Shell>, part: Part) -> Self {
        Self { shell, part }
    }
}

impl Render for Chrome {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let part = self.part;
        self.shell
            .update(cx, |shell, cx| shell.chrome(part, window, cx))
            .unwrap_or_else(|_| div().into_any_element())
    }
}
