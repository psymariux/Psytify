//! Le decisioni del videoclip, pure: quando una pipeline deve esistere, che cosa fare quando il core
//! pubblica un videoclip nuovo, come applicare al player ciò che le regole di sincronia del core decidono
//! (`psytify_core::video::Alignment`) e quando controllare lo scarto. Le applica `stage.rs`.
//!
//! **Quando esiste una pipeline.** Solo per il brano in corso, solo mentre un posto dell'interfaccia
//! mostra il videoclip (il pannello sul videoclip o la vista immersiva sul videoclip), solo mentre il brano
//! suona o si sta caricando, e solo per il videoclip che il core ha pronto per quel brano. Pausa, fermo,
//! errore, cambio di brano, videoclip nascosto: nessuna pipeline, nessun thread, nessun fotogramma
//! (`05-video-spike.md` §5).

use std::time::{Duration, Instant};

use psytify_core::video::Adjust;
use psytify_design::tokens::motion::PROGRESS_TICK_MS;
use uuid::Uuid;

/// Il videoclip che deve essere aperto: il brano e il numero d'ordine dell'indirizzo del core.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Want {
    pub track: Uuid,
    pub seq: u64,
}

/// Che cosa si sa per decidere.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Inputs {
    /// Un posto dell'interfaccia mostra il videoclip.
    pub shown: bool,
    /// Il brano in corso, se suona o si sta caricando (in pausa, fermo o in errore: `None`).
    pub running: Option<Uuid>,
    /// Il brano di cui parla la vista del video del core, e il `seq` del videoclip pronto (`None` se non
    /// è pronto: fermo, in preparazione o non riuscito).
    pub ready: Option<Want>,
}

/// Il videoclip che deve essere aperto adesso, se ce n'è uno.
pub fn wanted(inputs: Inputs) -> Option<Want> {
    let ready = inputs.ready?;
    (inputs.shown && inputs.running == Some(ready.track)).then_some(ready)
}

/// La pipeline che c'è: per quale videoclip, se si è già aperta e quale versione migliore si sta aprendo
/// di fianco.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Current {
    pub track: Uuid,
    pub seq: u64,
    /// La pipeline si è aperta (altrimenti si sta ancora aprendo su un altro thread).
    pub open: bool,
    /// Il `seq` della versione migliore che si apre di fianco.
    pub upgrading: Option<u64>,
}

/// Che cosa fare.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Niente: quello che c'è va bene (o non c'è niente e niente serve).
    Keep,
    /// Chiudere quello che c'è (se c'è) e aprire questo.
    Open(Want),
    /// Aprire di fianco una versione migliore dello stesso brano e sostituirla quando è pronta, senza
    /// fermarsi.
    Upgrade(Want),
    /// Chiudere: nessuno vuole più un videoclip.
    Close,
}

/// Dalla pipeline che c'è a quella che serve (le stesse regole del client Iced, `open_clip`).
pub fn plan(current: Option<Current>, want: Option<Want>) -> Action {
    match (current, want) {
        (None, None) => Action::Keep,
        (Some(_), None) => Action::Close,
        (Some(now), Some(want))
            if now.track == want.track && (now.seq == want.seq || now.upgrading == Some(want.seq)) =>
        {
            Action::Keep
        }
        (Some(now), Some(want)) if now.track == want.track && now.open => Action::Upgrade(want),
        (_, Some(want)) => Action::Open(want),
    }
}

/// Ciò che si può fare al player perché segua l'audio.
pub trait Controls {
    fn set_paused(&mut self, paused: bool);
    fn seek(&mut self, position: Duration);
}

/// Fa al player ciò che le regole del core hanno deciso: prima la pausa o la ripresa, poi il salto.
pub fn apply(player: &mut impl Controls, adjust: Adjust) {
    if let Some(paused) = adjust.pause {
        player.set_paused(paused);
    }
    if let Some(position) = adjust.seek {
        player.seek(position);
    }
}

/// Ogni quanto si controlla lo scarto fra video e audio: il battito della barra di avanzamento. Il controllo
/// viaggia sulla sveglia dei fotogrammi che c'è già (nessun timer in più): si fa solo se è passato almeno
/// questo dall'ultimo.
pub const CHECK_EVERY: Duration = Duration::from_millis(PROGRESS_TICK_MS as u64);

