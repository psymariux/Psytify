//! Progressive listening, as in a streaming service: the track starts as soon as the first
//! bytes arrive, while the rest keeps downloading into memory. Nothing ends up on disk.
//!
//! A track already downloaded to the device plays without going through memory: [`StreamBuffer::open_file`]
//! opens the file and the reader reads it on request, with positional reads (no stretch in RAM).
//!
//! A tokio task writes the download's blocks into the shared buffer; the decoder reads with
//! [`StreamReader`] and, if it asks for bytes not yet arrived, it waits. The buffer is made of stretches:
//! if the reader jumps far from where the download is writing (a seek to a minute
//! of a FLAC), it asks the download to restart from there with a `Range` request, instead of
//! waiting for it to arrive. When the current stretch ends, the download fills the remaining holes, so
//! a track listened to again is all in memory.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// Bytes to have in memory before opening the decoder: enough for the most common headers.
/// On archive.org 128 KB arrive in about 2.5 seconds, 512 KB in about 5: it starts earlier and the
/// rest arrives while listening. If more bytes are needed (embedded cover), the decoder waits.
pub const PREFETCH_BYTES: usize = 128 * 1024;
/// A track's head kept ready in advance: about eight seconds of FLAC, the time the rest
/// takes to arrive when the track really starts.
pub const WARM_BYTES: usize = 768 * 1024;
/// A reader that asks for bytes within this distance ahead of the download waits for them to arrive; further
/// away a new request from that point is better. On archive.org the first bytes of a `Range` request
/// arrive in about 1.7 s, the time in which the download in progress fetches about 1.4 MB.
pub const JUMP_DISTANCE: usize = 1024 * 1024;
/// A jump starts a little before the requested byte. The seek in a FLAC without a seektable searches by
/// bisection: after the first far point it asks for points closer and closer, often further back
/// (116 KB, 58 KB, 29 KB…). Without a margin each step is a new request, about a second each
/// on archive.org; with a 256 KB margin (0.3 s of download) the first jump covers almost all of them.
pub const JUMP_LEAD: usize = 256 * 1024;
/// Maximum wait for a block: beyond it, the read fails instead of blocking forever.
const READ_WAIT: Duration = Duration::from_secs(30);

/// A contiguous stretch of the file, from `start`.
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
    /// Stretches sorted and separated by holes: two stretches that touch become one.
    segments: Vec<Segment>,
    /// Length of the file, if the server said it.
    total: Option<usize>,
    /// The server accepts `Range` requests: one can jump.
    ranges: bool,
    /// Next byte the download will write.
    front: usize,
    /// Byte asked for by the reader far from the download: the download restarts from here.
    wanted: Option<usize>,
    complete: bool,
    failed: bool,
    cancelled: bool,
    /// A "warm" buffer downloads only the head: the download stops when it has these bytes
    /// from the start and restarts with [`StreamBuffer::release`].
    hold: Option<usize>,
}

impl State {
    fn segment_at(&self, position: usize) -> Option<&Segment> {
        self.segments.iter().find(|s| s.contains(position))
    }

    /// Contiguous bytes from the start of the file.
    fn head_len(&self) -> usize {
        self.segments.first().filter(|s| s.start == 0).map_or(0, |s| s.data.len())
    }

    /// The download must wait: it has the requested head and nobody has unblocked it yet.
    fn held(&self) -> bool {
        !self.complete && self.hold.is_some_and(|hold| self.head_len() >= hold)
    }

    /// The first byte after `from` that is not yet in memory (`from` itself, if it is missing).
    fn first_missing(&self, from: usize) -> usize {
        self.segment_at(from).map_or(from, Segment::end)
    }

    /// The whole file is in memory.
    fn is_full(&self) -> bool {
        self.total.is_some_and(|total| self.head_len() >= total)
    }

    /// Writes `chunk` from `offset`, skipping the parts already present and merging the stretches that
    /// touch.
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

