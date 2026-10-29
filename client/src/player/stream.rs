//! Ascolto progressivo, come in un servizio di streaming: il brano parte appena arrivano i primi
//! byte, mentre il resto continua a scaricarsi in memoria. Niente finisce su disco.
//!
//! Un task tokio scrive i blocchi del download nel buffer condiviso; il decoder legge con
//! [`StreamReader`] e, se chiede byte non ancora arrivati, aspetta. Il buffer è fatto di tratti:
//! se il lettore salta lontano dal punto in cui sta scrivendo il download (un seek a un minuto
//! di un FLAC), chiede al download di ripartire da lì con una richiesta `Range`, invece di
//! aspettare che ci arrivi. Finito il tratto in corso, il download riempie i buchi rimasti, così
//! un brano riascoltato è tutto in memoria.

use std::io::{self, Read, Seek, SeekFrom};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// Byte da avere in memoria prima di aprire il decoder: bastano per le intestazioni più comuni.
/// Su archive.org 128 KB arrivano in circa 2,5 secondi, 512 KB in circa 5: si parte prima e il
/// resto arriva mentre si ascolta. Se servono più byte (copertina incorporata), il decoder aspetta.
pub const PREFETCH_BYTES: usize = 128 * 1024;
/// Testata di un brano tenuta pronta in anticipo: circa otto secondi di FLAC, il tempo che il resto
/// impiega ad arrivare quando il brano parte davvero.
pub const WARM_BYTES: usize = 768 * 1024;
/// Un lettore che chiede byte entro questa distanza davanti al download aspetta che arrivino; più
/// lontano conviene una nuova richiesta da quel punto. Su archive.org i primi byte di una richiesta
/// `Range` arrivano in circa 1,7 s, il tempo in cui il download in corso scarica circa 1,4 MB.
pub const JUMP_DISTANCE: usize = 1024 * 1024;
/// Un salto parte un po' prima del byte chiesto. Il seek in un FLAC senza seektable cerca per
/// bisezione: dopo il primo punto lontano chiede punti via via più vicini e spesso più indietro
/// (116 KB, 58 KB, 29 KB…). Senza margine ogni passo è una richiesta nuova, circa un secondo l'una
/// su archive.org; con 256 KB di margine (0,3 s di download) il primo salto li copre quasi tutti.
pub const JUMP_LEAD: usize = 256 * 1024;
/// Attesa massima di un blocco: oltre, la lettura fallisce invece di bloccare per sempre.
const READ_WAIT: Duration = Duration::from_secs(30);

/// Un tratto contiguo del file, da `start`.
#[derive(Debug)]
struct Segment {
    start: usize,
    data: Vec<u8>,
}

impl Segment {
    fn end(&self) -> usize {
        self.start + self.data.len()
    }

    fn contains(&self, position: usize) -> bool {
        (self.start..self.end()).contains(&position)
    }
}

#[derive(Debug, Default)]
struct State {
    /// Tratti ordinati e separati da buchi: due tratti che si toccano diventano uno.
    segments: Vec<Segment>,
    /// Lunghezza del file, se il server l'ha detta.
    total: Option<usize>,
    /// Il server accetta richieste `Range`: si può saltare.
    ranges: bool,
    /// Prossimo byte che il download scriverà.
    front: usize,
    /// Byte chiesto dal lettore lontano dal download: il download riparte da qui.
    wanted: Option<usize>,
    complete: bool,
    failed: bool,
    cancelled: bool,
    /// Un buffer "caldo" scarica solo la testata: il download si ferma quando ha questi byte
    /// dall'inizio e riparte con [`StreamBuffer::release`].
    hold: Option<usize>,
}

impl State {
    fn segment_at(&self, position: usize) -> Option<&Segment> {
        self.segments.iter().find(|s| s.contains(position))
    }

    /// Byte contigui dall'inizio del file.
    fn head_len(&self) -> usize {
        self.segments.first().filter(|s| s.start == 0).map_or(0, |s| s.data.len())
    }

    /// Il primo byte dopo `from` che non è ancora in memoria (`from` stesso, se manca).
    fn first_missing(&self, from: usize) -> usize {
        self.segment_at(from).map_or(from, Segment::end)
    }

    /// Tutto il file è in memoria.
    fn is_full(&self) -> bool {
        self.total.is_some_and(|total| self.head_len() >= total)
    }

