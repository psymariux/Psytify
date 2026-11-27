//! Gli effetti che il riduttore chiede all'host. Sono dati, non chiusure: i test controllano che
//! cosa è stato chiesto senza eseguirlo. L'host arma e annulla i timer, esegue sul runtime i lavori
//! asincroni dei domini ([`Job`]) e ne riporta l'esito al riduttore ([`Done`]).

use std::time::Duration;

use crate::error::AppError;
use crate::state::jam::JamOp;
use crate::state::playback::AudioOp;
use crate::state::system::{DiscordOp, MediaOp};
#[cfg(feature = "video")]
use crate::state::video;
use crate::state::{catalog, downloads, external, jam, library, playback, settings, social};

/// Un effetto da eseguire.
#[derive(Debug)]
pub(crate) enum Effect {
    /// Arma un timer che scade fra `after`. Riarmare la stessa chiave annulla il timer precedente:
    /// un debounce è un riarmo.
    Timer { key: TimerKey, after: Duration },
    /// Annulla un timer armato; se non c'è, niente.
    CancelTimer(TimerKey),
    /// Un'operazione sull'uscita audio. L'host la esegue subito, sul thread del core, e ciò che
    /// l'audio risponde torna nello stesso giro come `Input::Audio`.
    Audio(AudioOp),
    /// Il brano o il volume da mostrare al sistema: l'host lo manda al collegamento multimediale che
    /// tiene, se è pronto.
    Media(MediaOp),
    /// Accende, spegne o aggiorna Discord Rich Presence.
    Discord(DiscordOp),
    /// Un'operazione sul collegamento alla stanza della jam, che l'host tiene sul proprio thread: lo
    /// avvia, gli manda lo stato locale, lo chiude. Gli eventi della stanza tornano come `Input::Jam`.
    Jam(JamOp),
    /// Apre un indirizzo nel browser del sistema (l'accesso con GitHub, Google o Discord): l'host
    /// lo esegue subito con `platform::open_url`, e se non si apre risponde con
    /// `Done::Social(BrowserFailed)`. L'indirizzo è un `Secret`: nei log non compare.
    OpenUrl(crate::auth::Secret),
    /// Apre (`true`) o chiude il collegamento che ascolta il movimento chiesto dal sistema
    /// (`system_motion::events`), che l'host tiene sul thread del core: le letture tornano come
    /// `Input::SystemMotion`.
    WatchMotion(bool),
    /// Esegue un lavoro sul runtime; il suo esito torna al dominio come `Input::Done`.
    Run(Job),
    /// La chiusura chiesta con `Input::Shutdown` è pronta: `config.json` ha l'ultimo stato, o
    /// l'ultima scrittura è fallita per questo motivo. Dopo, il core si ferma.
    Flushed(Result<(), AppError>),
}

/// La chiave di un timer: il dominio e il timer, dichiarato nel modulo del dominio. Un dominio
/// toglie il suo `expect` quando arma il primo timer. La jam non ne ha: gira sugli eventi del suo
/// collegamento. Il videoclip ne ha uno solo, armato mentre un videoclip mostrato aspetta la
/// versione completa.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TimerKey {
    Settings(settings::Timer),
    #[expect(dead_code, reason = "nessun timer finché il dominio non passa nel core (passo 4.2)")]
    Library(library::Timer),
    Playback(playback::Timer),
    Catalog(catalog::Timer),
    External(external::Timer),
    Downloads(downloads::Timer),
    Social(social::Timer),
    #[cfg(feature = "video")]
    Video(video::Timer),
}

/// Un lavoro asincrono (rete, file): il dominio e il lavoro, dichiarato nel modulo del dominio.
/// Un dominio toglie il suo `expect` quando chiede il primo lavoro.
#[derive(Debug)]
pub(crate) enum Job {
    Settings(settings::Job),
    #[expect(dead_code, reason = "nessun lavoro finché il dominio non passa nel core (passo 4.2)")]
    Library(library::Job),
    Playback(playback::Job),
    Catalog(catalog::Job),
    External(external::Job),
    Downloads(downloads::Job),
    Social(social::Job),
    Jam(jam::Job),
    #[cfg(feature = "video")]
    Video(video::Job),
}

/// L'esito di un [`Job`], per il dominio che l'ha chiesto.
#[derive(Debug)]
pub(crate) enum Done {
    Settings(settings::Done),
    Library(library::Done),
    Playback(playback::Done),
    Catalog(catalog::Done),
    External(external::Done),
    Downloads(downloads::Done),
    Social(social::Done),
    Jam(jam::Done),
    #[cfg(feature = "video")]
    Video(video::Done),
}

impl Job {
    /// Esegue il lavoro. Gira sul runtime, fuori dal riduttore: è l'unico posto dove un dominio
    /// tocca la rete o i file.
    pub(crate) async fn run(self) -> Done {
        match self {
            Self::Settings(job) => Done::Settings(job.run().await),
            Self::Library(job) => Done::Library(job.run().await),
            Self::Playback(job) => Done::Playback(job.run().await),
            Self::Catalog(job) => Done::Catalog(job.run().await),
            Self::External(job) => Done::External(job.run().await),
            Self::Downloads(job) => Done::Downloads(job.run().await),
            Self::Social(job) => Done::Social(job.run().await),
            Self::Jam(job) => Done::Jam(job.run().await),
            #[cfg(feature = "video")]
            Self::Video(job) => Done::Video(job.run().await),
        }
    }

    /// L'esito da consegnare se il lavoro finisce in un panic: il dominio riceve comunque una
    /// risposta e non resta ad aspettare (alla chiusura, il core non si fermerebbe più).
    pub(crate) fn lost(&self) -> Done {
        match self {
            Self::Settings(job) => Done::Settings(job.lost()),
            Self::Library(job) => Done::Library(job.lost()),
            Self::Playback(job) => Done::Playback(job.lost()),
            Self::Catalog(job) => Done::Catalog(job.lost()),
            Self::External(job) => Done::External(job.lost()),
            Self::Downloads(job) => Done::Downloads(job.lost()),
            Self::Social(job) => Done::Social(job.lost()),
            Self::Jam(job) => Done::Jam(job.lost()),
            #[cfg(feature = "video")]
            Self::Video(job) => Done::Video(job.lost()),
        }
    }
}