    /// Where the download must continue after writing up to `after`: the point asked for by the
    /// reader if there is one, otherwise the first missing byte from there, otherwise the first hole in the file.
    fn next_step(&mut self, after: usize) -> Next {
        if let Some(wanted) = self.wanted.take()
            && self.segment_at(wanted).is_none()
        {
            // The margin stops at the end of the bytes already present before the requested point.
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

/// What the download does after a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Keeps reading the response in progress: the next block starts from this byte.
    Continue(usize),
    /// Closes the response and opens a new one from this byte (`Range: bytes=N-`).
    Jump(usize),
    /// The file is all in memory.
    Done,
}

#[derive(Default)]
struct Shared {
    /// The file to read from on request (a downloaded track): `state` is already complete and the reader
    /// does not go through the stretches. It does not change after birth.
    file: Option<File>,
    state: Mutex<State>,
    /// Wakes the readers when bytes arrive or the state changes.
    arrived: Condvar,
    /// Wakes the download when a reader asks for a jump: it does not wait for the next block
    /// of the response in progress, which can be slow on a sluggish node.
    jump: tokio::sync::Notify,
    /// Wakes whoever waits for a change without blocking the runtime ([`StreamBuffer::ready`], the download stopped
    /// on the head): no timed polling, the task sleeps until the state changes (BACKLOG 33).
    changed: tokio::sync::Notify,
}

impl Shared {
    /// The state changed: the readers (threads) and whoever waits in a task are woken.
    fn wake(&self) {
        self.arrived.notify_all();
        self.changed.notify_waiters();
    }
}

/// Buffer of an incoming track. Cloning it is cheap: all the copies see the same bytes.
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

    /// Buffer already complete.
    #[cfg(any(test, feature = "test-support"))]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        let buffer = Self::new();
        buffer.set_total(bytes.len(), false);
        buffer.write_at(0, &bytes);
        buffer
    }

    /// Opens a track already on disk: the buffer is born complete and reads from the file on request, without
    /// keeping it in memory. It fails if the file is missing or does not weigh `expected_len` bytes (it is no longer
    /// the one that was downloaded): the caller falls back to the network.
    pub fn open_file(path: &Path, expected_len: u64) -> io::Result<Self> {
        let file = File::open(path)?;
        let meta = file.metadata()?;
        if !meta.is_file() || meta.len() != expected_len {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "the file is not the one that was saved"));
        }
        let total = usize::try_from(meta.len())
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "the file is too large"))?;
        let state = State { total: Some(total), front: total, complete: true, ..State::default() };
        Ok(Self(Arc::new(Shared { file: Some(file), state: Mutex::new(state), ..Shared::default() })))
    }

    /// The buffer reads from a file on disk (a downloaded track), not from the network.
    pub fn is_file(&self) -> bool {
        self.0.file.is_some()
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        // A panic in the writing task must not make the buffer unusable.
        self.0.state.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn update<T>(&self, change: impl FnOnce(&mut State) -> T) -> T {
        let result = change(&mut self.lock());
        self.0.wake();
        result
    }

    /// Length of the file (from the first response) and whether the server accepts `Range` requests.
    pub fn set_total(&self, total: usize, ranges: bool) {
        self.update(|state| {
            state.total = Some(total);
            state.ranges = ranges;
        });
    }

    /// Writes a block arrived from `offset` and tells the download how to continue.
    pub fn write_at(&self, offset: usize, chunk: &[u8]) -> Next {
        self.update(|state| {
            state.insert(offset, chunk);
            state.next_step(offset + chunk.len())
        })
    }

    /// The response in progress ended at `offset`: the file is complete, or holes remain to be
    /// filled (after a jump). Without a known length, the end of the response is the end of the file.
    pub fn response_ended(&self, offset: usize) -> Next {
        self.update(|state| {
            if state.total.is_none() || !state.ranges {
                state.complete = true;
                return Next::Done;
            }
            match state.next_step(offset) {
                // The response has ended: continuing means opening another one from there.
                Next::Continue(at) => Next::Jump(at),
                other => other,
            }
        })
    }

    /// Resolves when a reader asks to jump (at once, if it has already asked).
    pub async fn jump_requested(&self) {
        self.0.jump.notified().await;
    }

    pub fn fail(&self) {
        self.update(|state| state.failed = true);
    }

    /// Stops the download: needed when moving to another track before the end.
    pub fn cancel(&self) {
        self.update(|state| state.cancelled = true);
    }

    pub fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    /// The download fell and the buffer will not complete any more.
    pub fn is_failed(&self) -> bool {
        self.lock().failed
    }

    /// The server accepts `Range` requests and has said the length: the download can be reopened.
    pub fn can_range(&self) -> bool {
        let state = self.lock();
        state.ranges && state.total.is_some()
    }

    /// Asks the download to stop as soon as it has `bytes` bytes from the start, waiting for [`Self::release`].
    pub fn hold_at(&self, bytes: usize) {
        self.update(|state| state.hold = Some(bytes));
    }

    /// Lets a download stopped by [`Self::hold_at`] continue. Does nothing if it was not stopped.
    pub fn release(&self) {
        self.update(|state| state.hold = None);
    }

    /// The download must wait: it already has the requested head and nobody has unblocked it yet.
    pub fn is_held(&self) -> bool {
        self.lock().held()
    }

    /// Waits until the download can restart: someone has unblocked it ([`Self::release`], or a reader
    /// that goes beyond the head), the file is complete or the download is cancelled. At once, if it is not stopped.
    pub async fn wait_released(&self) {
        self.wait_until(|state| (state.cancelled || !state.held()).then_some(())).await;
    }

    /// Waits without polling until `check` gives an outcome: it tries it now and then at each state change.
    async fn wait_until<T>(&self, mut check: impl FnMut(&State) -> Option<T>) -> T {
        loop {
            // One subscribes before looking at the state: a change between the check and the wait wakes the
            // same one (`notify_waiters` reaches only those already subscribed).
            let notified = self.0.changed.notified();
            let mut notified = std::pin::pin!(notified);
            notified.as_mut().enable();
            if let Some(outcome) = check(&self.lock()) {
                return outcome;
            }
            notified.await;
        }
    }

    pub fn is_complete(&self) -> bool {
        self.lock().complete
    }

    /// Length of the file, if known (from the server or when the download is finished): the decoder needs it to seek
    /// in the file.
    pub fn total(&self) -> Option<usize> {
        let state = self.lock();
        state.total.or_else(|| state.complete.then(|| state.head_len()))
    }

    /// The first `n` bytes downloaded so far (fewer, if there are fewer).
    pub fn head(&self, n: usize) -> Vec<u8> {
        if let Some(file) = &self.0.file {
            let total = self.lock().total.unwrap_or(0);
            let mut bytes = vec![0; n.min(total)];
            let mut got = 0;
            while got < bytes.len() {
                match read_at(file, &mut bytes[got..], got as u64) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => got += read,
                }
            }
            bytes.truncate(got);
            return bytes;
        }
        let state = self.lock();
        state
            .segments
            .first()
            .filter(|s| s.start == 0)
            .map_or_else(Vec::new, |s| s.data[..n.min(s.data.len())].to_vec())
    }

    /// Copy of the contiguous bytes from the start: when the download is complete, the whole file.
    #[cfg(test)]
    pub fn snapshot(&self) -> Vec<u8> {
        self.head(usize::MAX)
    }

    /// Waits without blocking the runtime until there are `bytes` bytes from the start, or the download
    /// ends. `false` if the download failed or was cancelled before.
    pub async fn ready(&self, bytes: usize) -> bool {
        self.wait_until(|state| {
            if state.failed || state.cancelled {
                Some(false)
            } else if state.complete || state.head_len() >= bytes {
                Some(true)
            } else {
                None
            }
        })
        .await
    }

    pub fn reader(&self) -> StreamReader {
        StreamReader { buffer: self.clone(), position: 0 }
    }
}

