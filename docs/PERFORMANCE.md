# Prestazioni del client

Misure, colli di bottiglia e stato dell'ottimizzazione del client Psytify. Le regole per scrivere
codice veloce stanno in `CONTRIBUTING.md`, sezione «Performance rules».

## Come si misura

- Strumento: `scripts/perf-sample.py PID [SECONDI]`. Legge `/proc` (CPU per thread, RSS) e
  `nvidia-smi` (memoria di calcolo). Non richiede privilegi.
- **La finestra deve essere visibile e a fuoco sul workspace attivo.** Con lo schermo bloccato o la
  finestra coperta il compositore non manda i frame callback e la CPU della UI risulta quasi zero:
  una misura così è falsa (è successo, vedi sotto).
- Scenario di riferimento: istanza di prova con configurazione isolata (`XDG_CONFIG_HOME`), volume a
  zero, brano m4a con testo sincronizzato, pannello «Testo» aperto (tasto `T`), video chiuso. Si
  avvia e si fa partire via MPRIS (`org.mpris.MediaPlayer2.psytify.instance<PID>`).
- Confronti A/B: due binari, due round ciascuno, alternati. La RAM varia di ±20 MB fra due avvii
  uguali: contano le differenze ripetute, non il singolo valore.
- Profiler a campionamento (`perf`): non installato. Per la memoria si usa `gdb` con il programma
  avviato da gdb (`ptrace_scope=1`): `call (void)malloc_stats()` e `call (int)malloc_trim(0)`.

## Risultati (riproduzione m4a, lyrics aperte, video chiuso)

| | Prima | Dopo |
|---|---|---|
| CPU totale | 10,6% di un core | **1,6–1,8%** |
| Thread UI | 9,0% | **0,7%** |
| RSS dopo 15 s | 337–388 MB | **230–237 MB** |
| Librerie NVIDIA e Mesa/LLVM caricate | sì (~70 MB) | no |
| Contesto NVIDIA del processo | sì (grafico, 2 MiB) | nessuno |

Server: 21 MB di RSS, nessun intervento necessario.

### Cosa è cambiato

1. **Niente frame continui** (`app.rs`, `subscription`). `window::frames()` mandava un messaggio a ogni
   vsync mentre suona, e ogni messaggio ridisegna la finestra. Ora i frame pieni partono solo con
   un'animazione (`is_animating`); la barra di avanzamento si aggiorna col Tick (4 al secondo, passo
   sotto il pixel). Strumentando `view` e `update`: costano meno di 1 µs e 2 µs. I ~1,4 ms per
   ridisegno sono tutti nel renderer di iced, quindi l'unica leva è il numero di ridisegni.
2. **Ambiente leggero** (`launch.rs`, solo Linux). Il client si rilancia una volta con `exec` (stesso
   PID) impostando:
   - `MALLOC_ARENA_MAX=2`: glibc apriva 32 arene (una per thread) e teneva 186 MB di heap per 111 MB in
     uso; `malloc_trim` da solo riportava la RAM da 270 a 226 MB.
   - `VK_LOADER_DRIVERS_DISABLE=*nvidia*` e `WGPU_BACKEND=vulkan`, solo se c'è anche il driver di una GPU
     integrata (RADV, ANV, AMDVLK): iced disegna già sulla GPU a basso consumo, e il driver NVIDIA e
     l'OpenGL di wgpu si caricavano per niente (~70 MB e la scheda dedicata sveglia).
   - Per scegliere la GPU a mano: `PSYTIFY_ALL_GPUS=1`, oppure una delle variabili `VK_*`, `WGPU_*`,
     `DRI_PRIME`, `__NV_PRIME_RENDER_OFFLOAD`: in quel caso non si tocca nulla.
3. L'opzione «Videoclip sulla scheda video» (Impostazioni → Riproduzione) funziona con l'ambiente
   leggero: NVDEC si carica solo quando è attiva (126 MiB di VRAM), altrimenti VA-API.

### 27 ottobre 2026: nuove animazioni, cache, profilo di release

Binari di release, finestra visibile sul portatile (60 Hz), volume a zero, «In the End» (m4a,
testo sincronizzato), due giri ciascuno.

| Scenario | CPU totale | Thread UI | RSS |
|---|---|---|---|
| Cerca + «In riproduzione», profilo di prima (A) | 1,6% / 1,6% | 0,4–0,5% | 200 / 194 MB |
| Stesso, LTO completo + 1 unità di codegen (B) | 1,5% / 1,4% | 0,4% | 199 / 194 MB |
| Home + «Testo» nel pannello, scorrimento a vsync (650 ms) | 6,9% / 7,2% | 5,8% | 192 / 201 MB |
| Stesso, scorrimento a 30 fps, 480 ms | 4,2% / 4,1% | 2,8–3,0% | 193 / 196 MB |
| Stesso, 24 fps e corpo dei versi al pixel (**adottato**) | **3,0% / 2,6%** | **1,6–1,8%** | **190 / 192 MB** |

