//! Download dei brani e aggiornamenti dell'app (passo 4.6).
//!
//! Il dominio tiene la coda dei download: ogni brano chiesto è un [`Item`] che passa da in coda a in
//! preparazione (il server sta scegliendo o ricodificando il file) a in scaricamento; finito bene
//! esce dalla coda, finito male resta come fallito, con l'errore, per riprovare. «Scarica tutto»
//! mette i suoi brani nella stessa coda e ne scarica due alla volta ([`BATCH_PARALLEL`]); un solo
//! brano chiesto a mano parte subito, come prima, anche se un lotto è in corso. Ogni download è un
//! lavoro ([`Job::Track`]) che scrive in un file temporaneo unico e lo pubblica a fine corsa
//! (`api::transfer`): un annullamento lo ferma e ne toglie il temporaneo.
//!
//! L'avanzamento non passa dal riduttore a ogni blocco: il lavoro conta i byte in un [`Progress`]
//! condiviso e un timer del core ([`Timer::Progress`], ogni 250 ms) lo legge. Il timer è armato solo
//! finché un download è in corso, e la vista cambia solo quando cambia un punto percentuale (o, senza
//! la lunghezza del file, ogni 256 KiB): fermi, o fra due punti, il core non pubblica niente.
//!
//! Ogni brano salvato entra nell'indice dei brani sul dispositivo (`offline.rs`: percorso, formato,
//! peso e SHA-256 calcolato mentre il file si scriveva), scritto in `config.json` con le altre
//! impostazioni. All'avvio un controllo veloce (esiste ed è del peso giusto) segna i file che non lo
//! sono ([`Lost`]): mai li cancella né li toglie dall'indice, e il brano si ascolta dalla rete. La
//! riproduzione riceve la mappa dei file a posto ([`Downloads::local`]) e suona da lì; se un file
//! non si apre lo segnala e il dominio lo segna perso. «Rimuovi» toglie la voce subito e il file
//! solo se pesa ancora quanto l'indice dice (un file cambiato fuori dall'app non si cancella).
//!
//! Il dominio controlla anche gli aggiornamenti dell'app (all'avvio, ogni sei ore con un timer del
//! core e a richiesta) e ne scarica il pacchetto nella cartella degli aggiornamenti. Ciò che il
//! frontend deve dire all'utente (brano salvato, lotto finito, release nuova) è un evento con il suo
//! numero d'ordine, come nel catalogo.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::api::update::{self, Offer};
use crate::api::{self, ApiClient, Cancel, DownloadQuality, Downloaded, Progress, Track};
use crate::config::{self, DownloadsFields};
use crate::effect::{self, Effect, TimerKey};
use crate::error::AppError;
use crate::offline::{self, LocalFile, Lost, LostReason, SavedTrack};
use crate::shared::Shared;

/// Quanti brani di «Scarica tutto» scaricano insieme: abbastanza per tenere occupata la rete, pochi
/// per non soffocare il telefono.
pub const BATCH_PARALLEL: usize = 2;
/// Ogni quanto si legge l'avanzamento dei download in corso.
pub const PROGRESS_EVERY: Duration = Duration::from_millis(250);
/// Brani in coda o in corso, al massimo: «Scarica tutto» su una libreria enorme non riempie la
/// memoria, e oltre il limite l'utente lo ripete a coda scesa.
pub const MAX_QUEUED: usize = 500;
/// Download falliti che restano nella coda, per riprovarli: oltre, esce il più vecchio.
pub const MAX_FAILED: usize = 20;
/// Senza la lunghezza del file, l'avanzamento cambia ogni tanti byte.
const UNKNOWN_STEP: u64 = 256 * 1024;
/// Eventi tenuti nella vista: un frontend che legge a ogni pubblicazione non ne perde.
const EVENTS: usize = 32;
/// Ogni quanto un'app rimasta aperta richiede l'ultima release. Una richiesta ogni sei ore: il
/// backend tiene la lista in cache per quindici minuti e GitHub non vede nulla.
pub const UPDATE_POLL: Duration = Duration::from_secs(6 * 60 * 60);

/// I comandi del dominio.
#[derive(Debug, Clone)]
pub enum DownloadsCmd {
    /// Scarica il brano nella qualità scelta nelle impostazioni, subito.
    Download(Box<Track>),
    /// Scarica il brano in questa qualità, subito.
    DownloadAs(Box<Track>, DownloadQuality),
    /// «Scarica tutto»: i brani che si possono scaricare entrano in coda, due alla volta.
    DownloadAll(Arc<[Track]>),
    /// Toglie dalla coda un brano in attesa, ferma uno in corso (il suo file temporaneo sparisce),
    /// o toglie dall'elenco uno fallito.
    Cancel(Uuid),
    /// Riprova un download fallito.
    Retry(Uuid),
    /// Cambia la qualità dei download.
    SetQuality(DownloadQuality),
    /// Toglie un brano dal dispositivo: la voce dell'indice sparisce subito, il file se è ancora
    /// quello scaricato.
    Remove(Uuid),
    /// Controlla a fondo i file scaricati (esistenza, peso e SHA-256): quelli che non sono a posto si
    /// segnano, quelli tornati a posto si liberano. All'avvio il core fa solo il controllo veloce.
    Verify,
    /// Apre la cartella dei download nel gestore di file del sistema.
    OpenFolder,
    /// Chiede al backend l'ultima release.
    CheckUpdate(UpdateCheck),
    /// Scarica il pacchetto dell'aggiornamento offerto.
    DownloadUpdate,
    /// Chiude il banner dell'aggiornamento: torna da solo con la prossima release.
    DismissUpdate,
    /// Apre nel gestore di file la cartella del pacchetto scaricato, per installarlo a mano
    /// (BACKLOG 39). Solo con un pacchetto pronto; non su Android, dove l'installazione la chiede il
    /// sistema. Installarlo e riavviare dall'app non si fa: è una decisione del proprietario (firma,
    /// permessi, file bloccati su Windows).
    RevealUpdate,
}

/// Chi ha chiesto il controllo: cambia cosa si dice quando non c'è nulla di nuovo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateCheck {
    /// All'avvio.
    Startup,
    /// Il timer, ad app aperta da ore.
    Poll,
    /// «Controlla ora» in Impostazioni: risponde sempre, anche «sei aggiornato» o un errore.
    Manual,
}

/// Com'è andato l'ultimo controllo, per la riga in Impostazioni.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpdateStatus {
    #[default]
    Idle,
    Checking,
    /// La release più recente è questa.
    Current,
    Failed,
}

/// L'aggiornamento proposto, e a che punto è.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateState {
    Offered(Offer),
    /// Il pacchetto di questa release si sta scaricando: `received` byte su `total`, se il server ha
    /// detto la lunghezza. Cambia al massimo una volta per punto percentuale (BACKLOG 38).
    Downloading {
        tag: String,
        received: u64,
        total: Option<u64>,
    },
    Downloaded(PathBuf),
}

impl UpdateState {
    /// L'avanzamento del pacchetto in punti percentuali, se si scarica e se ne sa la lunghezza.
    pub fn percent(&self) -> Option<u8> {
        match self {
            Self::Downloading { received, total: Some(total), .. } => Some(percent_of(*received, *total)),
            _ => None,
        }
    }
}

/// L'aggiornamento visto da fuori.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpdateView {
    pub state: Option<UpdateState>,
    pub status: UpdateStatus,
    /// La release di cui l'utente ha chiuso il banner.
    dismissed: Option<String>,
}

impl UpdateView {
    /// Il banner da mostrare, se c'è: un'offerta non chiusa, il download o il file pronto.
    pub fn banner(&self) -> Option<&UpdateState> {
        match self.state.as_ref()? {
            UpdateState::Offered(offer) if self.dismissed.as_deref() == Some(offer.tag.as_str()) => None,
            state => Some(state),
        }
    }

    /// Un download del pacchetto è in corso o già fatto: un controllo non lo tocca.
    fn busy(&self) -> bool {
        matches!(self.state, Some(UpdateState::Downloading { .. } | UpdateState::Downloaded(_)))
    }
}

/// A che punto è un brano della coda.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ItemState {
    /// Aspetta il suo turno (solo i brani di «Scarica tutto»).
    Queued,
    /// Il server sta preparando il file (scelta della versione, ricodifica): i byte non sono ancora
    /// cominciati.
    Preparing,
    /// I byte arrivano; `total` è la lunghezza annunciata, se il server l'ha detta.
    Downloading { received: u64, total: Option<u64> },
    /// Non è riuscito: l'errore, e «Riprova».
    Failed(AppError),
}

/// Un brano nella coda dei download.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub track: Arc<Track>,
    pub quality: DownloadQuality,
    pub state: ItemState,
    /// Viene da «Scarica tutto»: lo riassume l'avviso finale del lotto, non uno per brano.
    pub batched: bool,
}

impl Item {
    /// Il brano è in coda o in corso: non fallito.
    pub fn is_busy(&self) -> bool {
        !matches!(self.state, ItemState::Failed(_))
    }

    /// L'avanzamento in punti percentuali, se si sa la lunghezza del file.
    pub fn percent(&self) -> Option<u8> {
        match self.state {
            ItemState::Downloading { received, total: Some(total) } => Some(percent_of(received, total)),
            _ => None,
        }
    }
}

fn percent_of(received: u64, total: u64) -> u8 {
    if total == 0 {
        return 0;
    }
    (u128::from(received.min(total)) * 100 / u128::from(total)) as u8
}

/// Un «Scarica tutto» in corso: quanti brani aveva, quanti sono arrivati e quanti no.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Batch {
    pub total: usize,
    pub done: usize,
    pub failed: usize,
}

/// Perché un comando di download non ha fatto niente.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Questo brano non si può scaricare: la licenza non lo permette.
    NotDownloadable,
    /// Nessuno dei brani si può scaricare.
    NoneDownloadable,
    /// I brani sono già in coda o in corso.
    AlreadyQueued,
    /// La coda è piena ([`MAX_QUEUED`]).
    QueueFull,
    /// Il brano è già sul dispositivo e il suo file è a posto: per rifarlo si rimuove prima.
    AlreadySaved,
}

/// Qualcosa che il frontend deve far sapere o fare.
#[derive(Debug, Clone, PartialEq)]
pub enum DownloadsEvent {
    /// Un brano chiesto da solo è stato salvato.
    Saved(Downloaded),
    /// Un brano chiesto da solo non è stato salvato.
    Failed {
        id: Uuid,
        error: AppError,
    },
    /// L'ultimo brano di «Scarica tutto» è finito; `last_error` è l'ultimo errore, se qualcuno è
    /// fallito.
    BatchFinished {
        done: usize,
        failed: usize,
        last_error: Option<AppError>,
    },
    Refused(Refusal),
    /// Una release nuova da offrire. `notify` dice di avvisare anche fuori dall'app (sul telefono):
    /// solo dal controllo periodico e una volta per release.
    UpdateOffered {
        tag: String,
        notify: bool,
    },
    /// Il controllo a mano non ha trovato niente di nuovo.
    UpToDate,
    /// Il controllo a mano non è riuscito.
    UpdateCheckFailed(AppError),
    /// Il controllo a mano non è partito: il backend non è raggiungibile.
    Unreachable,
    /// Il pacchetto dell'aggiornamento è scaricato.
    UpdateSaved(PathBuf),
    /// Il pacchetto dell'aggiornamento non è arrivato.
    UpdateFailed(AppError),
    /// Un brano è stato tolto dal dispositivo. `kept_file`: il file non pesava più quanto l'indice
    /// ricordava (l'hanno cambiato fuori dall'app) e non è stato cancellato.
    Removed {
        title: String,
        kept_file: bool,
    },
    /// Il file di un brano tolto dall'indice non si è potuto cancellare.
    RemoveFailed {
        title: String,
        error: AppError,
    },
    /// Il controllo a fondo dei file è finito: quanti brani, e quanti non sono a posto.
    Verified {
        checked: usize,
        lost: usize,
    },
    /// La cartella dei download non si è aperta.
    FolderNotOpened,
    /// La cartella del pacchetto dell'aggiornamento non si è aperta.
    UpdateFolderNotOpened,
}

/// Un [`DownloadsEvent`] con il suo numero d'ordine, che cresce a ogni evento.
#[derive(Debug, Clone, PartialEq)]
pub struct DownloadsRaised {
    pub seq: u64,
    pub event: DownloadsEvent,
}