    /// Scrive `chunk` da `offset`, saltando le parti già presenti e fondendo i tratti che si
    /// toccano.
    fn insert(&mut self, offset: usize, chunk: &[u8]) {
        let mut position = offset;
        let mut rest = chunk;
        while !rest.is_empty() {
            if let Some(present) = self.segment_at(position) {
                let skip = (present.end() - position).min(rest.len());
                position += skip;
                rest = &rest[skip..];
                continue;
            }
            let next = self.segments.iter().position(|s| s.start > position);
            let gap_end = next.map_or(usize::MAX, |i| self.segments[i].start);
            let take = (gap_end - position).min(rest.len());
            let before = next.unwrap_or(self.segments.len()).checked_sub(1);
            match before.filter(|&i| self.segments[i].end() == position) {
                Some(i) => self.segments[i].data.extend_from_slice(&rest[..take]),
                None => {
                    let at = next.unwrap_or(self.segments.len());
                    self.segments.insert(at, Segment { start: position, data: rest[..take].to_vec() });
                }
            }
            position += take;
            rest = &rest[take..];
            self.merge_touching();
        }
    }

    fn merge_touching(&mut self) {
        let mut i = 0;
        while i + 1 < self.segments.len() {
            if self.segments[i].end() == self.segments[i + 1].start {
                let next = self.segments.remove(i + 1);
                self.segments[i].data.extend_from_slice(&next.data);
            } else {
                i += 1;
            }
        }
    }

    /// Dove il download deve continuare dopo aver scritto fino ad `after`: il punto chiesto dal
    /// lettore se c'è, altrimenti il primo byte mancante da lì, altrimenti il primo buco del file.
    fn next_step(&mut self, after: usize) -> Next {
        if let Some(wanted) = self.wanted.take()
            && self.segment_at(wanted).is_none()
        {
            // Il margine si ferma alla fine dei byte già presenti prima del punto chiesto.
            let present = self.segments.iter().map(Segment::end).filter(|&end| end <= wanted).max();
            let start = wanted.saturating_sub(JUMP_LEAD).max(present.unwrap_or(0));
            self.front = start;
            return Next::Jump(start);
        }
        if self.is_full() {
            self.complete = true;
            return Next::Done;
        }
        let resume = self.first_missing(after);
        let Some(total) = self.total.filter(|_| self.ranges) else {
            self.front = resume;
            return Next::Continue(resume);
        };
        let target = if resume < total { resume } else { self.first_missing(0) };
        self.front = target;
        if target == after { Next::Continue(target) } else { Next::Jump(target) }
    }
}

/// Cosa fa il download dopo un blocco.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Continua a leggere la risposta in corso: il prossimo blocco va da questo byte.
    Continue(usize),
    /// Chiude la risposta e ne apre una nuova da questo byte (`Range: bytes=N-`).
    Jump(usize),
    /// Il file è tutto in memoria.
    Done,
}

#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    /// Sveglia i lettori quando arrivano byte o cambia lo stato.
    arrived: Condvar,
    /// Sveglia il download quando un lettore chiede un salto: non aspetta il blocco successivo
    /// della risposta in corso, che su un nodo lento può tardare.
    jump: tokio::sync::Notify,
}

/// Buffer di un brano in arrivo. Si clona a basso costo: tutte le copie vedono gli stessi byte.
#[derive(Clone, Default)]
pub struct StreamBuffer(Arc<Shared>);

impl std::fmt::Debug for StreamBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.lock();
        f.debug_struct("StreamBuffer")
            .field("segments", &state.segments.len())
            .field("head", &state.head_len())
            .field("total", &state.total)
            .field("complete", &state.complete)
            .finish()
    }
}