- **Profilo di release** (`Cargo.toml`): `lto = "fat"`, `codegen-units = 1`. Binario da 41 a 28 MB, CPU
  uguale o appena più bassa (dentro il rumore), RAM uguale. Compilazione da zero 2 min 51 s; a ogni
  modifica del client il link con LTO costa ~2 min 15 s (in sviluppo si usa il profilo `dev`).
  `panic` resta `unwind`.
- **Il testo che scorre** è il movimento più frequente (a ogni verso, ogni ~2 s): a vsync costava il
  5,8% del thread UI. Ora non chiede i frame della finestra ma un timer a 24 fps
  (`AMBIENT_FRAME`), dura 480 ms, e il corpo dei versi che crescono è arrotondato al pixel, così il
  testo si reimpagina solo quando la misura cambia davvero. Gli altri movimenti restano a vsync.
- **Cache delle copertine** (`ui/cover.rs`): al massimo 240 copertine pronte, scarto della meno
  disegnata di recente (LRU). La versione tonda (artisti) si ricava dal quadrato solo quando serve:
  256 KB in meno per ogni copertina di album. `is_fading` non scorre più tutta la cache a ogni
  messaggio.
- Per misurare il pannello «Testo» senza mouse né tastiera c'è il passo `hold-lyrics` del giro
  automatico (`--features tour`, `PSYTIFY_TOUR_STEPS=playing,hold-lyrics`): apre il testo e resta
  fermo 40 s.

### 28 ottobre 2026: primo ascolto di un brano del catalogo ufficiale

Tempo di `POST /v1/meta/tracks/{id}/resolve` (server di sviluppo, misurato con `curl`):

| Scenario | Tempo |
|---|---|
| Primo ascolto via YouTube, ricerca con yt-dlp (10 brani) | 3,8–4,9 s, media ~4,3 s |
| Stesso, ricerca diretta e fonti insieme (10 brani nuovi) | **3,2–4,2 s, media ~3,7 s** |
| Primo ascolto via Internet Archive in lossless | 1,4–1,5 s → **1,2 s** |
| Secondo ascolto dello stesso brano | **2–4 ms** |
| Due richieste insieme per lo stesso brano | una sola importazione: la seconda aspetta la prima |