/// Ciò che i frontend vedono del dominio.
#[derive(Debug, Clone, Default)]
pub struct DownloadsView {
    /// La qualità dei download scelta nelle impostazioni.
    pub quality: DownloadQuality,
    /// La coda: in attesa, in corso e falliti, nell'ordine in cui sono stati chiesti.
    pub items: Shared<Item>,
    /// Il «Scarica tutto» in corso, se c'è.
    pub batch: Option<Batch>,
    pub update: UpdateView,
    /// I brani sul dispositivo, il più recente in testa. Non ha un limite: è la raccolta dell'utente,
    /// non una cache.
    pub saved: Shared<SavedTrack>,
    /// Gli id dei brani sul dispositivo, per chiedere «è scaricato?» riga per riga senza scorrere
    /// l'indice.
    pub saved_ids: Arc<HashSet<Uuid>>,
    /// I brani il cui file non è più a posto (mancante o cambiato): restano nell'indice, si
    /// ascoltano dalla rete.
    pub lost: Shared<Lost>,
    /// Byte occupati dai file dell'indice.
    pub used_bytes: u64,
    /// Un controllo a fondo dei file è in corso.
    pub verifying: bool,
    /// La cartella dove arrivano i download.
    pub folder: PathBuf,
    /// Gli ultimi eventi, dal più vecchio: un frontend mostra quelli con `seq` nuovo, in ordine. Se
    /// ne scorda al massimo [`EVENTS`] fra una lettura e l'altra, cioè mai.
    pub events: Arc<VecDeque<DownloadsRaised>>,
}

impl DownloadsView {
    /// Il brano nella coda, in qualunque stato.
    pub fn item(&self, id: Uuid) -> Option<&Item> {
        self.items.iter().find(|item| item.track.id == id)
    }

    /// Il brano è in coda o in corso (non fallito).
    pub fn is_busy(&self, id: Uuid) -> bool {
        self.item(id).is_some_and(Item::is_busy)
    }

    /// Quanti brani di «Scarica tutto» sono finiti e quanti erano in tutto.
    pub fn batch_progress(&self) -> Option<(usize, usize)> {
        self.batch.map(|batch| (batch.done + batch.failed, batch.total))
    }

    /// Quanti brani sono in coda o in corso.
    pub fn busy_count(&self) -> usize {
        self.items.iter().filter(|item| item.is_busy()).count()
    }

    /// Il brano è nell'indice dei brani sul dispositivo (anche se il suo file non è a posto).
    pub fn is_saved(&self, id: Uuid) -> bool {
        self.saved_ids.contains(&id)
    }

    /// La voce dell'indice del brano.
    pub fn saved_track(&self, id: Uuid) -> Option<&SavedTrack> {
        self.is_saved(id).then(|| self.saved.iter().find(|saved| saved.track.id == id)).flatten()
    }

    /// Perché il file del brano non è a posto, se non lo è.
    pub fn lost_reason(&self, id: Uuid) -> Option<LostReason> {
        self.lost.iter().find(|lost| lost.id == id).map(|lost| lost.reason)
    }

    /// Il brano suona senza rete: è sul dispositivo e il suo file è a posto (per quanto si sa).
    pub fn is_offline_ready(&self, id: Uuid) -> bool {
        self.is_saved(id) && self.lost_reason(id).is_none()
    }
}

/// Le cartelle dove il dominio scrive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Dirs {
    /// I brani scaricati.
    pub(crate) downloads: PathBuf,
    /// I pacchetti degli aggiornamenti.
    pub(crate) updates: PathBuf,
}

impl Default for Dirs {
    /// Cartelle sotto la temporanea: un riduttore di prova non scrive mai (i lavori si eseguono solo
    /// nell'host, che le imposta da sé).
    fn default() -> Self {
        let base = std::env::temp_dir();
        Self { downloads: base.join("psytify-downloads"), updates: base.join("psytify-updates") }
    }
}

/// I timer del dominio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Timer {
    /// Ogni [`PROGRESS_EVERY`], finché un download è in corso: si legge l'avanzamento.
    Progress,
    /// Ogni [`UPDATE_POLL`]: si controlla se c'è una release nuova.
    UpdatePoll,
}

/// I lavori asincroni del dominio (rete, file), che l'host esegue sul runtime.
#[derive(Debug)]
pub(crate) enum Job {
    /// Scarica un brano; `ticket` lo distingue da un download precedente dello stesso brano.
    Track {
        id: Uuid,
        ticket: u64,
        api: Result<ApiClient, AppError>,
        track: Arc<Track>,
        dir: PathBuf,
        quality: DownloadQuality,
        progress: Arc<Progress>,
        cancel: Cancel,
    },
    /// Toglie dal disco un file che nessuno vuole più (un download finito mentre lo si annullava).
    Discard(PathBuf),
    /// Legge l'ultima release.
    CheckUpdate { api: Result<ApiClient, AppError>, kind: UpdateCheck, build: Option<&'static str> },
    /// Scarica il pacchetto dell'aggiornamento.
    DownloadUpdate { api: Result<ApiClient, AppError>, offer: Offer, dir: PathBuf, progress: Arc<Progress> },
    /// Controlla i file dell'indice (`deep`: anche il contenuto).
    Verify { entries: Vec<(Uuid, LocalFile, String)>, deep: bool },
    /// Toglie dal disco il file di un brano tolto dall'indice, se pesa ancora `size`.
    Remove { path: PathBuf, size: u64, title: String },
    /// Apre la cartella dei download nel gestore di file.
    OpenFolder(PathBuf),
    /// Apre nel gestore di file la cartella del pacchetto dell'aggiornamento.
    RevealUpdate(PathBuf),
}

/// L'esito di un [`Job`], che torna al dominio.
#[derive(Debug)]
pub(crate) enum Done {
    Track {
        id: Uuid,
        ticket: u64,
        result: Result<Downloaded, AppError>,
    },
    Discarded,
    UpdateChecked {
        kind: UpdateCheck,
        result: Result<Option<Offer>, AppError>,
    },
    UpdateDownloaded(Result<PathBuf, AppError>),
    Verified {
        deep: bool,
        checked: usize,
        lost: Vec<Lost>,
    },
    /// `Ok(true)`: il file è rimasto dov'era perché non è più quello scaricato.
    Removed {
        title: String,
        result: Result<bool, AppError>,
    },
    FolderOpened(bool),
    UpdateRevealed(bool),
}

/// Un download che il core sta eseguendo: il numero che lo distingue, e come leggerne l'avanzamento
/// e fermarlo.
#[derive(Debug)]
struct Slot {
    ticket: u64,
    progress: Arc<Progress>,
    cancel: Cancel,
}

/// Lo stato del dominio: la vista pubblicata e ciò che resta dentro il core.
#[derive(Debug)]
pub(crate) struct Downloads {
    pub(crate) view: Arc<DownloadsView>,
    api: Result<ApiClient, AppError>,
    server: Option<String>,
    dirs: Dirs,
    /// La release di questa copia: senza, una compilazione locale non controlla da sola.
    build: Option<&'static str>,
    /// I download in corso.
    slots: HashMap<Uuid, Slot>,
    /// L'avanzamento del pacchetto dell'aggiornamento, finché si scarica: lo legge lo stesso timer dei
    /// brani.
    update_progress: Option<Arc<Progress>>,
    /// Il numero del prossimo download.
    tickets: u64,
    /// Il numero dell'ultimo evento.
    events: u64,
    /// Il timer dell'avanzamento è armato.
    ticking: bool,
    /// L'ultimo errore del lotto in corso, per l'avviso finale.
    batch_error: Option<AppError>,
    /// La release di cui si è già avvisato fuori dall'app.
    notified: Option<String>,
    /// La qualità o l'indice sono cambiati e le impostazioni non l'hanno ancora
    /// ([`Downloads::take_persist`]).
    persist: bool,
    /// I file da cui la riproduzione può suonare: i brani dell'indice il cui file non è segnato
    /// perso. Si rifà a ogni cambio dell'indice; chi riceve confronta il puntatore.
    local: Arc<HashMap<Uuid, LocalFile>>,
}

impl Default for Downloads {
    fn default() -> Self {
        Self::new(&DownloadsFields::default(), None, Dirs::default())
    }
}

impl Downloads {
    /// Il dominio con la qualità letta da `config.json`, il backend scelto nelle impostazioni e le
    /// cartelle decise dall'host.
    pub(crate) fn new(fields: &DownloadsFields, server: Option<String>, dirs: Dirs) -> Self {
        let view = DownloadsView {
            quality: fields.download_quality,
            saved: fields.downloaded.clone(),
            folder: dirs.downloads.clone(),
            ..DownloadsView::default()
        };
        let mut this = Self {
            view: Arc::new(view),
            api: ApiClient::new(&config::api_url(server.as_deref())),
            server,
            dirs,
            build: update::BUILD_TAG,
            slots: HashMap::new(),
            update_progress: None,
            tickets: 0,
            events: 0,
            ticking: false,
            batch_error: None,
            notified: None,
            persist: false,
            local: Arc::default(),
        };
        this.reindex();
        this
    }

    /// Le cartelle decise dall'host.
    pub(crate) fn set_dirs(&mut self, dirs: Dirs) {
        if self.view.folder != dirs.downloads {
            self.view_mut().folder.clone_from(&dirs.downloads);
        }
        self.dirs = dirs;
    }

    /// La release che questa copia dice di essere (per i test).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn with_build(mut self, build: Option<&'static str>) -> Self {
        self.build = build;
        self
    }

    fn view_mut(&mut self) -> &mut DownloadsView {
        Arc::make_mut(&mut self.view)
    }

    fn run(effects: &mut Vec<Effect>, job: Job) {
        effects.push(Effect::Run(effect::Job::Downloads(job)));
    }

    fn raise(&mut self, event: DownloadsEvent) {
        self.events += 1;
        let raised = DownloadsRaised { seq: self.events, event };
        let events = Arc::make_mut(&mut self.view_mut().events);
        events.push_back(raised);
        while events.len() > EVENTS {
            events.pop_front();
        }
    }

    // ---------- Passaggio fra i domini ----------

    /// I campi che il dominio possiede, per le impostazioni.
    pub(crate) fn fields(&self) -> DownloadsFields {
        DownloadsFields { download_quality: self.view.quality, downloaded: self.view.saved.clone() }
    }

    /// I file da cui suonare, per la riproduzione.
    pub(crate) fn local(&self) -> &Arc<HashMap<Uuid, LocalFile>> {
        &self.local
    }

    /// La riproduzione non ha potuto aprire questi file: si segnano persi (restano nell'indice).
    pub(crate) fn files_lost(&mut self, found: Vec<(Uuid, LostReason)>) {
        let fresh: Vec<Lost> = found
            .into_iter()
            .filter(|(id, _)| self.view.is_saved(*id) && self.view.lost_reason(*id).is_none())
            .map(|(id, reason)| Lost { id, reason })
            .collect();
        if fresh.is_empty() {
            return;
        }
        self.view_mut().lost.make_mut().extend(fresh);
        self.reindex();
    }

    /// L'indice o i file persi sono cambiati: si rifanno i dati che ne derivano (gli id, lo spazio
    /// occupato, la mappa per la riproduzione).
    fn reindex(&mut self) {
        let ids: HashSet<Uuid> = self.view.saved.iter().map(|saved| saved.track.id).collect();
        let used = self.view.saved.iter().map(|saved| saved.size).sum();
        let local: HashMap<Uuid, LocalFile> = self
            .view
            .saved
            .iter()
            .filter(|saved| self.view.lost_reason(saved.track.id).is_none())
            .map(|saved| (saved.track.id, saved.local()))
            .collect();
        let view = self.view_mut();
        view.saved_ids = Arc::new(ids);
        view.used_bytes = used;
        self.local = Arc::new(local);
    }

    /// Le impostazioni devono scrivere i campi del dominio: lo dice una volta per cambiamento.
    pub(crate) fn take_persist(&mut self) -> bool {
        std::mem::take(&mut self.persist)
    }

    /// Il backend scelto nelle impostazioni è cambiato: il client HTTP si rifà.
    pub(crate) fn set_server(&mut self, server: &Option<String>) {
        if self.server == *server {
            return;
        }
        self.server.clone_from(server);
        self.api = ApiClient::new(&config::api_url(server.as_deref()));
    }

    /// Il core è appena partito: una copia con la sua release controlla se ce n'è una nuova, e poi
    /// ogni sei ore.
    pub(crate) fn start(&mut self, effects: &mut Vec<Effect>) {
        // Un controllo veloce dei file scaricati: una `stat` per brano, fuori dal riduttore.
        self.verify(false, effects);
        if self.build.is_some() {
            self.check_update(UpdateCheck::Startup, effects);
            effects.push(Effect::Timer { key: TimerKey::Downloads(Timer::UpdatePoll), after: UPDATE_POLL });
        }
    }