impl StreamBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Buffer già completo.
    #[cfg(test)]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let buffer = Self::new();
        buffer.set_total(bytes.len(), false);
        buffer.write_at(0, &bytes);
        buffer
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // Un panic nel task che scrive non deve rendere inutilizzabile il buffer.
        self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn update<T>(&self, change: impl FnOnce(&mut State) -> T) -> T {
        let result = change(&mut self.lock());
        self.0.arrived.notify_all();
        result
    }

    /// Lunghezza del file (dalla prima risposta) e se il server accetta richieste `Range`.
    pub fn set_total(&self, total: usize, ranges: bool) {
        self.update(|state| {
            state.total = Some(total);
            state.ranges = ranges;
        });
    }

    /// Scrive un blocco arrivato da `offset` e dice al download come continuare.
    pub fn write_at(&self, offset: usize, chunk: &[u8]) -> Next {
        self.update(|state| {
            state.insert(offset, chunk);
            state.next_step(offset + chunk.len())
        })
    }

    /// La risposta in corso è finita a `offset`: il file è completo, oppure restano buchi da
    /// riempire (dopo un salto). Senza lunghezza nota, la fine della risposta è la fine del file.
    pub fn response_ended(&self, offset: usize) -> Next {
        self.update(|state| {
            if state.total.is_none() || !state.ranges {
                state.complete = true;
                return Next::Done;
            }
            match state.next_step(offset) {
                // La risposta è finita: continuare vuol dire aprirne un'altra da lì.
                Next::Continue(at) => Next::Jump(at),
                other => other,
            }
        })
    }

    /// Si risolve quando un lettore chiede di saltare (subito, se l'ha già chiesto).
    pub async fn jump_requested(&self) {
        self.0.jump.notified().await;
    }

    pub fn fail(&self) {
        self.update(|state| state.failed = true);
    }

    /// Ferma il download: serve quando si passa a un altro brano prima della fine.
    pub fn cancel(&self) {
        self.update(|state| state.cancelled = true);
    }

    pub fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    /// Il download è caduto e il buffer non si completerà più.
    pub fn is_failed(&self) -> bool {
        self.lock().failed
    }

    /// Il server accetta richieste `Range` e ha detto la lunghezza: si può riaprire il download.
    pub fn can_range(&self) -> bool {
        let state = self.lock();
        state.ranges && state.total.is_some()
    }

    /// Chiede al download di fermarsi appena ha `bytes` byte dall'inizio, in attesa di [`Self::release`].
    pub fn hold_at(&self, bytes: usize) {
        self.update(|state| state.hold = Some(bytes));
    }

    /// Lascia continuare un download fermato da [`Self::hold_at`]. Non fa nulla se non era fermo.
    pub fn release(&self) {
        self.update(|state| state.hold = None);
    }

    /// Il download deve aspettare: ha già la testata richiesta e nessuno l'ha ancora sbloccato.
    pub fn is_held(&self) -> bool {
        let state = self.lock();
        !state.complete && state.hold.is_some_and(|hold| state.head_len() >= hold)
    }

    pub fn is_complete(&self) -> bool {
        self.lock().complete
    }

    /// Lunghezza del file, se nota (dal server o a download finito): serve al decoder per cercare
    /// nel file.
    pub fn total(&self) -> Option<usize> {
        let state = self.lock();
        state.total.or_else(|| state.complete.then(|| state.head_len()))
    }

    /// I primi `n` byte scaricati finora (meno, se sono di meno).
    pub fn head(&self, n: usize) -> Vec<u8> {
        let state = self.lock();
        state
            .segments
            .first()
            .filter(|s| s.start == 0)
            .map_or_else(Vec::new, |s| s.data[..n.min(s.data.len())].to_vec())
    }

    /// Copia dei byte contigui dall'inizio: a download completo, il file intero.
    #[cfg(test)]
    pub fn snapshot(&self) -> Vec<u8> {
        self.head(usize::MAX)
    }

    /// Aspetta senza bloccare il runtime che ci siano `bytes` byte dall'inizio, o che il download
    /// finisca. `false` se il download è fallito o è stato annullato prima.
    pub async fn ready(&self, bytes: usize) -> bool {
        loop {
            {
                let state = self.lock();
                if state.failed || state.cancelled {
                    return false;
                }
                if state.complete || state.head_len() >= bytes {
                    return true;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    pub fn reader(&self) -> StreamReader {
        StreamReader { buffer: self.clone(), position: 0 }
    }
}

/// Lettore per il decoder: legge dal buffer e aspetta i byte che mancano.
#[derive(Debug)]
pub struct StreamReader {
    buffer: StreamBuffer,
    position: usize,
}

impl StreamReader {
    /// Lunghezza del file, se nota.
    pub fn total(&self) -> Option<usize> {
        self.buffer.total()
    }
}

impl Read for StreamReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let shared = &*self.buffer.0;
        let mut state = shared.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            if let Some(segment) = state.segment_at(self.position) {
                let available = &segment.data[self.position - segment.start..];
                let n = available.len().min(out.len());
                out[..n].copy_from_slice(&available[..n]);
                self.position += n;
                return Ok(n);
            }
            if state.total.is_some_and(|total| self.position >= total) || state.complete {
                return Ok(0);
            }
            if state.failed || state.cancelled {
                return Err(io::Error::new(io::ErrorKind::BrokenPipe, "stream interrupted"));
            }
            // Chi legge oltre la testata sblocca il download: il brano sta suonando.
            if state.hold.take().is_some() {
                shared.arrived.notify_all();
            }
            // Dietro il download o troppo avanti: il download salta qui.
            let far = self.position < state.front || self.position > state.front + JUMP_DISTANCE;
            if state.ranges && far && state.wanted != Some(self.position) {
                state.wanted = Some(self.position);
                shared.jump.notify_one();
            }
            let (next, timeout) = shared
                .arrived
                .wait_timeout(state, READ_WAIT)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
            if timeout.timed_out() && state.segment_at(self.position).is_none() && !state.complete {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "stream stalled"));
            }
        }
    }
}