/// Reader for the decoder: it reads from the buffer and waits for the bytes that are missing.
#[derive(Debug)]
pub struct StreamReader {
    buffer: StreamBuffer,
    position: usize,
}

impl StreamReader {
    /// Length of the file, if known.
    pub fn total(&self) -> Option<usize> {
        self.buffer.total()
    }
}

/// Reads from `at` without moving the file's cursor: several readers do not step on each other.
#[cfg(unix)]
fn read_at(file: &File, out: &mut [u8], at: u64) -> io::Result<usize> {
    std::os::unix::fs::FileExt::read_at(file, out, at)
}

#[cfg(windows)]
fn read_at(file: &File, out: &mut [u8], at: u64) -> io::Result<usize> {
    std::os::windows::fs::FileExt::seek_read(file, out, at)
}

impl Read for StreamReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        // A track on disk: positional read, without stretches and without waits.
        if let Some(file) = &self.buffer.0.file {
            let read = read_at(file, out, self.position as u64)?;
            self.position += read;
            return Ok(read);
        }
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
            // Who reads beyond the head unblocks the download: the track is playing.
            if state.hold.take().is_some() {
                shared.wake();
            }
            // Behind the download or too far ahead: the download jumps here.
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
                    // Without a length the end is only known when the download finishes.
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

    /// Buffer of a file of `total` bytes with `Range` requests, still empty.
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
        // The download writes the next block and finds out that the reader is far.
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
        // Jump to 6: the 6..10 stretch arrives up to the end of the file.
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
            // The download has unblocked: it writes the rest.
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

    /// The waiting task does not have an outcome yet: it is run until it stays stopped.
    async fn settle() {
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
    }

    // With the time stopped (`start_paused`) a polling wait would advance the clock: these tests
    // show that the wait wakes for the state change and not for a timer (BACKLOG 33).
    #[tokio::test(start_paused = true)]
    async fn ready_wakes_on_the_bytes_without_a_timer() {
        let buffer = ranged(100);
        let start = tokio::time::Instant::now();
        let waiting = tokio::spawn({
            let buffer = buffer.clone();
            async move { buffer.ready(10).await }
        });
        settle().await;
        assert!(!waiting.is_finished());
        buffer.write_at(0, &[0u8; 6]);
        settle().await;
        assert!(!waiting.is_finished(), "six bytes are not enough");
        buffer.write_at(6, &[0u8; 4]);
        assert!(waiting.await.unwrap());
        assert_eq!(tokio::time::Instant::now(), start, "no timer ran");
    }

    #[tokio::test(start_paused = true)]
    async fn a_held_download_sleeps_until_release_or_cancel() {
        for cancel in [false, true] {
            let buffer = ranged(100);
            buffer.hold_at(4);
            buffer.write_at(0, &[0u8; 4]);
            assert!(buffer.is_held());
            let start = tokio::time::Instant::now();
            let waiting = tokio::spawn({
                let buffer = buffer.clone();
                async move { buffer.wait_released().await }
            });
            settle().await;
            assert!(!waiting.is_finished());
            if cancel {
                buffer.cancel();
            } else {
                buffer.release();
            }
            waiting.await.unwrap();
            assert_eq!(tokio::time::Instant::now(), start, "no timer ran");
        }
        // Not stopped: it returns at once.
        let free = ranged(100);
        tokio::time::timeout(Duration::from_millis(1), free.wait_released()).await.unwrap();
    }

    #[tokio::test(start_paused = true)]
    async fn a_reader_past_the_head_releases_the_download() {
        let buffer = ranged(100);
        buffer.hold_at(4);
        buffer.write_at(0, &[0u8; 4]);
        let waiting = tokio::spawn({
            let buffer = buffer.clone();
            async move { buffer.wait_released().await }
        });
        settle().await;
        assert!(!waiting.is_finished());
        // The reader goes beyond the head (on another thread: the read waits for the missing bytes).
        let reader = buffer.clone();
        let read = std::thread::spawn(move || {
            let mut out = [0u8; 8];
            let mut reader = reader.reader();
            let _ = reader.read(&mut out);
            let _ = reader.read(&mut out);
        });
        waiting.await.unwrap();
        buffer.write_at(4, &[0u8; 96]);
        read.join().unwrap();
    }

    #[tokio::test]
    async fn ready_reports_a_failed_download() {
        let buffer = StreamBuffer::new();
        buffer.fail();
        assert!(!buffer.ready(10).await);
    }

    fn scratch_file(bytes: &[u8]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("psytify-stream-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("track.bin");
        std::fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn a_file_backed_buffer_is_complete_and_reads_and_seeks_from_disk() {
        let bytes: Vec<u8> = (0..=255u8).cycle().take(5000).collect();
        let path = scratch_file(&bytes);
        let buffer = StreamBuffer::open_file(&path, 5000).unwrap();
        assert!(buffer.is_file() && buffer.is_complete() && !buffer.can_range());
        assert_eq!(buffer.total(), Some(5000));
        assert_eq!(buffer.head(4), bytes[..4]);
        assert_eq!(buffer.head(usize::MAX), bytes);

        let mut reader = buffer.reader();
        let mut out = vec![0u8; 100];
        reader.seek(SeekFrom::Start(4000)).unwrap();
        reader.read_exact(&mut out).unwrap();
        assert_eq!(out, bytes[4000..4100]);
        reader.seek(SeekFrom::End(-10)).unwrap();
        let mut tail = Vec::new();
        reader.read_to_end(&mut tail).unwrap();
        assert_eq!(tail, bytes[4990..]);
        assert_eq!(reader.read(&mut out).unwrap(), 0, "at the end of the file");
        // A second reader starts from the beginning, independent of the first.
        let mut again = buffer.reader();
        again.read_exact(&mut out).unwrap();
        assert_eq!(out, bytes[..100]);
    }

    #[test]
    fn a_file_that_is_missing_or_has_another_size_does_not_open() {
        let path = scratch_file(b"abcdef");
        assert!(StreamBuffer::open_file(&path, 6).is_ok());
        assert_eq!(StreamBuffer::open_file(&path, 7).unwrap_err().kind(), io::ErrorKind::InvalidData);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(StreamBuffer::open_file(&path, 6).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn a_file_backed_buffer_is_ready_at_once() {
        let path = scratch_file(&[7u8; 300]);
        let buffer = StreamBuffer::open_file(&path, 300).unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(50), buffer.ready(usize::MAX)).await.unwrap());
    }
}