    /// Il numero del download in corso di `id` (per i test).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn ticket(&self, id: Uuid) -> Option<u64> {
        self.slots.get(&id).map(|slot| slot.ticket)
    }

    /// L'avanzamento condiviso del download in corso di `id` (per i test).
    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn progress_of(&self, id: Uuid) -> Option<Arc<Progress>> {
        self.slots.get(&id).map(|slot| Arc::clone(&slot.progress))
    }

    // ---------- Comandi ----------

    pub(crate) fn apply(&mut self, command: DownloadsCmd, _now: Instant, effects: &mut Vec<Effect>) {
        match command {
            DownloadsCmd::Download(track) => {
                let quality = self.view.quality;
                self.download_one(*track, quality, effects);
            }
            DownloadsCmd::DownloadAs(track, quality) => self.download_one(*track, quality, effects),
            DownloadsCmd::DownloadAll(tracks) => self.download_all(&tracks, effects),
            DownloadsCmd::Cancel(id) => self.cancel(id, effects),
            DownloadsCmd::Retry(id) => self.retry(id, effects),
            DownloadsCmd::SetQuality(quality) => {
                if self.view.quality != quality {
                    self.view_mut().quality = quality;
                    self.persist = true;
                }
            }
            DownloadsCmd::Remove(id) => self.remove(id, effects),
            DownloadsCmd::Verify => self.verify(true, effects),
            DownloadsCmd::OpenFolder => Self::run(effects, Job::OpenFolder(self.dirs.downloads.clone())),
            DownloadsCmd::CheckUpdate(kind) => self.check_update(kind, effects),
            DownloadsCmd::DownloadUpdate => self.download_update(effects),
            DownloadsCmd::DismissUpdate => self.dismiss_update(),
            DownloadsCmd::RevealUpdate => self.reveal_update(effects),
        }
    }

    /// Un brano chiesto da solo: parte subito, anche se un lotto è in corso. Se è già in coda o in
    /// corso non fa niente; se era fallito, ricomincia.
    fn download_one(&mut self, track: Track, quality: DownloadQuality, effects: &mut Vec<Effect>) {
        // Il controllo sta qui: il player del telefono mostra il tasto anche per i brani che non si
        // possono scaricare, e dice perché.
        if !track.downloadable {
            self.raise(DownloadsEvent::Refused(Refusal::NotDownloadable));
            return;
        }
        if self.view.is_busy(track.id) {
            return;
        }
        if self.view.is_offline_ready(track.id) {
            self.raise(DownloadsEvent::Refused(Refusal::AlreadySaved));
            return;
        }
        self.forget_failed(track.id);
        if self.view.items.len() >= MAX_QUEUED {
            self.raise(DownloadsEvent::Refused(Refusal::QueueFull));
            return;
        }
        let id = track.id;
        self.view_mut().items.make_mut().push(Item {
            track: Arc::new(track),
            quality,
            state: ItemState::Queued,
            batched: false,
        });
        self.launch(id, effects);
    }

    /// «Scarica tutto»: i brani che si possono scaricare e non sono già in coda entrano in fondo.
    fn download_all(&mut self, tracks: &[Track], effects: &mut Vec<Effect>) {
        if !tracks.iter().any(|track| track.downloadable) {
            self.raise(DownloadsEvent::Refused(Refusal::NoneDownloadable));
            return;
        }
        let quality = self.view.quality;
        let mut added = 0;
        let mut already_saved = 0;
        let mut full = false;
        for track in tracks.iter().filter(|track| track.downloadable) {
            if self.view.is_busy(track.id) {
                continue;
            }
            if self.view.is_offline_ready(track.id) {
                already_saved += 1;
                continue;
            }
            self.forget_failed(track.id);
            if self.view.items.len() >= MAX_QUEUED {
                full = true;
                break;
            }
            self.view_mut().items.make_mut().push(Item {
                track: Arc::new(track.clone()),
                quality,
                state: ItemState::Queued,
                batched: true,
            });
            added += 1;
        }
        if added > 0 {
            let batch = self.view_mut().batch.get_or_insert_with(Batch::default);
            batch.total += added;
        }
        if full {
            self.raise(DownloadsEvent::Refused(Refusal::QueueFull));
        } else if added == 0 && self.view.batch.is_none() {
            let refusal = if already_saved > 0 { Refusal::AlreadySaved } else { Refusal::AlreadyQueued };
            self.raise(DownloadsEvent::Refused(refusal));
        }
        self.pump(effects);
    }

    /// Toglie dall'elenco un download fallito dello stesso brano, per rimetterlo in coda.
    fn forget_failed(&mut self, id: Uuid) {
        if self.view.item(id).is_some_and(|item| !item.is_busy()) {
            self.view_mut().items.make_mut().retain(|item| item.track.id != id);
        }
    }

    /// Fa partire i prossimi brani del lotto fino a [`BATCH_PARALLEL`] insieme; finiti tutti, un solo
    /// evento riassume com'è andata.
    fn pump(&mut self, effects: &mut Vec<Effect>) {
        let mut active = self.slots.keys().filter(|id| self.is_batched(**id)).count();
        while active < BATCH_PARALLEL {
            let next = self
                .view
                .items
                .iter()
                .find(|item| item.batched && item.state == ItemState::Queued)
                .map(|item| item.track.id);
            let Some(id) = next else { break };
            self.launch(id, effects);
            active += 1;
        }
        let batch_busy = self.view.items.iter().any(|item| item.batched && item.is_busy());
        if let Some(batch) = self.view.batch.filter(|_| !batch_busy) {
            let last_error = self.batch_error.take();
            self.view_mut().batch = None;
            // Un lotto annullato per intero non ha niente da riassumere.
            if batch.total > 0 {
                self.raise(DownloadsEvent::BatchFinished {
                    done: batch.done,
                    failed: batch.failed,
                    last_error,
                });
            }
        }
    }

    fn is_batched(&self, id: Uuid) -> bool {
        self.view.item(id).is_some_and(|item| item.batched)
    }

    /// Fa partire il lavoro del brano `id`, che è già nella coda come in attesa.
    fn launch(&mut self, id: Uuid, effects: &mut Vec<Effect>) {
        let Some(item) = self.view.item(id) else { return };
        let (track, quality) = (Arc::clone(&item.track), item.quality);
        self.tickets += 1;
        let slot = Slot { ticket: self.tickets, progress: Arc::default(), cancel: Cancel::default() };
        let job = Job::Track {
            id,
            ticket: slot.ticket,
            api: self.api.clone(),
            track,
            dir: self.dirs.downloads.clone(),
            quality,
            progress: Arc::clone(&slot.progress),
            cancel: slot.cancel.clone(),
        };
        self.slots.insert(id, slot);
        self.set_state(id, ItemState::Preparing);
        Self::run(effects, job);
        self.start_ticking(effects);
    }

    /// Arma il timer dell'avanzamento, se non lo è già.
    fn start_ticking(&mut self, effects: &mut Vec<Effect>) {
        if !self.ticking {
            self.ticking = true;
            effects.push(Effect::Timer { key: TimerKey::Downloads(Timer::Progress), after: PROGRESS_EVERY });
        }
    }

    /// Qualcosa si scarica: un brano o il pacchetto dell'aggiornamento.
    fn downloading(&self) -> bool {
        !self.slots.is_empty() || self.update_progress.is_some()
    }

    fn set_state(&mut self, id: Uuid, state: ItemState) {
        if let Some(item) = self.view_mut().items.make_mut().iter_mut().find(|item| item.track.id == id) {
            item.state = state;
        }
    }

    fn cancel(&mut self, id: Uuid, effects: &mut Vec<Effect>) {
        let Some(item) = self.view.item(id) else { return };
        let batched = item.batched && item.is_busy();
        if let Some(slot) = self.slots.remove(&id) {
            slot.cancel.cancel();
        }
        self.view_mut().items.make_mut().retain(|item| item.track.id != id);
        if batched && let Some(batch) = self.view_mut().batch.as_mut() {
            batch.total = batch.total.saturating_sub(1);
        }
        self.stop_ticking_if_idle(effects);
        self.pump(effects);
    }

    fn retry(&mut self, id: Uuid, effects: &mut Vec<Effect>) {
        let Some(item) = self.view.item(id).filter(|item| !item.is_busy()) else { return };
        let (track, quality) = (Arc::clone(&item.track), item.quality);
        self.view_mut().items.make_mut().retain(|item| item.track.id != id);
        self.view_mut().items.make_mut().push(Item {
            track,
            quality,
            state: ItemState::Queued,
            batched: false,
        });
        self.launch(id, effects);
    }

    fn stop_ticking_if_idle(&mut self, effects: &mut Vec<Effect>) {
        if !self.downloading() && self.ticking {
            self.ticking = false;
            effects.push(Effect::CancelTimer(TimerKey::Downloads(Timer::Progress)));
        }
    }

    // ---------- Timer ----------

    pub(crate) fn fire(&mut self, timer: Timer, _now: Instant, effects: &mut Vec<Effect>) {
        match timer {
            Timer::Progress => {
                self.ticking = false;
                self.read_progress();
                if self.downloading() {
                    self.ticking = true;
                    effects.push(Effect::Timer {
                        key: TimerKey::Downloads(Timer::Progress),
                        after: PROGRESS_EVERY,
                    });
                }
            }
            Timer::UpdatePoll => {
                self.check_update(UpdateCheck::Poll, effects);
                effects
                    .push(Effect::Timer { key: TimerKey::Downloads(Timer::UpdatePoll), after: UPDATE_POLL });
            }
        }
    }

    /// Legge l'avanzamento dei download in corso e cambia la vista solo dove un punto percentuale
    /// (o 256 KiB, senza la lunghezza) è passato.
    fn read_progress(&mut self) {
        let changes: Vec<(Uuid, ItemState)> = self
            .slots
            .iter()
            .filter_map(|(id, slot)| {
                let item = self.view.item(*id)?;
                let next = if slot.progress.started() {
                    ItemState::Downloading {
                        received: slot.progress.received(),
                        total: slot.progress.total(),
                    }
                } else {
                    ItemState::Preparing
                };
                moved(&item.state, &next).then_some((*id, next))
            })
            .collect();
        for (id, state) in changes {
            self.set_state(id, state);
        }
        // Il pacchetto dell'aggiornamento: come un brano, una pubblicazione per punto percentuale.
        if let (Some(progress), Some(UpdateState::Downloading { tag, received, total })) =
            (&self.update_progress, &self.view.update.state)
            && progress.started()
        {
            let (now, length) = (progress.received(), progress.total());
            if bytes_moved((*received, *total), (now, length)) {
                let tag = tag.clone();
                self.view_mut().update.state =
                    Some(UpdateState::Downloading { tag, received: now, total: length });
            }
        }
    }

    // ---------- Esiti ----------

    pub(crate) fn done(&mut self, done: Done, _now: Instant, effects: &mut Vec<Effect>) {
        match done {
            Done::Track { id, ticket, result } => self.track_done(id, ticket, result, effects),
            Done::Discarded => {}
            Done::UpdateChecked { kind, result } => self.update_checked(kind, result),
            Done::UpdateDownloaded(result) => self.update_downloaded(result, effects),
            Done::Verified { deep, checked, lost } => self.verified(deep, checked, lost),
            Done::Removed { title, result } => match result {
                Ok(kept_file) => self.raise(DownloadsEvent::Removed { title, kept_file }),
                Err(error) => self.raise(DownloadsEvent::RemoveFailed { title, error }),
            },
            Done::FolderOpened(opened) => {
                if !opened {
                    self.raise(DownloadsEvent::FolderNotOpened);
                }
            }
            Done::UpdateRevealed(opened) => {
                if !opened {
                    self.raise(DownloadsEvent::UpdateFolderNotOpened);
                }
            }
        }
    }

    fn track_done(
        &mut self,
        id: Uuid,
        ticket: u64,
        result: Result<Downloaded, AppError>,
        effects: &mut Vec<Effect>,
    ) {
        // Un esito di un download annullato (o rifatto da capo): il suo file, se è arrivato, non lo
        // vuole nessuno.
        if self.slots.get(&id).is_none_or(|slot| slot.ticket != ticket) {
            if let Ok(saved) = result {
                Self::run(effects, Job::Discard(saved.path));
            }
            return;
        }
        self.slots.remove(&id);
        let batched = self.is_batched(id);
        match result {
            Ok(saved) => {
                let track = self.view.item(id).map(|item| Arc::clone(&item.track));
                self.view_mut().items.make_mut().retain(|item| item.track.id != id);
                if let Some(track) = track {
                    self.add_saved(&track, &saved);
                }
                if batched {
                    if let Some(batch) = self.view_mut().batch.as_mut() {
                        batch.done += 1;
                    }
                } else {
                    self.raise(DownloadsEvent::Saved(saved));
                }
            }
            Err(error) => {
                self.set_state(id, ItemState::Failed(error.clone()));
                self.trim_failed();
                if batched {
                    if let Some(batch) = self.view_mut().batch.as_mut() {
                        batch.failed += 1;
                    }
                    self.batch_error = Some(error);
                } else {
                    self.raise(DownloadsEvent::Failed { id, error });
                }
            }
        }
        self.stop_ticking_if_idle(effects);
        self.pump(effects);
    }

    /// Tiene al massimo [`MAX_FAILED`] download falliti: oltre, esce il più vecchio.
    fn trim_failed(&mut self) {
        let failed = self.view.items.iter().filter(|item| !item.is_busy()).count();
        if failed <= MAX_FAILED {
            return;
        }
        let mut excess = failed - MAX_FAILED;
        self.view_mut().items.make_mut().retain(|item| {
            if excess > 0 && !item.is_busy() {
                excess -= 1;
                return false;
            }
            true
        });
    }

    // ---------- Indice dei brani sul dispositivo ----------

    /// Un brano scaricato entra in testa all'indice, al posto di una voce vecchia dello stesso brano
    /// (un file perso e riscaricato).
    fn add_saved(&mut self, track: &Track, saved: &Downloaded) {
        let entry = SavedTrack {
            track: track.clone(),
            path: saved.path.clone(),
            format: saved.format.clone(),
            quality: saved.delivered,
            size: saved.size,
            sha256: saved.sha256.clone(),
        };
        let view = self.view_mut();
        let list = view.saved.make_mut();
        list.retain(|old| old.track.id != track.id);
        list.insert(0, entry);
        if view.lost.iter().any(|lost| lost.id == track.id) {
            view.lost.make_mut().retain(|lost| lost.id != track.id);
        }
        self.persist = true;
        self.reindex();
    }

    /// Toglie un brano dal dispositivo: la voce sparisce subito, il file lo toglie un lavoro.
    fn remove(&mut self, id: Uuid, effects: &mut Vec<Effect>) {
        let Some(saved) = self.view.saved_track(id).cloned() else { return };
        let view = self.view_mut();
        view.saved.make_mut().retain(|entry| entry.track.id != id);
        if view.lost.iter().any(|lost| lost.id == id) {
            view.lost.make_mut().retain(|lost| lost.id != id);
        }
        self.persist = true;
        self.reindex();
        Self::run(
            effects,
            Job::Remove { path: saved.path, size: saved.size, title: saved.track.title.clone() },
        );
    }

    /// Controlla i file dell'indice fuori dal riduttore. `deep` legge anche il contenuto: lo chiede
    /// l'utente; all'avvio basta il controllo veloce.
    fn verify(&mut self, deep: bool, effects: &mut Vec<Effect>) {
        if self.view.verifying || self.view.saved.is_empty() {
            return;
        }
        let entries = self
            .view
            .saved
            .iter()
            .map(|saved| (saved.track.id, saved.local(), saved.sha256.clone()))
            .collect();
        if deep {
            self.view_mut().verifying = true;
        }
        Self::run(effects, Job::Verify { entries, deep });
    }

    /// Il controllo è tornato: i file non a posto si segnano (e quelli tornati a posto si liberano).
    fn verified(&mut self, deep: bool, checked: usize, mut lost: Vec<Lost>) {
        // Una voce tolta mentre si controllava non c'entra più.
        lost.retain(|entry| self.view.is_saved(entry.id));
        // Un file segnato perso dopo l'ultimo giro (la riproduzione non l'ha aperto) resta tale, se il
        // controllo veloce non lo vede: non c'è prova che sia tornato a posto senza leggerlo.
        if !deep {
            for known in self.view.lost.iter() {
                if !lost.iter().any(|entry| entry.id == known.id) {
                    lost.push(*known);
                }
            }
        }
        let count = lost.len();
        if deep {
            self.view_mut().verifying = false;
        }
        if self.view.lost.as_slice() != lost.as_slice() {
            self.view_mut().lost = lost.into();
            self.reindex();
        }
        if deep {
            self.raise(DownloadsEvent::Verified { checked, lost: count });
        }
    }

    // ---------- Aggiornamenti ----------

    /// Chiede al backend l'ultima release. Una compilazione locale (senza tag) non controlla da
    /// sola, perché non saprebbe cosa confrontare; a richiesta sì, e offre l'ultima release.
    fn check_update(&mut self, kind: UpdateCheck, effects: &mut Vec<Effect>) {
        if self.build.is_none() && kind != UpdateCheck::Manual {
            return;
        }
        if self.api.is_err() {
            if kind == UpdateCheck::Manual {
                self.raise(DownloadsEvent::Unreachable);
            }
            return;
        }
        self.set_status(UpdateStatus::Checking);
        Self::run(
            effects,
            Job::CheckUpdate { api: self.api.clone(), kind, build: Some(self.build.unwrap_or("")) },
        );
    }

    fn set_status(&mut self, status: UpdateStatus) {
        if self.view.update.status != status {
            self.view_mut().update.status = status;
        }
    }

    /// Il controllo è tornato: un'offerta nuova compare nel banner e in un avviso; senza offerta si
    /// resta com'eravamo, e a chi ha chiesto a mano si dice come stanno le cose.
    fn update_checked(&mut self, kind: UpdateCheck, result: Result<Option<Offer>, AppError>) {
        // Un download in corso o già fatto non si tocca.
        let busy = self.view.update.busy();
        match result {
            Ok(Some(offer)) => {
                self.set_status(UpdateStatus::Idle);
                if busy {
                    return;
                }
                let fresh = !matches!(&self.view.update.state, Some(UpdateState::Offered(known)) if known.tag == offer.tag);
                let tag = offer.tag.clone();
                let update = &mut self.view_mut().update;
                update.state = Some(UpdateState::Offered(offer));
                if fresh {
                    update.dismissed = None;
                    // L'avviso fuori dall'app serve a chi non guarda l'app: solo dal timer, quando
                    // l'app è rimasta aperta (o in sottofondo) per ore, e una volta per release.
                    let notify = kind == UpdateCheck::Poll && self.notified.as_deref() != Some(tag.as_str());
                    if notify {
                        self.notified = Some(tag.clone());
                    }
                    self.raise(DownloadsEvent::UpdateOffered { tag, notify });
                }
            }
            Ok(None) => {
                self.set_status(UpdateStatus::Current);
                if !busy {
                    self.view_mut().update.state = None;
                }
                if kind == UpdateCheck::Manual {
                    self.raise(DownloadsEvent::UpToDate);
                }
            }
            Err(error) => {
                self.set_status(UpdateStatus::Failed);
                if kind == UpdateCheck::Manual {
                    self.raise(DownloadsEvent::UpdateCheckFailed(error));
                }
            }
        }
    }

    /// Scarica il pacchetto dell'offerta, se c'è.
    fn download_update(&mut self, effects: &mut Vec<Effect>) {
        let (Some(UpdateState::Offered(offer)), Ok(_)) = (&self.view.update.state, &self.api) else {
            return;
        };
        let offer = offer.clone();
        let progress = Arc::new(Progress::default());
        self.update_progress = Some(Arc::clone(&progress));
        self.view_mut().update.state =
            Some(UpdateState::Downloading { tag: offer.tag.clone(), received: 0, total: None });
        Self::run(
            effects,
            Job::DownloadUpdate { api: self.api.clone(), offer, dir: self.dirs.updates.clone(), progress },
        );
        self.start_ticking(effects);
    }

    fn update_downloaded(&mut self, result: Result<PathBuf, AppError>, effects: &mut Vec<Effect>) {
        self.update_progress = None;
        self.stop_ticking_if_idle(effects);
        match result {
            Ok(path) => {
                self.view_mut().update.state = Some(UpdateState::Downloaded(path.clone()));
                self.raise(DownloadsEvent::UpdateSaved(path));
            }
            Err(error) => {
                self.view_mut().update.state = None;
                self.raise(DownloadsEvent::UpdateFailed(error));
            }
        }
    }

    /// La cartella del pacchetto pronto, nel gestore di file: quella del file (la cartella degli
    /// aggiornamenti, se il percorso non ne ha una). Senza un pacchetto pronto non fa niente.
    fn reveal_update(&mut self, effects: &mut Vec<Effect>) {
        let Some(UpdateState::Downloaded(path)) = &self.view.update.state else { return };
        let dir = path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .map_or_else(|| self.dirs.updates.clone(), std::path::Path::to_path_buf);
        Self::run(effects, Job::RevealUpdate(dir));
    }

    /// Il banner si può chiudere: torna da solo con la prossima release.
    fn dismiss_update(&mut self) {
        match &self.view.update.state {
            Some(UpdateState::Offered(offer)) => {
                let tag = offer.tag.clone();
                if self.view.update.dismissed.as_deref() != Some(tag.as_str()) {
                    self.view_mut().update.dismissed = Some(tag);
                }
            }
            Some(UpdateState::Downloaded(_)) => self.view_mut().update.state = None,
            _ => {}
        }
    }
}