impl Seek for StreamReader {
    fn seek(&mut self, to: SeekFrom) -> io::Result<u64> {
        let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "invalid seek");
        let target: i128 = match to {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(delta) => self.position as i128 + i128::from(delta),
            SeekFrom::End(delta) => {
                let state = self.buffer.lock();
                let end = match state.total {
                    Some(total) => total,
                    None if state.complete => state.head_len(),
                    // Senza lunghezza la fine si conosce solo a download finito.
                    None => return Err(io::Error::new(io::ErrorKind::Unsupported, "length not known yet")),
                };
                end as i128 + i128::from(delta)
            }
        };
        self.position = usize::try_from(target).map_err(|_| invalid())?;
        u64::try_from(self.position).map_err(|_| invalid())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Buffer di un file di `total` byte con richieste `Range`, ancora vuoto.
    fn ranged(total: usize) -> StreamBuffer {
        let buffer = StreamBuffer::new();
        buffer.set_total(total, true);
        buffer
    }

    #[test]
    fn complete_buffer_reads_like_a_file() {
        let mut reader = StreamBuffer::from_bytes(b"abcdef".to_vec()).reader();
        let mut out = Vec::new();
        reader.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"abcdef");
    }

    #[test]
    fn reader_waits_for_bytes_that_arrive_later() {
        let buffer = ranged(4);
        buffer.write_at(0, b"ab");
        let writer = buffer.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            writer.write_at(2, b"cd");
        });
        let mut out = Vec::new();
        buffer.reader().read_to_end(&mut out).unwrap();
        handle.join().unwrap();
        assert_eq!(out, b"abcd");
    }

    #[test]
    fn seek_forward_reads_from_the_new_position() {
        let mut reader = StreamBuffer::from_bytes(b"0123456789".to_vec()).reader();
        reader.seek(SeekFrom::Start(7)).unwrap();
        let mut out = [0u8; 3];
        reader.read_exact(&mut out).unwrap();
        assert_eq!(&out, b"789");
    }

    #[test]
    fn seek_from_end_uses_the_announced_length() {
        let buffer = ranged(1000);
        buffer.write_at(0, b"abc");
        assert_eq!(buffer.reader().seek(SeekFrom::End(-10)).unwrap(), 990);
    }

    #[test]
    fn seek_from_end_without_a_length_waits_for_the_end() {
        let buffer = StreamBuffer::new();
        buffer.write_at(0, b"abc");
        assert_eq!(buffer.reader().seek(SeekFrom::End(0)).unwrap_err().kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn cancelled_stream_stops_the_reader() {
        let buffer = StreamBuffer::new();
        buffer.cancel();
        assert_eq!(buffer.reader().read(&mut [0u8; 4]).unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn negative_seek_is_rejected() {
        let mut reader = StreamBuffer::from_bytes(b"abc".to_vec()).reader();
        assert!(reader.seek(SeekFrom::Current(-1)).is_err());
    }

    #[test]
    fn sequential_chunks_keep_going() {
        let buffer = ranged(10);
        assert_eq!(buffer.write_at(0, b"0123"), Next::Continue(4));
        assert_eq!(buffer.write_at(4, b"456789"), Next::Done);
        assert!(buffer.is_complete());
    }

    #[test]
    fn a_far_read_asks_the_download_to_jump_there() {
        let buffer = ranged(10 * JUMP_DISTANCE);
        buffer.write_at(0, &[0u8; 1024]);
        let reader_buffer = buffer.clone();
        let target = 5 * JUMP_DISTANCE;
        let handle = std::thread::spawn(move || {
            let mut reader = reader_buffer.reader();
            reader.seek(SeekFrom::Start(target as u64)).unwrap();
            let mut out = [0u8; 2];
            reader.read_exact(&mut out).unwrap();
            out
        });
        // Il download scrive il blocco successivo e scopre che il lettore è lontano.
        let next = loop {
            std::thread::sleep(Duration::from_millis(5));
            match buffer.write_at(1024, &[]) {
                Next::Continue(_) => continue,
                other => break other,
            }
        };
        assert_eq!(next, Next::Jump(target - JUMP_LEAD));
        buffer.write_at(target - JUMP_LEAD, &vec![0u8; JUMP_LEAD]);
        buffer.write_at(target, b"xy");
        assert_eq!(&handle.join().unwrap(), b"xy");
    }

    #[test]
    fn a_jump_lead_stops_at_bytes_already_there() {
        let buffer = ranged(10 * JUMP_DISTANCE);
        buffer.write_at(0, &[0u8; 100]);
        buffer.update(|state| state.wanted = Some(200));
        assert_eq!(buffer.write_at(100, &[]), Next::Jump(100));
    }

    #[test]
    fn a_read_just_ahead_waits_instead_of_jumping() {
        let buffer = ranged(10 * JUMP_DISTANCE);
        buffer.write_at(0, &[0u8; 1024]);
        let mut reader = buffer.reader();
        reader.seek(SeekFrom::Start(2048)).unwrap();
        let writer = buffer.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            writer.write_at(1024, &[7u8; 2048])
        });
        let mut out = [0u8; 1];
        reader.read_exact(&mut out).unwrap();
        assert_eq!((out[0], handle.join().unwrap()), (7, Next::Continue(3072)));
    }

    #[test]
    fn after_a_jump_the_gap_before_is_filled() {
        let buffer = ranged(10);
        buffer.write_at(0, b"01");
        // Salto a 6: il tratto 6..10 arriva fino alla fine del file.
        assert_eq!(buffer.write_at(6, b"6789"), Next::Jump(2));
        assert_eq!(buffer.write_at(2, b"2345"), Next::Done);
        assert_eq!(buffer.snapshot(), b"0123456789");
    }

    #[test]
    fn reaching_data_already_there_skips_past_it() {
        let buffer = ranged(10);
        buffer.write_at(5, b"567");
        assert_eq!(buffer.write_at(0, b"0123456"), Next::Jump(8));
        assert_eq!(buffer.response_ended(8), Next::Jump(8));
        assert_eq!(buffer.write_at(8, b"89"), Next::Done);
        assert_eq!(buffer.snapshot(), b"0123456789");
    }

    #[test]
    fn without_ranges_the_end_of_the_response_is_the_end_of_the_file() {
        let buffer = StreamBuffer::new();
        buffer.write_at(0, b"abc");
        assert_eq!(buffer.response_ended(3), Next::Done);
        assert!(buffer.is_complete());
    }

    #[test]
    fn a_held_buffer_waits_for_the_release() {
        let buffer = ranged(100);
        buffer.hold_at(10);
        assert!(!buffer.is_held(), "nothing arrived yet");
        buffer.write_at(0, &[0u8; 10]);
        assert!(buffer.is_held());
        buffer.release();
        assert!(!buffer.is_held());
    }

    #[test]
    fn a_reader_past_the_head_releases_the_hold() {
        let buffer = ranged(100);
        buffer.hold_at(4);
        buffer.write_at(0, &[1u8; 4]);
        let writer = buffer.clone();
        let handle = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            // Il download si è sbloccato: scrive il resto.
            assert!(!writer.is_held());
            writer.write_at(4, &[2u8; 4]);
        });
        let mut out = [0u8; 8];
        buffer.reader().read_exact(&mut out).unwrap();
        handle.join().unwrap();
        assert_eq!(out, [1, 1, 1, 1, 2, 2, 2, 2]);
    }

    #[test]
    fn a_complete_buffer_is_never_held() {
        let buffer = StreamBuffer::from_bytes(vec![0; 8]);
        buffer.hold_at(4);
        buffer.write_at(0, &[]);
        assert!(!buffer.is_held());
    }

    #[tokio::test]
    async fn ready_returns_once_enough_bytes_arrived() {
        let buffer = ranged(100);
        let writer = buffer.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(30)).await;
            writer.write_at(0, &[0u8; 10]);
        });
        assert!(buffer.ready(10).await);
    }

    #[tokio::test]
    async fn ready_counts_only_bytes_from_the_start() {
        let buffer = ranged(100);
        buffer.write_at(50, &[0u8; 20]);
        assert!(tokio::time::timeout(Duration::from_millis(50), buffer.ready(10)).await.is_err());
    }

    #[tokio::test]
    async fn ready_reports_a_failed_download() {
        let buffer = StreamBuffer::new();
        buffer.fail();
        assert!(!buffer.ready(10).await);
    }
}