Dove va il tempo di un brano da YouTube (yt-dlp 2026.08.19, `-v` con l'ora di ogni riga):

| Fase | Tempo |
|---|---|
| Avvio di yt-dlp (Python) | ~0,25 s per processo |
| Ricerca `ytsearch20`: configurazione del client + API | ~1,3 s (diretta: **~0,7 s**) |
| Estrazione: pagina del video (1,2 MB, lenta anche con `curl`) + API del player | ~1,4 s |
| Download dell'audio m4a (5 MB, un solo file) | ~0,8 s |
| Correzione del contenitore m4a | ~0,1 s |

Opzioni provate e scartate: `player_skip=webpage` e `player_skip=webpage,configs` fanno scattare
il controllo anti-bot di YouTube («Sign in to confirm you're not a bot»); `player_client=default,-tv`
non cambia il tempo; i client `android_vr`, `ios`, `tv_simply` e `web_safari` non danno il formato
audio; `-N 8` (frammenti in parallelo) non serve, l'audio è un file unico. Cosa è cambiato:

- **Ricerca di YouTube diretta** (`ingest/sources.rs`, `youtube_direct`): la stessa chiamata della
  pagina dei risultati (`youtubei/v1/search`), senza processo. Se non risponde entro 4 s, cambia
  forma o non dà video, si usa `ytsearch` come prima. Vale anche per la ricerca dell'app.
- **Le tre fonti insieme** (`find_song_anywhere`): YouTube, SoundCloud e Bandcamp partono insieme;
  vince sempre la prima nell'ordine che ha il brano, le altre si fermano. Se YouTube non lo ha, la
  risposta di SoundCloud è già pronta.
- **Il download parte mentre Archive cerca** (`ingest/resolve.rs`): l'audio dall'altra fonte si
  scarica in parallelo alla ricerca del lossless (`prepare_source`) e si registra solo se Archive non
  lo trova; altrimenti il download si ferma (yt-dlp terminato, cartella cancellata).
- **Il play premuto non fa la fila** (`app/external.rs`): «Riproduci» importa subito il primo
  brano, anche se un'importazione in sottofondo (una playlist salvata) è in corso. Prima aspettava
  che finisse: fino a ~4–5 s in più.
- **Un brano alla volta per id** (`ingest/resolve.rs`, `Resolving`): due richieste per lo stesso
  brano non lanciano due download; chi aspetta trova il brano nel catalogo.
- **Archive**: i metadati dei 3 candidati si scaricano insieme invece che in fila, e il file scelto
  resta legato all'id di Deezer, così il secondo ascolto è immediato anche se su Archive il titolo è
  scritto diversamente.
- **Scartato: preparare il brano al passaggio del mouse.** Risolvere e firmare hanno un limite di 120
  richieste al minuto per IP: le richieste del passaggio del mouse toglierebbero spazio al clic vero.
- **Resta da provare: suonare mentre scarica.** Oggi il client parte quando il file è sul disco del
  server (~0,9 s di download e correzione). Il m4a di YouTube è frammentato e yt-dlp lo riscrive alla
  fine: servirlo a metà richiede di saltare la correzione e un decoder che legga i frammenti.

### Tentativi scartati

- **mimalloc** come allocatore globale: RSS 522–540 MB contro 344 (tiene le pagine, e le librerie C
  continuano a usare glibc). Rimosso.
- **Barra a 20 o 10 Hz con `time::every`**: 4,5% e 3,6% di CPU totale. Il Tick basta.

## Da fare

- **FLAC**: nel catalogo locale tutti i 52 brani sono m4a con licenza esterna; il FLAC di Internet
  Archive arriva dopo un import (scrive nel database, serve l'autorizzazione dell'utente). Va misurata la
  decodifica FLAC (Symphonia) e il buffer di `player/stream.rs`.
- **Video aperto**: costo di GStreamer e dei decoder, con e senza opzione dedicata.
- **Heap principale ~50 MB in uso** (stima: font di sistema di cosmic-text, immagini, testi). Serve un
  profilo dell'heap per dire altro; `perf` o `heaptrack` non sono installati.
- **Lyrics**: `verse_heights` ricalcola le altezze a ogni ridisegno (una somma di larghezze per verso,
  pochi microsecondi). Da tenere in cache per brano e larghezza se un profilo lo indica.

## Ricerca: cosa resta da provare (27 ottobre 2026)

Ordinate per guadagno atteso sul nostro caso, che è dominato dal numero di ridisegni (il renderer
costa ~1,4–2,6 ms a frame, `view` e `update` pochi microsecondi).

1. **Meno ridisegni, non ridisegni più veloci.** È la leva che ha funzionato ogni volta (frame solo
   con un'animazione, Tick a 250 ms, testo a 24 fps). Prossimo passo: fermare il Tick mentre è
   attivo il timer del testo (oggi arrivano entrambi) e non ridisegnare quando la posizione mostrata
   non cambia di almeno un pixel.
2. **Iced 0.14**: le note di rilascio citano il *reactive rendering* (#2662: si ridisegna solo ciò
   che cambia) e il *culling* delle primitive in `column` e `row` (#2611). Sono i due problemi che qui
   aggiriamo a mano; aggiornare vale una misura, ma l'API di `Task`, `Subscription` e stili cambia e
   la migrazione tocca tutta la UI.
3. **`lazy` di Iced** sulle parti che non cambiano durante la riproduzione (barra laterale, scaffali
   della Home): il widget si ricostruisce solo se cambia la chiave. Utile se un profilo mostra `view`
   pesante; oggi non lo è.
4. **PGO** (ottimizzazione guidata dal profilo, `cargo-pgo`): il Rust Performance Book riporta
   guadagni del 10% e più, ma serve un carico rappresentativo e due compilazioni; il nostro collo di
   bottiglia è la GPU/il renderer, non il codice Rust, quindi l'attesa è bassa.
5. **`target-cpu=native`**: solo per build personali, il binario non girerebbe su CPU più vecchie.
6. **Allocatori alternativi** (jemalloc, mimalloc): il Perf Book li consiglia, ma qui mimalloc ha
   peggiorato la RAM (vedi sotto). jemalloc va misurato allo stesso modo prima di adottarlo.
7. **Modalità di presentazione** (`ICED_PRESENT_MODE`, `fifo` di default): `fifo` è il vsync e la scelta
   giusta per un player; `mailbox`/`immediate` aumentano i frame, non li riducono.

Fonti: [Rust Performance Book, Build configuration](https://nnethercote.github.io/perf-book/build-configuration.html),
[iced, note di rilascio](https://github.com/iced-rs/iced/releases),
[iced, guida alle prestazioni (`lazy`)](https://www.mintlify.com/iced-rs/iced/advanced/performance),
[iced_wgpu, `present_mode_from_env`](https://docs.iced.rs/iced_wgpu/settings/fn.present_mode_from_env.html),
[wgpu, `PresentMode`](https://wgpu.rs/doc/wgpu/enum.PresentMode.html).

## Obiettivi

- Fermo con lyrics: CPU ≤ 0,5% (oggi 0,6–0,7%).
- In riproduzione con lyrics: CPU ≤ 3% (oggi 2,6–3,0% con il testo animato; era 1,6–1,8% senza
  animazione).
- RSS in riproduzione ≤ 300 MB (oggi 190–200 MB).