/// L'avanzamento è cambiato abbastanza da pubblicarlo: lo stato è un altro, o un punto percentuale
/// (senza lunghezza, 256 KiB) è passato.
fn moved(old: &ItemState, new: &ItemState) -> bool {
    match (old, new) {
        (
            ItemState::Downloading { received: before, total: a },
            ItemState::Downloading { received: now, total: b },
        ) => bytes_moved((*before, *a), (*now, *b)),
        (a, b) => a != b,
    }
}

/// Fra due letture di byte ricevuti e lunghezza: la lunghezza è un'altra, o è passato un punto
/// percentuale (senza lunghezza, 256 KiB).
fn bytes_moved(before: (u64, Option<u64>), now: (u64, Option<u64>)) -> bool {
    match (before, now) {
        ((before, Some(a)), (now, Some(b))) => a != b || percent_of(before, a) != percent_of(now, b),
        ((before, None), (now, None)) => before / UNKNOWN_STEP != now / UNKNOWN_STEP,
        _ => true,
    }
}

impl Job {
    pub(crate) async fn run(self) -> Done {
        match self {
            Self::Track { id, ticket, api, track, dir, quality, progress, cancel } => {
                let download = async {
                    match api {
                        Ok(api) => api::download_track(api, (*track).clone(), dir, quality, progress).await,
                        Err(error) => Err(error),
                    }
                };
                let result = tokio::select! {
                    () = cancel.cancelled() => Err(AppError::Cancelled),
                    result = download => result,
                };
                // Annullato un istante dopo la fine: il file è già al suo posto, e non lo si vuole.
                let result = match result {
                    Ok(saved) if cancel.is_cancelled() => {
                        let _ = tokio::fs::remove_file(&saved.path).await;
                        Err(AppError::Cancelled)
                    }
                    other => other,
                };
                Done::Track { id, ticket, result }
            }
            Self::Discard(path) => {
                let _ = tokio::fs::remove_file(&path).await;
                Done::Discarded
            }
            Self::CheckUpdate { api, kind, build } => {
                let result = match api {
                    Ok(api) => api.latest_release().await.map(|release| update::offer(release, build)),
                    Err(error) => Err(error),
                };
                Done::UpdateChecked { kind, result }
            }
            Self::DownloadUpdate { api, offer, dir, progress } => {
                let result = match api {
                    Ok(api) => api.download_release(&offer.asset, &dir, &progress).await,
                    Err(error) => Err(error),
                };
                Done::UpdateDownloaded(result)
            }
            Self::Verify { entries, deep } => {
                let checked = entries.len();
                // Legge il disco (e, a fondo, ogni file): fuori dal runtime.
                let lost = tokio::task::spawn_blocking(move || offline::verify(&entries, deep))
                    .await
                    .unwrap_or_default();
                Done::Verified { deep, checked, lost }
            }
            Self::Remove { path, size, title } => {
                let result = remove_saved(&path, size).await;
                Done::Removed { title, result }
            }
            Self::OpenFolder(dir) => Done::FolderOpened(open_folder(dir).await),
            // La cartella c'è già (il pacchetto ci sta): se nel frattempo è sparita, si ricrea vuota e
            // si apre lo stesso, come quella dei download.
            Self::RevealUpdate(dir) => Done::UpdateRevealed(open_folder(dir).await),
        }
    }

    /// Se il lavoro va perso, l'esito è un errore di rete: un download non resta «in corso» per
    /// sempre, e il controllo degli aggiornamenti non resta «in corso».
    pub(crate) fn lost(&self) -> Done {
        match self {
            Self::Track { id, ticket, .. } => {
                Done::Track { id: *id, ticket: *ticket, result: Err(AppError::Network) }
            }
            Self::Discard(_) => Done::Discarded,
            Self::CheckUpdate { kind, .. } => {
                Done::UpdateChecked { kind: *kind, result: Err(AppError::Network) }
            }
            Self::DownloadUpdate { .. } => Done::UpdateDownloaded(Err(AppError::Network)),
            // Un controllo perso non segna niente: nessuna prova, nessun file perso.
            Self::Verify { entries, deep } => {
                Done::Verified { deep: *deep, checked: entries.len(), lost: Vec::new() }
            }
            Self::Remove { title, .. } => {
                Done::Removed { title: title.clone(), result: Err(AppError::Io("job lost".into())) }
            }
            Self::OpenFolder(_) => Done::FolderOpened(false),
            Self::RevealUpdate(_) => Done::UpdateRevealed(false),
        }
    }
}