/// È ora di controllare lo scarto.
pub fn check_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|last| now.saturating_duration_since(last) >= CHECK_EVERY)
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: Uuid = Uuid::from_u128(0xA);
    const B: Uuid = Uuid::from_u128(0xB);

    fn ready(track: Uuid, seq: u64) -> Option<Want> {
        Some(Want { track, seq })
    }

    #[test]
    fn a_pipeline_exists_only_for_the_running_track_with_a_ready_clip_on_screen() {
        let on = Inputs { shown: true, running: Some(A), ready: ready(A, 1) };
        assert_eq!(wanted(on), ready(A, 1));
        // Nascosto, in pausa (o fermo, in errore), un altro brano, nessun videoclip pronto: niente.
        assert_eq!(wanted(Inputs { shown: false, ..on }), None);
        assert_eq!(wanted(Inputs { running: None, ..on }), None);
        assert_eq!(wanted(Inputs { running: Some(B), ..on }), None);
        assert_eq!(wanted(Inputs { ready: None, ..on }), None);
        // La vista del core parla ancora del brano di prima: si aspetta la sua prossima pubblicazione.
        assert_eq!(wanted(Inputs { ready: ready(B, 4), ..on }), None);
    }

    #[test]
    fn a_new_clip_opens_a_better_one_opens_beside_and_nothing_closes_what_is_wanted() {
        let open = Current { track: A, seq: 1, open: true, upgrading: None };
        // Niente e niente: fermi. Qualcosa e niente: si chiude.
        assert_eq!(plan(None, None), Action::Keep);
        assert_eq!(plan(Some(open), None), Action::Close);
        // Il primo videoclip si apre.
        assert_eq!(plan(None, ready(A, 1)), Action::Open(Want { track: A, seq: 1 }));
        // Lo stesso: resta. Un seq nuovo dello stesso brano già aperto: la versione migliore di fianco.
        assert_eq!(plan(Some(open), ready(A, 1)), Action::Keep);
        assert_eq!(plan(Some(open), ready(A, 2)), Action::Upgrade(Want { track: A, seq: 2 }));
        // Mentre la versione migliore si apre non se ne apre un'altra copia.
        let upgrading = Current { upgrading: Some(2), ..open };
        assert_eq!(plan(Some(upgrading), ready(A, 2)), Action::Keep);
        // Un seq nuovo mentre la prima pipeline si sta ancora aprendo: si ricomincia con quello.
        let opening = Current { open: false, ..open };
        assert_eq!(plan(Some(opening), ready(A, 2)), Action::Open(Want { track: A, seq: 2 }));
        // Un altro brano: si chiude il vecchio e si apre il nuovo.
        assert_eq!(plan(Some(open), ready(B, 3)), Action::Open(Want { track: B, seq: 3 }));
    }

    #[derive(Default)]
    struct Fake {
        calls: Vec<String>,
    }

    impl Controls for Fake {
        fn set_paused(&mut self, paused: bool) {
            self.calls.push(format!("paused {paused}"));
        }
        fn seek(&mut self, position: Duration) {
            self.calls.push(format!("seek {}", position.as_millis()));
        }
    }

    #[test]
    fn the_core_decision_is_applied_pause_first_then_the_seek() {
        let mut player = Fake::default();
        apply(&mut player, Adjust::default());
        assert!(player.calls.is_empty(), "niente da fare, niente al player");
        apply(&mut player, Adjust { pause: Some(false), seek: Some(Duration::from_millis(1500)) });
        apply(&mut player, Adjust { pause: Some(true), seek: None });
        apply(&mut player, Adjust { pause: None, seek: Some(Duration::from_millis(40)) });
        assert_eq!(player.calls, ["paused false", "seek 1500", "paused true", "seek 40"]);
    }

    #[test]
    fn the_drift_check_rides_the_frames_at_most_every_tick() {
        let start = Instant::now();
        assert!(check_due(None, start), "il primo fotogramma controlla subito");
        assert!(!check_due(Some(start), start + Duration::from_millis(33)));
        assert!(!check_due(Some(start), start + CHECK_EVERY - Duration::from_millis(1)));
        assert!(check_due(Some(start), start + CHECK_EVERY));
        // Un orologio che torna indietro non rompe niente.
        assert!(!check_due(Some(start + CHECK_EVERY), start));
    }
}