/// Apre `dir` nel gestore di file del sistema (`platform::open_path`, mai su Android), creandola se
/// manca. Dice se si è aperta.
async fn open_folder(dir: PathBuf) -> bool {
    match tokio::fs::create_dir_all(&dir).await {
        Ok(()) => {
            tokio::task::spawn_blocking(move || crate::platform::open_path(&dir)).await.unwrap_or(false)
        }
        Err(_) => false,
    }
}

/// Toglie dal disco il file di un brano tolto dall'indice, ma solo se è ancora quello scaricato (stesso
/// peso): un file cambiato fuori dall'app non è nostro e resta. `Ok(true)`: è rimasto; un file che non
/// c'è più è già com'era voluto.
async fn remove_saved(path: &std::path::Path, size: u64) -> Result<bool, AppError> {
    let io = |err: std::io::Error| AppError::Io(err.to_string());
    match tokio::fs::metadata(path).await {
        Ok(meta) if meta.is_file() && meta.len() == size => {
            tokio::fs::remove_file(path).await.map_err(io)?;
            Ok(false)
        }
        Ok(_) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(io(err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::models::fixtures;
    use crate::api::update::Asset;
    use crate::command::Command;
    use crate::snapshot::DomainVersions;
    use crate::state::{CoreState, Input};

    fn track(n: u128) -> Track {
        Track { downloadable: true, ..fixtures::track(n) }
    }

    fn id(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }

    fn saved(n: u128) -> Downloaded {
        Downloaded {
            path: PathBuf::from(format!("{n}.flac")),
            size: 4096,
            sha256: format!("hash{n}"),
            format: "flac".into(),
            delivered: None,
            asked: DownloadQuality::Lossless,
        }
    }

    fn offer(tag: &str) -> Offer {
        Offer { tag: tag.into(), asset: Asset { name: format!("psytify-{tag}.bin"), size: 1 } }
    }

    /// Il dominio da solo, con un orologio finto e senza release (una compilazione locale).
    /// BACKLOG 38: il pacchetto dell'aggiornamento ha il suo avanzamento, letto dallo stesso timer da
    /// 250 ms dei brani (armato solo mentre si scarica) e pubblicato una volta per punto percentuale;
    /// a fine corsa il timer si spegne e un'altra scadenza non pubblica niente.
    #[test]
    fn the_update_package_progress_is_read_on_the_timer_and_published_per_whole_percent() {
        let mut state = CoreState::default();
        let mut now = Instant::now();
        let send = |state: &mut CoreState, input: Input, now: Instant| state.apply(input, now);
        let done = |done| Input::Done(effect::Done::Downloads(done));
        send(
            &mut state,
            done(Done::UpdateChecked { kind: UpdateCheck::Manual, result: Ok(Some(offer("v2"))) }),
            now,
        );
        let offered = state.snapshot();
        assert!(
            timers(&send(&mut state, Input::Timer(TimerKey::Downloads(Timer::Progress)), now)).is_empty()
        );
        assert!(state.publish(&offered).is_none(), "nessun download: il timer non c'è e non pubblica");

        let effects = send(&mut state, Input::Command(Command::Downloads(DownloadsCmd::DownloadUpdate)), now);
        assert_eq!(timers(&effects), [(Timer::Progress, Some(PROGRESS_EVERY))]);
        let progress = match jobs(effects).as_slice() {
            [Job::DownloadUpdate { progress, offer, .. }] if offer.tag == "v2" => Arc::clone(progress),
            other => panic!("one update job, got {other:?}"),
        };
        let started = state.publish(&offered).expect("the download appears");
        assert_eq!(
            started.downloads.update.state,
            Some(UpdateState::Downloading { tag: "v2".into(), received: 0, total: None })
        );

        // Il server non ha ancora risposto: la lettura non cambia niente.
        now += PROGRESS_EVERY;
        let effects = send(&mut state, Input::Timer(TimerKey::Downloads(Timer::Progress)), now);
        assert_eq!(timers(&effects), [(Timer::Progress, Some(PROGRESS_EVERY))], "si riarma mentre scarica");
        assert!(state.publish(&started).is_none());

        let mut last = started;
        let mut published = 0;
        for step in 0..=1000u64 {
            progress.set(step, Some(1000));
            now += PROGRESS_EVERY;
            send(&mut state, Input::Timer(TimerKey::Downloads(Timer::Progress)), now);
            if let Some(next) = state.publish(&last) {
                published += 1;
                last = next;
            }
        }
        assert_eq!(published, 101, "lo 0 % e poi un punto alla volta");
        assert_eq!(last.downloads.update.state.as_ref().and_then(UpdateState::percent), Some(100));

        let path = PathBuf::from("/dati/updates/psytify-v2.bin");
        let effects = send(&mut state, done(Done::UpdateDownloaded(Ok(path.clone()))), now);
        assert_eq!(timers(&effects), [(Timer::Progress, None)], "finito, il timer si spegne");
        let finished = state.publish(&last).expect("the package is ready");
        assert_eq!(finished.downloads.update.state, Some(UpdateState::Downloaded(path)));
        let effects = send(&mut state, Input::Timer(TimerKey::Downloads(Timer::Progress)), now);
        assert!(effects.is_empty());
        assert!(state.publish(&finished).is_none());
    }

    /// Un brano e il pacchetto si scaricano insieme: il timer resta finché uno dei due è in corso, e
    /// senza la lunghezza il pacchetto avanza ogni 256 KiB.
    #[test]
    fn the_progress_timer_waits_for_both_a_track_and_the_update_package() {
        let mut rig = Rig::with_build("v1");
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Startup, result: Ok(Some(offer("v2"))) });
        assert_eq!(
            timers(&rig.cmd(DownloadsCmd::Download(Box::new(track(1))))),
            [(Timer::Progress, Some(PROGRESS_EVERY))]
        );
        let effects = rig.cmd(DownloadsCmd::DownloadUpdate);
        assert!(timers(&effects).is_empty(), "già armato dal brano");
        let Some(progress) = rig.downloads.update_progress.clone() else { panic!("the update progress") };

        progress.set(UNKNOWN_STEP - 1, None);
        rig.fire(Timer::Progress);
        assert_eq!(
            rig.view().update.state,
            Some(UpdateState::Downloading { tag: "v2".into(), received: 0, total: None }),
            "meno di 256 KiB: niente da pubblicare"
        );
        progress.set(UNKNOWN_STEP + 10, None);
        let before = rig.view();
        rig.fire(Timer::Progress);
        assert!(!Arc::ptr_eq(&before, &rig.view()), "256 KiB passati: si pubblica");
        progress.set(UNKNOWN_STEP + 20, None);
        let before = rig.view();
        rig.fire(Timer::Progress);
        assert!(Arc::ptr_eq(&before, &rig.view()), "fra due passi niente");
        assert_eq!(rig.view().update.state.as_ref().and_then(UpdateState::percent), None);

        assert!(timers(&rig.finish(1, Ok(saved(1)))).is_empty(), "il pacchetto scarica ancora");
        assert_eq!(timers(&rig.fire(Timer::Progress)), [(Timer::Progress, Some(PROGRESS_EVERY))]);
        let effects = rig.done(Done::UpdateDownloaded(Err(AppError::Network)));
        assert_eq!(timers(&effects), [(Timer::Progress, None)]);
        assert!(rig.downloads.update_progress.is_none());
    }

    struct Rig {
        downloads: Downloads,
        now: Instant,
    }

    impl Rig {
        fn new() -> Self {
            Self { downloads: Downloads::default().with_build(None), now: Instant::now() }
        }

        fn with_build(build: &'static str) -> Self {
            Self { downloads: Downloads::default().with_build(Some(build)), now: Instant::now() }
        }

        fn cmd(&mut self, command: DownloadsCmd) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.downloads.apply(command, self.now, &mut effects);
            effects
        }

        fn done(&mut self, done: Done) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.downloads.done(done, self.now, &mut effects);
            effects
        }

        fn fire(&mut self, timer: Timer) -> Vec<Effect> {
            let mut effects = Vec::new();
            self.downloads.fire(timer, self.now, &mut effects);
            effects
        }

        /// Il download in corso del brano `n` finisce con `result`.
        fn finish(&mut self, n: u128, result: Result<Downloaded, AppError>) -> Vec<Effect> {
            let ticket = self.downloads.ticket(id(n)).expect("a running download");
            self.done(Done::Track { id: id(n), ticket, result })
        }

        fn view(&self) -> Arc<DownloadsView> {
            Arc::clone(&self.downloads.view)
        }

        /// I brani in coda o in corso, in ordine.
        fn busy(&self) -> Vec<Uuid> {
            let mut ids: Vec<Uuid> =
                self.view().items.iter().filter(|item| item.is_busy()).map(|item| item.track.id).collect();
            ids.sort();
            ids
        }

        /// I brani il cui lavoro è partito, in ordine.
        fn running(&self) -> Vec<Uuid> {
            let mut ids: Vec<Uuid> = self.downloads.slots.keys().copied().collect();
            ids.sort();
            ids
        }

        fn events(&self) -> Vec<DownloadsEvent> {
            self.view().events.iter().map(|raised| raised.event.clone()).collect()
        }
    }

    fn jobs(effects: Vec<Effect>) -> Vec<Job> {
        effects
            .into_iter()
            .filter_map(|effect| match effect {
                Effect::Run(effect::Job::Downloads(job)) => Some(job),
                _ => None,
            })
            .collect()
    }

    fn timers(effects: &[Effect]) -> Vec<(Timer, Option<Duration>)> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::Timer { key: TimerKey::Downloads(timer), after } => Some((*timer, Some(*after))),
                Effect::CancelTimer(TimerKey::Downloads(timer)) => Some((*timer, None)),
                _ => None,
            })
            .collect()
    }

    // ---------- Coda e lotti ----------

    /// `download_all_runs_two_at_a_time_and_reports_once` (era nel client): «Scarica tutto» scarica
    /// due brani alla volta, quello che non si può scaricare non conta, e un solo evento riassume.
    #[test]
    fn download_all_runs_two_at_a_time_and_reports_once() {
        let mut rig = Rig::new();
        let mut tracks: Vec<Track> = (1..=5).map(track).collect();
        tracks[4].downloadable = false;

        let effects = rig.cmd(DownloadsCmd::DownloadAll(Arc::from(tracks)));
        assert_eq!(jobs(effects).len(), 2, "due alla volta");
        assert_eq!(rig.view().batch_progress(), Some((0, 4)), "il brano che non si può scaricare non conta");
        assert_eq!(rig.running(), [id(1), id(2)]);
        assert_eq!(rig.busy(), [id(1), id(2), id(3), id(4)]);

        rig.finish(1, Ok(saved(1)));
        assert_eq!((rig.view().batch_progress(), rig.running()), (Some((1, 4)), vec![id(2), id(3)]));
        rig.finish(2, Err(AppError::Network));
        assert_eq!((rig.view().batch_progress(), rig.running()), (Some((2, 4)), vec![id(3), id(4)]));
        rig.finish(3, Ok(saved(3)));
        assert_eq!((rig.view().batch_progress(), rig.running()), (Some((3, 4)), vec![id(4)]));
        assert!(rig.events().is_empty(), "nessun avviso brano per brano");

        // Un solo evento alla fine, con l'ultimo errore.
        let last = rig.finish(4, Ok(saved(4)));
        assert_eq!((rig.view().batch_progress(), rig.running()), (None, Vec::new()));
        assert_eq!(
            rig.events(),
            [DownloadsEvent::BatchFinished { done: 3, failed: 1, last_error: Some(AppError::Network) }]
        );
        assert_eq!(timers(&last), [(Timer::Progress, None)], "fermo, il timer dell'avanzamento si spegne");
        // Il brano fallito resta nell'elenco per riprovare, e non conta come in corso.
        assert!(matches!(rig.view().item(id(2)).map(|item| &item.state), Some(ItemState::Failed(_))));
        assert!(!rig.view().is_busy(id(2)));
    }

    #[test]
    fn a_single_download_starts_at_once_even_during_a_batch_and_is_reported_alone() {
        let mut rig = Rig::new();
        let tracks: Vec<Track> = (1..=3).map(track).collect();
        rig.cmd(DownloadsCmd::DownloadAll(Arc::from(tracks)));
        assert_eq!(rig.running(), [id(1), id(2)]);

        let effects = rig.cmd(DownloadsCmd::Download(Box::new(track(9))));
        assert_eq!(jobs(effects).len(), 1, "un brano chiesto a mano non aspetta il lotto");
        assert_eq!(rig.running(), [id(1), id(2), id(9)]);
        assert_eq!(rig.view().batch_progress(), Some((0, 3)), "e non entra nel conto del lotto");

        rig.finish(9, Ok(saved(9)));
        assert_eq!(rig.events(), [DownloadsEvent::Saved(saved(9))]);
        assert!(!rig.view().is_busy(id(9)));
        assert_eq!(rig.running(), [id(1), id(2)], "il lotto non ha preso il posto lasciato libero");
    }

    #[test]
    fn download_as_asks_for_that_quality_and_download_for_the_settings_one() {
        let mut rig = Rig::new();
        let ask = |effects: Vec<Effect>| match jobs(effects).as_slice() {
            [Job::Track { quality, .. }] => *quality,
            other => panic!("{other:?}"),
        };
        rig.cmd(DownloadsCmd::SetQuality(DownloadQuality::Low));
        assert_eq!(ask(rig.cmd(DownloadsCmd::Download(Box::new(track(1))))), DownloadQuality::Low);
        assert_eq!(
            ask(rig.cmd(DownloadsCmd::DownloadAs(Box::new(track(2)), DownloadQuality::Lossless))),
            DownloadQuality::Lossless
        );
    }

    #[test]
    fn a_track_that_cannot_be_downloaded_is_refused_and_a_busy_one_is_ignored() {
        let mut rig = Rig::new();
        let locked = fixtures::track(1);
        assert!(jobs(rig.cmd(DownloadsCmd::Download(Box::new(locked.clone())))).is_empty());
        assert!(jobs(rig.cmd(DownloadsCmd::DownloadAll(Arc::from(vec![locked])))).is_empty());
        assert_eq!(
            rig.events(),
            [
                DownloadsEvent::Refused(Refusal::NotDownloadable),
                DownloadsEvent::Refused(Refusal::NoneDownloadable)
            ]
        );
        assert!(rig.view().items.is_empty() && rig.view().batch.is_none());

        // Già in corso: né un secondo lavoro né un avviso.
        assert_eq!(jobs(rig.cmd(DownloadsCmd::Download(Box::new(track(2))))).len(), 1);
        assert!(jobs(rig.cmd(DownloadsCmd::Download(Box::new(track(2))))).is_empty());
        assert_eq!(rig.events().len(), 2);
        // «Scarica tutto» di brani già in corso e senza un lotto: lo dice.
        assert!(jobs(rig.cmd(DownloadsCmd::DownloadAll(Arc::from(vec![track(2)])))).is_empty());
        assert_eq!(rig.events().last(), Some(&DownloadsEvent::Refused(Refusal::AlreadyQueued)));
        assert!(rig.view().batch.is_none());
    }

    /// Una seconda «Scarica tutto» aggiunge al lotto in corso solo i brani nuovi.
    #[test]
    fn a_second_download_all_extends_the_running_batch() {
        let mut rig = Rig::new();
        rig.cmd(DownloadsCmd::DownloadAll(Arc::from((1..=3).map(track).collect::<Vec<_>>())));
        rig.cmd(DownloadsCmd::DownloadAll(Arc::from((2..=5).map(track).collect::<Vec<_>>())));
        assert_eq!(rig.view().batch_progress(), Some((0, 5)));
        assert_eq!(rig.view().busy_count(), 5);
        assert_eq!(rig.running(), [id(1), id(2)]);
    }

    #[test]
    fn the_queue_is_bounded() {
        let mut rig = Rig::new();
        let many: Vec<Track> = (1..=(MAX_QUEUED as u128 + 50)).map(track).collect();
        rig.cmd(DownloadsCmd::DownloadAll(Arc::from(many)));
        assert_eq!(rig.view().items.len(), MAX_QUEUED);
        assert_eq!(rig.view().batch_progress(), Some((0, MAX_QUEUED)));
        assert_eq!(rig.events(), [DownloadsEvent::Refused(Refusal::QueueFull)]);
        assert!(jobs(rig.cmd(DownloadsCmd::Download(Box::new(track(9999))))).is_empty());
        assert_eq!(rig.events().last(), Some(&DownloadsEvent::Refused(Refusal::QueueFull)));
    }

    // ---------- Annullo, errori, nuovo tentativo ----------

    #[test]
    fn a_cancelled_download_is_stopped_removed_and_its_late_file_discarded() {
        let mut rig = Rig::new();
        let effects = rig.cmd(DownloadsCmd::DownloadAll(Arc::from((1..=3).map(track).collect::<Vec<_>>())));
        let (cancel, ticket) = match jobs(effects).remove(0) {
            Job::Track { cancel, ticket, .. } => (cancel, ticket),
            other => panic!("{other:?}"),
        };
        assert!(!cancel.is_cancelled());

        let effects = rig.cmd(DownloadsCmd::Cancel(id(1)));
        assert!(cancel.is_cancelled(), "il lavoro lo vede");
        assert!(!rig.view().is_busy(id(1)) && rig.view().item(id(1)).is_none());
        assert_eq!(rig.view().batch_progress(), Some((0, 2)), "il brano tolto non conta più");
        assert_eq!(jobs(effects).len(), 1, "il posto libero va al brano successivo");
        assert_eq!(rig.running(), [id(2), id(3)]);

        // Il lavoro era già alla fine: il suo file non lo vuole nessuno.
        let late = rig.done(Done::Track { id: id(1), ticket, result: Ok(saved(1)) });
        assert!(matches!(jobs(late).as_slice(), [Job::Discard(path)] if path == &PathBuf::from("1.flac")));
        assert!(rig.events().is_empty());
        // Un esito di errore dello stesso lavoro non fa niente.
        assert!(rig.done(Done::Track { id: id(1), ticket, result: Err(AppError::Cancelled) }).is_empty());
    }

    #[test]
    fn a_stale_result_does_not_touch_a_download_started_again() {
        let mut rig = Rig::new();
        let first = match jobs(rig.cmd(DownloadsCmd::Download(Box::new(track(1))))).remove(0) {
            Job::Track { ticket, .. } => ticket,
            other => panic!("{other:?}"),
        };
        rig.cmd(DownloadsCmd::Cancel(id(1)));
        rig.cmd(DownloadsCmd::Download(Box::new(track(1))));
        assert!(rig.view().is_busy(id(1)));
        let stale = rig.done(Done::Track { id: id(1), ticket: first, result: Err(AppError::Network) });
        assert!(stale.is_empty());
        assert!(rig.view().is_busy(id(1)), "il nuovo download continua");
        assert!(rig.events().is_empty());
    }

    #[test]
    fn cancelling_every_item_of_a_batch_ends_it_silently() {
        let mut rig = Rig::new();
        rig.cmd(DownloadsCmd::DownloadAll(Arc::from((1..=2).map(track).collect::<Vec<_>>())));
        rig.cmd(DownloadsCmd::Cancel(id(1)));
        let effects = rig.cmd(DownloadsCmd::Cancel(id(2)));
        assert!(rig.view().batch.is_none() && rig.view().items.is_empty());
        assert!(rig.events().is_empty(), "niente da riassumere");
        assert_eq!(timers(&effects), [(Timer::Progress, None)]);
    }

    #[test]
    fn a_failed_single_download_stays_to_be_retried() {
        let mut rig = Rig::new();
        rig.cmd(DownloadsCmd::Download(Box::new(track(1))));
        rig.finish(1, Err(AppError::Timeout));
        assert_eq!(rig.events(), [DownloadsEvent::Failed { id: id(1), error: AppError::Timeout }]);
        assert!(matches!(
            rig.view().item(id(1)).map(|item| &item.state),
            Some(ItemState::Failed(AppError::Timeout))
        ));

        let effects = rig.cmd(DownloadsCmd::Retry(id(1)));
        assert_eq!(jobs(effects).len(), 1);
        assert!(rig.view().is_busy(id(1)));
        rig.finish(1, Ok(saved(1)));
        assert!(rig.view().items.is_empty());
        // Riprovare ciò che non è fallito non fa niente.
        assert!(rig.cmd(DownloadsCmd::Retry(id(1))).is_empty());
    }

    #[test]
    fn a_failed_item_can_be_dismissed_and_failed_items_are_bounded() {
        let mut rig = Rig::new();
        for n in 1..=(MAX_FAILED as u128 + 5) {
            rig.cmd(DownloadsCmd::Download(Box::new(track(n))));
            rig.finish(n, Err(AppError::Network));
        }
        assert_eq!(rig.view().items.len(), MAX_FAILED);
        assert!(rig.view().item(id(1)).is_none(), "esce il più vecchio");
        assert!(rig.view().item(id(MAX_FAILED as u128 + 5)).is_some());
        let last = id(MAX_FAILED as u128 + 5);
        rig.cmd(DownloadsCmd::Cancel(last));
        assert!(rig.view().item(last).is_none());
    }

    #[test]
    fn a_lost_job_fails_the_download_instead_of_leaving_it_running() {
        let mut rig = Rig::new();
        let job = jobs(rig.cmd(DownloadsCmd::Download(Box::new(track(1))))).remove(0);
        rig.done(job.lost());
        assert!(!rig.view().is_busy(id(1)));
        assert!(matches!(rig.events().as_slice(), [DownloadsEvent::Failed { error: AppError::Network, .. }]));
    }

    // ---------- Avanzamento ----------

    #[test]
    fn progress_is_read_on_the_timer_and_published_per_whole_percent() {
        let mut rig = Rig::new();
        let effects = rig.cmd(DownloadsCmd::Download(Box::new(track(1))));
        assert_eq!(timers(&effects), [(Timer::Progress, Some(PROGRESS_EVERY))]);
        let progress = rig.downloads.progress_of(id(1)).unwrap();
        let state = |rig: &Rig| rig.view().item(id(1)).unwrap().state.clone();
        assert_eq!(state(&rig), ItemState::Preparing);

        // Il server prepara ancora: niente cambia, ma il timer resta armato.
        let first = rig.view();
        assert_eq!(timers(&rig.fire(Timer::Progress)), [(Timer::Progress, Some(PROGRESS_EVERY))]);
        assert!(Arc::ptr_eq(&first, &rig.downloads.view), "nessuna pubblicazione");

        progress.set(0, Some(1000));
        rig.fire(Timer::Progress);
        assert_eq!(state(&rig), ItemState::Downloading { received: 0, total: Some(1000) });
        let at_zero = rig.view();
        // Fra due punti percentuali la vista non si muove.
        progress.set(9, Some(1000));
        rig.fire(Timer::Progress);
        assert!(Arc::ptr_eq(&at_zero, &rig.downloads.view), "0,9%: sempre lo 0%");
        progress.set(10, Some(1000));
        rig.fire(Timer::Progress);
        assert!(!Arc::ptr_eq(&at_zero, &rig.downloads.view));
        assert_eq!(rig.view().item(id(1)).unwrap().percent(), Some(1));
    }

    #[test]
    fn without_a_length_progress_moves_every_256_kib() {
        let mut rig = Rig::new();
        rig.cmd(DownloadsCmd::Download(Box::new(track(1))));
        let progress = rig.downloads.progress_of(id(1)).unwrap();
        progress.set(1000, None);
        rig.fire(Timer::Progress);
        let view = rig.view();
        progress.set(200_000, None);
        rig.fire(Timer::Progress);
        assert!(Arc::ptr_eq(&view, &rig.downloads.view));
        progress.set(300_000, None);
        rig.fire(Timer::Progress);
        assert!(!Arc::ptr_eq(&view, &rig.downloads.view));
        assert_eq!(rig.view().item(id(1)).unwrap().percent(), None);
    }

    /// Prestazioni: fermo il core non pubblica; un download pubblica solo quando qualcosa si vede
    /// (il suo inizio, un punto percentuale, la fine), e a fine corsa il timer si spegne.
    #[test]
    fn idle_publishes_nothing_progress_is_throttled_and_done_publishes_once() {
        let mut state = CoreState::default();
        let mut now = Instant::now();
        let start = state.snapshot();
        assert!(state.publish(&start).is_none(), "idle");
        let send = |state: &mut CoreState, input: Input, now: Instant| state.apply(input, now);
        let command = |cmd| Input::Command(Command::Downloads(cmd));

        // I comandi che non cambiano niente (un brano senza licenza è un evento: questo no).
        send(&mut state, command(DownloadsCmd::Cancel(id(404))), now);
        send(&mut state, command(DownloadsCmd::Retry(id(404))), now);
        send(&mut state, command(DownloadsCmd::DismissUpdate), now);
        send(&mut state, command(DownloadsCmd::SetQuality(DownloadQuality::default())), now);
        assert!(state.publish(&start).is_none(), "commands that change nothing");

        // Il brano parte: una pubblicazione, solo del dominio dei download.
        let effects = send(&mut state, command(DownloadsCmd::Download(Box::new(track(1)))), now);
        assert!(effects.iter().any(|e| matches!(e, Effect::Timer { key: TimerKey::Downloads(_), .. })));
        let started = state.publish(&start).expect("the download appears");
        assert_eq!(started.versions, DomainVersions { downloads: 1, ..DomainVersions::default() });
        assert!(state.publish(&started).is_none(), "published once");

        let progress = state.downloads.progress_of(id(1)).unwrap();
        let mut last = started;
        let mut published = 0;
        // 100 letture del timer mentre arrivano i byte da 0 a 100 %, in passi da un punto: ogni lettura
        // pubblica una volta; con passi da un decimo di punto, una lettura su dieci.
        for step in 1..=1000u64 {
            progress.set(step, Some(1000));
            now += PROGRESS_EVERY;
            let effects = send(&mut state, Input::Timer(TimerKey::Downloads(Timer::Progress)), now);
            assert!(
                effects.iter().any(|e| matches!(e, Effect::Timer { key: TimerKey::Downloads(_), .. })),
                "il timer resta armato finché si scarica"
            );
            if let Some(next) = state.publish(&last) {
                published += 1;
                assert_eq!(next.versions.downloads, last.versions.downloads + 1);
                last = next;
            }
        }
        assert_eq!(published, 101, "lo 0 % (inizio dei byte) e poi un punto alla volta, mai di più");

        // La fine: una sola pubblicazione (il brano esce dalla coda, nasce l'evento), e il timer si spegne.
        let ticket = state.downloads.ticket(id(1)).unwrap();
        let effects = send(
            &mut state,
            Input::Done(effect::Done::Downloads(Done::Track { id: id(1), ticket, result: Ok(saved(1)) })),
            now,
        );
        assert_eq!(timers(&effects), [(Timer::Progress, None)]);
        let finished = state.publish(&last).expect("the download is done");
        assert!(finished.downloads.items.is_empty());
        assert!(matches!(finished.downloads.events.back().map(|e| &e.event), Some(DownloadsEvent::Saved(_))));
        assert!(state.publish(&finished).is_none(), "done publishes once");
        // Fermo di nuovo: il timer scaduto per sbaglio non pubblica né si riarma.
        let effects = send(&mut state, Input::Timer(TimerKey::Downloads(Timer::Progress)), now);
        assert!(effects.is_empty());
        assert!(state.publish(&finished).is_none());
    }

    #[test]
    fn the_quality_is_a_setting_the_domain_hands_to_the_settings() {
        let mut state = CoreState::new(crate::config::Config::default(), None, 1);
        let command = |cmd| Input::Command(Command::Downloads(cmd));
        state.apply(command(DownloadsCmd::SetQuality(DownloadQuality::Low)), Instant::now());
        assert_eq!(state.settings.view.config.download_quality, DownloadQuality::Low);
        assert_eq!(state.downloads.view.quality, DownloadQuality::Low);
        // Il frontend che manda ancora `Store` non cambia la qualità che il dominio possiede.
        let stale =
            crate::config::CoreConfig { download_quality: DownloadQuality::High, ..Default::default() };
        state.apply(
            Input::Command(Command::Settings(crate::command::SettingsCmd::Store(Box::new(stale)))),
            Instant::now(),
        );
        assert_eq!(state.settings.view.config.download_quality, DownloadQuality::Low);
    }

    // ---------- Aggiornamenti ----------

    #[test]
    fn a_local_build_checks_nothing_by_itself_but_a_release_checks_at_start_and_every_six_hours() {
        let mut local = Rig::new();
        let mut effects = Vec::new();
        local.downloads.start(&mut effects);
        assert!(effects.is_empty(), "senza release non si controlla da soli");
        assert!(local.cmd(DownloadsCmd::CheckUpdate(UpdateCheck::Poll)).is_empty());
        assert_eq!(
            jobs(local.cmd(DownloadsCmd::CheckUpdate(UpdateCheck::Manual))).len(),
            1,
            "a richiesta sì"
        );

        let mut rig = Rig::with_build("v1");
        let mut effects = Vec::new();
        rig.downloads.start(&mut effects);
        assert_eq!(timers(&effects), [(Timer::UpdatePoll, Some(UPDATE_POLL))]);
        match jobs(effects).as_slice() {
            [Job::CheckUpdate { kind: UpdateCheck::Startup, build: Some("v1"), .. }] => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(rig.view().update.status, UpdateStatus::Checking);

        let tick = rig.fire(Timer::UpdatePoll);
        assert_eq!(timers(&tick), [(Timer::UpdatePoll, Some(UPDATE_POLL))], "si riarma");
        assert!(matches!(jobs(tick).as_slice(), [Job::CheckUpdate { kind: UpdateCheck::Poll, .. }]));
    }

    #[test]
    fn a_new_release_is_offered_announced_once_and_can_be_dismissed() {
        let mut rig = Rig::with_build("v1");
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Startup, result: Ok(Some(offer("v2"))) });
        assert_eq!(rig.events(), [DownloadsEvent::UpdateOffered { tag: "v2".into(), notify: false }]);
        assert!(rig.view().update.banner().is_some());

        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Ok(Some(offer("v2"))) });
        assert_eq!(rig.events().len(), 1, "la stessa release non si annuncia di nuovo");

        rig.cmd(DownloadsCmd::DismissUpdate);
        assert!(rig.view().update.banner().is_none());
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Ok(Some(offer("v3"))) });
        assert!(rig.view().update.banner().is_some(), "una release più nuova riporta il banner");
        assert_eq!(
            rig.events().last(),
            Some(&DownloadsEvent::UpdateOffered { tag: "v3".into(), notify: true }),
            "dal timer, una release nuova avvisa anche fuori dall'app"
        );

        // Una release già annunciata fuori dall'app non si annuncia due volte.
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Ok(None) });
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Ok(Some(offer("v3"))) });
        assert_eq!(
            rig.events().last(),
            Some(&DownloadsEvent::UpdateOffered { tag: "v3".into(), notify: false })
        );
    }

    #[test]
    fn a_manual_check_always_answers_and_a_poll_stays_quiet() {
        let mut rig = Rig::with_build("v1");
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Ok(None) });
        assert!(rig.events().is_empty());
        assert_eq!(rig.view().update.status, UpdateStatus::Current);

        rig.done(Done::UpdateChecked { kind: UpdateCheck::Manual, result: Ok(None) });
        assert_eq!(rig.events(), [DownloadsEvent::UpToDate]);

        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Err(AppError::Network) });
        assert_eq!(rig.events().len(), 1);
        assert_eq!(rig.view().update.status, UpdateStatus::Failed);
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Manual, result: Err(AppError::Network) });
        assert_eq!(rig.events().last(), Some(&DownloadsEvent::UpdateCheckFailed(AppError::Network)));
    }

    #[test]
    fn a_download_in_progress_is_not_replaced_by_a_check() {
        let mut rig = Rig::with_build("v1");
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Ok(Some(offer("v2"))) });
        let effects = rig.cmd(DownloadsCmd::DownloadUpdate);
        assert!(matches!(jobs(effects).as_slice(), [Job::DownloadUpdate { offer, .. }] if offer.tag == "v2"));
        assert_eq!(
            rig.view().update.state,
            Some(UpdateState::Downloading { tag: "v2".into(), received: 0, total: None })
        );

        rig.done(Done::UpdateChecked { kind: UpdateCheck::Poll, result: Ok(Some(offer("v3"))) });
        assert!(matches!(rig.view().update.state, Some(UpdateState::Downloading { .. })));
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Manual, result: Ok(None) });
        assert!(matches!(rig.view().update.state, Some(UpdateState::Downloading { .. })));
    }

    #[test]
    fn the_update_package_lands_or_fails() {
        let mut rig = Rig::with_build("v1");
        assert!(
            rig.cmd(DownloadsCmd::DownloadUpdate).is_empty(),
            "senza un'offerta non c'è niente da scaricare"
        );
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Startup, result: Ok(Some(offer("v2"))) });
        rig.cmd(DownloadsCmd::DownloadUpdate);
        rig.done(Done::UpdateDownloaded(Ok(PathBuf::from("/dati/updates/psytify-v2.bin"))));
        assert_eq!(
            rig.view().update.state,
            Some(UpdateState::Downloaded(PathBuf::from("/dati/updates/psytify-v2.bin")))
        );
        assert_eq!(
            rig.events().last(),
            Some(&DownloadsEvent::UpdateSaved(PathBuf::from("/dati/updates/psytify-v2.bin")))
        );
        // Il file pronto si chiude con il banner.
        rig.cmd(DownloadsCmd::DismissUpdate);
        assert!(rig.view().update.state.is_none());

        rig.done(Done::UpdateChecked { kind: UpdateCheck::Startup, result: Ok(Some(offer("v3"))) });
        rig.cmd(DownloadsCmd::DownloadUpdate);
        rig.done(Done::UpdateDownloaded(Err(AppError::Network)));
        assert!(rig.view().update.state.is_none());
        assert_eq!(rig.events().last(), Some(&DownloadsEvent::UpdateFailed(AppError::Network)));
    }

    #[test]
    fn events_are_numbered_and_bounded() {
        let mut rig = Rig::new();
        let locked = fixtures::track(1);
        for _ in 0..(EVENTS + 10) {
            rig.cmd(DownloadsCmd::Download(Box::new(locked.clone())));
        }
        let view = rig.view();
        assert_eq!(view.events.len(), EVENTS);
        assert_eq!(view.events.back().map(|e| e.seq), Some(EVENTS as u64 + 10));
        assert_eq!(view.events.front().map(|e| e.seq), Some(11));
    }

    // ---------- Brani sul dispositivo ----------

    /// Un'indice con i brani `ns`, come lo legge `config.json` all'avvio.
    fn rig_with_saved(ns: &[u128]) -> Rig {
        let entries: Vec<SavedTrack> = ns
            .iter()
            .map(|n| {
                let downloaded = saved(*n);
                SavedTrack {
                    track: track(*n),
                    path: downloaded.path,
                    format: downloaded.format,
                    quality: None,
                    size: downloaded.size,
                    sha256: downloaded.sha256,
                }
            })
            .collect();
        let fields = DownloadsFields { downloaded: entries.into(), ..DownloadsFields::default() };
        Rig {
            downloads: Downloads::new(&fields, None, Dirs::default()).with_build(None),
            now: Instant::now(),
        }
    }

    #[test]
    fn a_downloaded_track_enters_the_index_newest_first_and_is_handed_to_the_settings() {
        let mut rig = Rig::new();
        rig.cmd(DownloadsCmd::Download(Box::new(track(1))));
        rig.finish(1, Ok(Downloaded { delivered: Some(DownloadQuality::Lossless), ..saved(1) }));
        rig.cmd(DownloadsCmd::Download(Box::new(track(2))));
        rig.finish(2, Ok(saved(2)));

        let view = rig.view();
        let order: Vec<Uuid> = view.saved.iter().map(|entry| entry.track.id).collect();
        assert_eq!(order, [id(2), id(1)], "il più recente in testa");
        assert!(view.is_saved(id(1)) && view.is_offline_ready(id(2)) && !view.is_saved(id(3)));
        let first = view.saved_track(id(1)).expect("its entry");
        assert_eq!(
            (first.size, first.sha256.as_str(), first.format.as_str(), first.quality),
            (4096, "hash1", "flac", Some(DownloadQuality::Lossless))
        );
        assert_eq!(view.used_bytes, 8192);
        assert_eq!(rig.downloads.local().len(), 2);
        assert_eq!(rig.downloads.local()[&id(1)], LocalFile { path: PathBuf::from("1.flac"), size: 4096 });

        // Le impostazioni ricevono l'indice, una volta; e un core nuovo lo rilegge uguale.
        assert!(rig.downloads.take_persist());
        assert!(!rig.downloads.take_persist());
        let restarted = Downloads::new(&rig.downloads.fields(), None, Dirs::default());
        assert_eq!(restarted.view.saved, view.saved);
        assert_eq!(restarted.view.used_bytes, 8192);
        assert_eq!(restarted.local().len(), 2);
    }

    #[test]
    fn a_saved_track_is_refused_unless_its_file_was_lost_and_a_new_copy_replaces_the_old_entry() {
        let mut rig = rig_with_saved(&[1, 2]);
        assert!(jobs(rig.cmd(DownloadsCmd::Download(Box::new(track(1))))).is_empty());
        assert_eq!(rig.events(), [DownloadsEvent::Refused(Refusal::AlreadySaved)]);

        // «Scarica tutto» salta i brani già sul dispositivo, e se non resta niente lo dice.
        let batch: Arc<[Track]> = Arc::from(vec![track(1), track(3)]);
        assert_eq!(jobs(rig.cmd(DownloadsCmd::DownloadAll(batch))).len(), 1);
        assert_eq!(rig.busy(), [id(3)]);
        rig.cmd(DownloadsCmd::Cancel(id(3)));
        let only_saved: Arc<[Track]> = Arc::from(vec![track(1), track(2)]);
        assert!(jobs(rig.cmd(DownloadsCmd::DownloadAll(only_saved))).is_empty());
        assert_eq!(rig.events().last(), Some(&DownloadsEvent::Refused(Refusal::AlreadySaved)));

        // Un file perso si riscarica: la voce vecchia lascia il posto alla nuova.
        rig.downloads.files_lost(vec![(id(1), LostReason::Missing)]);
        assert_eq!(jobs(rig.cmd(DownloadsCmd::Download(Box::new(track(1))))).len(), 1);
        rig.finish(1, Ok(Downloaded { path: PathBuf::from("1 (1).flac"), ..saved(1) }));
        let view = rig.view();
        assert_eq!(view.saved.iter().filter(|entry| entry.track.id == id(1)).count(), 1);
        assert_eq!(view.saved[0].path, PathBuf::from("1 (1).flac"));
        assert!(view.lost.is_empty() && view.is_offline_ready(id(1)));
        assert_eq!(rig.downloads.local()[&id(1)].path, PathBuf::from("1 (1).flac"));
    }

    #[test]
    fn the_startup_check_marks_a_missing_file_but_never_deletes_its_entry() {
        let mut rig = rig_with_saved(&[1, 2]);
        let mut effects = Vec::new();
        rig.downloads.start(&mut effects);
        let verify = jobs(effects);
        assert!(
            matches!(verify.as_slice(), [Job::Verify { entries, deep: false }] if entries.len() == 2),
            "{verify:?}"
        );
        assert!(!rig.view().verifying, "il controllo veloce non mostra un'attesa");
        assert_eq!(rig.downloads.local().len(), 2, "finché non si sa, i file si suonano");

        let before = rig.view();
        rig.done(Done::Verified { deep: false, checked: 2, lost: Vec::new() });
        assert!(Arc::ptr_eq(&before, &rig.view()), "tutto a posto: niente cambia, niente si pubblica");

        rig.done(Done::Verified {
            deep: false,
            checked: 2,
            lost: vec![Lost { id: id(2), reason: LostReason::Missing }],
        });
        let view = rig.view();
        assert_eq!(view.saved.len(), 2, "la voce resta");
        assert_eq!(view.lost_reason(id(2)), Some(LostReason::Missing));
        assert!(view.is_saved(id(2)) && !view.is_offline_ready(id(2)) && view.is_offline_ready(id(1)));
        assert!(!rig.downloads.local().contains_key(&id(2)), "un file perso non si suona dal disco");
        assert!(rig.events().is_empty());
        assert!(!rig.downloads.take_persist(), "un file perso non si scrive: si ricontrolla a ogni avvio");

        // Senza brani sul dispositivo non c'è niente da controllare.
        let mut empty = Rig::new();
        let mut effects = Vec::new();
        empty.downloads.start(&mut effects);
        assert!(jobs(effects).is_empty());
    }

    #[test]
    fn a_deep_check_reports_once_and_frees_the_files_that_came_back() {
        let mut rig = rig_with_saved(&[1, 2]);
        let first = jobs(rig.cmd(DownloadsCmd::Verify));
        assert!(matches!(first.as_slice(), [Job::Verify { deep: true, .. }]));
        assert!(rig.view().verifying);
        assert!(jobs(rig.cmd(DownloadsCmd::Verify)).is_empty(), "uno alla volta");

        rig.done(Done::Verified {
            deep: true,
            checked: 2,
            lost: vec![Lost { id: id(1), reason: LostReason::Changed }],
        });
        assert!(!rig.view().verifying);
        assert_eq!(rig.view().lost_reason(id(1)), Some(LostReason::Changed));
        assert_eq!(rig.events(), [DownloadsEvent::Verified { checked: 2, lost: 1 }]);

        // Il file è tornato: il controllo a fondo lo libera, il veloce no.
        rig.done(Done::Verified { deep: false, checked: 2, lost: Vec::new() });
        assert_eq!(rig.view().lost_reason(id(1)), Some(LostReason::Changed));
        rig.cmd(DownloadsCmd::Verify);
        rig.done(Done::Verified { deep: true, checked: 2, lost: Vec::new() });
        assert!(rig.view().lost.is_empty());
        assert_eq!(rig.downloads.local().len(), 2);
        assert_eq!(rig.events().last(), Some(&DownloadsEvent::Verified { checked: 2, lost: 0 }));
    }

    #[test]
    fn playback_marks_a_file_lost_once_and_only_for_tracks_in_the_index() {
        let mut rig = rig_with_saved(&[1]);
        rig.downloads.files_lost(vec![(id(9), LostReason::Missing)]);
        assert!(rig.view().lost.is_empty(), "un brano che non è nell'indice non c'entra");
        rig.downloads.files_lost(vec![(id(1), LostReason::Missing)]);
        let marked = rig.view();
        rig.downloads.files_lost(vec![(id(1), LostReason::Changed)]);
        assert!(Arc::ptr_eq(&marked, &rig.view()), "già segnato: niente cambia");
        assert_eq!(marked.lost_reason(id(1)), Some(LostReason::Missing));
        assert!(rig.downloads.local().is_empty());
    }

    #[test]
    fn removing_a_track_forgets_it_at_once_and_a_job_deletes_the_file() {
        let mut rig = rig_with_saved(&[1, 2]);
        rig.downloads.files_lost(vec![(id(1), LostReason::Missing)]);
        assert!(jobs(rig.cmd(DownloadsCmd::Remove(id(404)))).is_empty());
        assert!(!rig.downloads.take_persist());

        let removal = jobs(rig.cmd(DownloadsCmd::Remove(id(1))));
        assert!(matches!(
            removal.as_slice(),
            [Job::Remove { path, size: 4096, title }] if path == &PathBuf::from("1.flac") && title == "Brano 1"
        ));
        let view = rig.view();
        assert!(!view.is_saved(id(1)) && view.lost.is_empty(), "la voce e il suo segno spariscono subito");
        assert_eq!((view.saved.len(), view.used_bytes), (1, 4096));
        assert!(!rig.downloads.local().contains_key(&id(1)));
        assert!(rig.downloads.take_persist());

        rig.done(Done::Removed { title: "Brano 1".into(), result: Ok(false) });
        rig.done(Done::Removed { title: "Brano 2".into(), result: Ok(true) });
        rig.done(Done::Removed { title: "Brano 3".into(), result: Err(AppError::Io("negato".into())) });
        assert_eq!(
            rig.events(),
            [
                DownloadsEvent::Removed { title: "Brano 1".into(), kept_file: false },
                DownloadsEvent::Removed { title: "Brano 2".into(), kept_file: true },
                DownloadsEvent::RemoveFailed {
                    title: "Brano 3".into(),
                    error: AppError::Io("negato".into())
                },
            ]
        );
    }

    /// BACKLOG 39: «Apri cartella» del pacchetto pronto apre la sua cartella con un lavoro; senza un
    /// pacchetto pronto (offerto, in download, chiuso) non fa niente, e un'apertura fallita è un evento
    /// suo, non quello della cartella dei download.
    #[test]
    fn the_downloaded_update_folder_opens_only_when_a_package_is_ready() {
        let mut rig = Rig::with_build("v1");
        assert!(rig.cmd(DownloadsCmd::RevealUpdate).is_empty(), "niente da aprire");
        rig.done(Done::UpdateChecked { kind: UpdateCheck::Startup, result: Ok(Some(offer("v2"))) });
        assert!(rig.cmd(DownloadsCmd::RevealUpdate).is_empty(), "offerto, non ancora scaricato");
        rig.cmd(DownloadsCmd::DownloadUpdate);
        assert!(jobs(rig.cmd(DownloadsCmd::RevealUpdate)).is_empty(), "in download");

        rig.done(Done::UpdateDownloaded(Ok(PathBuf::from("/dati/updates/psytify-v2.bin"))));
        let open = jobs(rig.cmd(DownloadsCmd::RevealUpdate));
        assert!(
            matches!(open.as_slice(), [Job::RevealUpdate(dir)] if dir.as_path() == std::path::Path::new("/dati/updates")),
            "{open:?}"
        );
        let before = rig.view();
        rig.done(Done::UpdateRevealed(true));
        assert!(Arc::ptr_eq(&before, &rig.view()), "aperta: niente da dire");
        rig.done(Done::UpdateRevealed(false));
        assert_eq!(rig.events().last(), Some(&DownloadsEvent::UpdateFolderNotOpened));
        assert!(matches!(rig.view().update.state, Some(UpdateState::Downloaded(_))), "il pacchetto resta");

        // Un percorso senza cartella: quella degli aggiornamenti.
        rig.downloads.view_mut().update.state =
            Some(UpdateState::Downloaded(PathBuf::from("psytify-v2.bin")));
        let updates = rig.downloads.dirs.updates.clone();
        assert!(
            matches!(jobs(rig.cmd(DownloadsCmd::RevealUpdate)).as_slice(), [Job::RevealUpdate(dir)] if *dir == updates)
        );

        rig.cmd(DownloadsCmd::DismissUpdate);
        assert!(rig.cmd(DownloadsCmd::RevealUpdate).is_empty(), "chiuso, il pacchetto non è più nel banner");
    }

    #[test]
    fn opening_the_folder_is_a_job_and_a_failure_is_an_event() {
        let mut rig = Rig::new();
        let open = jobs(rig.cmd(DownloadsCmd::OpenFolder));
        assert!(matches!(open.as_slice(), [Job::OpenFolder(dir)] if *dir == rig.view().folder));
        rig.done(Done::FolderOpened(true));
        assert!(rig.events().is_empty());
        rig.done(Done::FolderOpened(false));
        assert_eq!(rig.events(), [DownloadsEvent::FolderNotOpened]);
    }

    /// Una cartella nuova per il test, con un file di `size` byte.
    fn file_of(size: usize) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!("psytify-remove-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("brano.flac");
        std::fs::write(&path, vec![1u8; size]).unwrap();
        (dir, path)
    }

    /// `BACKLOG` 3, la parte della rimozione: si cancella solo il file che l'indice descrive.
    #[tokio::test]
    async fn remove_deletes_the_saved_file_but_never_one_that_changed() {
        let (dir, path) = file_of(100);
        assert_eq!(remove_saved(&path, 100).await, Ok(false));
        assert!(!path.exists(), "il file scaricato è stato cancellato");
        assert_eq!(remove_saved(&path, 100).await, Ok(false), "già sparito: com'era voluto");

        std::fs::write(&path, vec![2u8; 150]).unwrap();
        assert_eq!(remove_saved(&path, 100).await, Ok(true));
        assert_eq!(std::fs::read(&path).unwrap(), vec![2u8; 150], "un file cambiato fuori dall'app resta");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn the_jobs_check_remove_and_open_with_the_real_disk() {
        let (dir, path) = file_of(64);
        let entries =
            vec![(id(1), LocalFile { path: path.clone(), size: 64 }, offline::hash_file(&path).unwrap())];
        let Done::Verified { deep: true, checked: 1, lost } = Job::Verify { entries, deep: true }.run().await
        else {
            panic!("a verification")
        };
        assert!(lost.is_empty());
        let gone = Job::Remove { path: path.clone(), size: 64, title: "x".into() }.run().await;
        assert!(matches!(gone, Done::Removed { result: Ok(false), .. }) && !path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Il core intero: il brano scaricato esce dalla coda, entra nell'indice e le impostazioni lo
    /// scrivono (`config.json`); chi manda ancora `Store` non lo cancella.
    #[test]
    fn the_index_is_written_to_the_settings_and_a_stale_store_does_not_erase_it() {
        let mut state = CoreState::new(crate::config::Config::default(), None, 1);
        let command = |cmd| Input::Command(Command::Downloads(cmd));
        let now = Instant::now();
        state.apply(command(DownloadsCmd::Download(Box::new(track(1)))), now);
        let ticket = state.downloads.ticket(id(1)).unwrap();
        state.apply(
            Input::Done(effect::Done::Downloads(Done::Track { id: id(1), ticket, result: Ok(saved(1)) })),
            now,
        );
        assert_eq!(state.settings.view.config.downloaded.len(), 1);
        assert_eq!(state.settings.view.config.downloaded[0].sha256, "hash1");

        let stale = crate::config::CoreConfig::default();
        state.apply(
            Input::Command(Command::Settings(crate::command::SettingsCmd::Store(Box::new(stale)))),
            now,
        );
        assert_eq!(state.settings.view.config.downloaded.len(), 1);
        assert!(state.downloads.view.is_saved(id(1)));
    }
}
