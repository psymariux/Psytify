# Stile visivo di Psytify

I token sono in `client/src/ui/theme.rs`: ogni altro file li usa e non definisce colori o misure
propri. Questo documento sostituisce la direzione «True-black flat» del 23 ottobre 2026.

## Perché è cambiato

Il 24 ottobre 2026 il proprietario del progetto ha chiesto un'interfaccia più vicina a Spotify e
Apple Music: pannelli, sfumature dalla copertina, più movimento, un menu del clic destro, icone
Iconly. Quindi **le sfumature e le ombre sono ora ammesse, ma solo in tre posti**:

1. la **tinta della copertina** in cima alla pagina e al pannello «In riproduzione»
   (`tinted_panel`, `aurora`): un gradiente verticale che si scioglie nel grigio del pannello;
2. le **ombre degli elementi che galleggiano** (menu, finestre di dialogo, avvisi, copertina
   grande, tasto Play sulla copertina);
3. la **copertina disegnata** di «Brani che ti piacciono» (sfumatura rosa).

Restano vietati: vetro e sfocature, bagliori, bordi luminosi, gradienti su pulsanti, righe o schede,
ombre sugli elementi a riposo.

I siti indicati dal proprietario (ui.unlumen.com, shadergradient.co, magicui.design,
styles.refero.design) sono stati usati come ispirazione e riportati in Rust con i gradienti
lineari di Iced: nessuna web view, nessun codice JavaScript.

## Direzione

**Tre pannelli arrotondati su nero.** Barra laterale (navigazione + libreria personale), pagina al
centro, pannello «In riproduzione» a destra, ognuno `PANEL` (`#121212`) con raggio 14, separati da
8px di nero. Sopra la barra superiore (marchio, Home e ricerca, account, tre controlli della
finestra), sotto la barra del player: entrambe direttamente sul nero.

**Colore dalla copertina.** La pagina e il pannello destro prendono la tinta media della copertina
(`Covers::tint_of`, calcolata alla decodifica); senza copertina, il colore piatto dell'album. Le
pagine senza copertina usano una tinta di casa (`BRAND_TINT`, `NEUTRAL_TINT`, `LIKED_TINT`).

**Il lime è il marchio.** `ACCENT` (`#c6f24e`) colora il Play grande, il brano in corso, le barre
dell'equalizzatore, il cuore, casuale e ripeti attivi, il segno della voce di navigazione attiva e
gli interruttori. Il pulsante primario resta bianco con testo nero (inversione).

## Token

| Ruolo | Valore | Uso |
|---|---|---|
| `GROUND` | `#000000` | Fondo della finestra, barre superiore e inferiore |
| `PANEL` | `#121212` | I tre pannelli |
| `SURFACE` | `#1b1b1b` | Card, ricerca, riga selezionata |
| `HOVER` | `#252525` | Passaggio del mouse, linee sottili |
| `RAISED` | `#2e2e2e` | Menu, finestre, binari degli slider |
| `CONTROL` | `#4d4d4d` | Bordo dei controlli |
| `TEXT` / `MUTED` / `FAINT` | `#fff` / `#b8b8b8` / `#9c9c9c` | Testo primario, secondario, meta |
| `ACCENT` | `#c6f24e` | Marchio e stati attivi |
| `NEGATIVE` | `#e5566f` | Errori, voci distruttive |

Contrasto: ogni livello di testo supera 4.5:1 (WCAG AA) su `GROUND`, `PANEL`, `SURFACE`, `HOVER` e
`RAISED`. Lo verifica il test `every_text_tier_passes_wcag_aa_on_every_surface` in `theme.rs`.

### Misure

| Token | Valore |
|---|---|
| Barra laterale | 252px |
| Barra superiore / player | 56px / 88px |
| Pannello destro | 340px |
| Margine esterno e spazio tra pannelli | 8px |
| Raggi | pannello e card 14, tessera 10, riga 8, copertina 6, pillola 999 |
| Righe di tabella / di ricerca | 56px / 64px |
| Copertina nella testa di una pagina | 188px |
| Menu contestuale | 276px, voci da 38px |
| Avviso (toast) | 420px |

### Tipografia

Inter 4.1 (OFL 1.1, `client/assets/fonts`) nei pesi 400, 500 e 600, con cifre tabellari.

| Ruolo | Corpo | Peso |
|---|---|---|
| Titolo di una pagina di brani (hero) | 44 | 600 |
| Titolo di pagina (h1) | 28 | 600 |
| Titolo di sezione | 18 | 600 |
| Corpo, celle, navigazione | 14 | 400/500 |
| Meta, etichette | 12 | 400 |

Sentence case ovunque.

## Componenti

- **Righe di brani**: nessuna zebratura. A riposo il numero; al passaggio del mouse Play, cuore,
  Scarica (se consentito) e «altre azioni»; sul brano in corso le barre dell'equalizzatore. Il clic
  destro apre il menu. Un clic sul brano in corso mette in pausa o riprende.
- **Tessere**: al passaggio compare il Play verde sulla copertina (brani); gli album si aprono.
- **Menu contestuale** (`ui/context_menu.rs`): Riproduci, Riproduci subito dopo, Aggiungi alla coda,
  Mi piace, Aggiungi a una playlist (elenco accanto, con spunta), Togli da questa playlist / dalla
  coda, Vai all'artista e all'album, Scarica. Si apre dove sta il mouse e resta nella finestra.
- **Dialoghi** (`ui/dialog.rs`): nuova playlist, rinomina, conferma di eliminazione. Invio conferma,
  Esc o un clic sul velo annullano.
- **Pannello destro** (`ui/side_panel.rs`): un selettore a pillola (binario `SURFACE`, cursore
  `RAISED` con una tacca lime) che scivola con la molla fra «In riproduzione», «Coda» e «Testo»; il
  contenuto nuovo entra salendo. «In riproduzione»: copertina che respira con il Play (o mini
  videoclip), titolo, cuore, etichette (formato, «Lossless» con contorno lime per FLAC e WAV, durata,
  scaricabile), licenza, artista, prossimo in coda. «Testo»: lo stesso motore della vista immersiva
  (`ui/lyrics.rs`), con la riga in corso in alto.
- **Impostazioni**: schede in alto (Riproduzione, Suono, Account, Informazioni) e gruppi di righe
  con nome e spiegazione a sinistra e controllo a destra; interruttori per ciò che si accende.
- **Home** (`ui/pages/home.rs`, `ui/shelf.rs`): in cima il carosello `hero` con le cinque uscite più
  recenti che hanno una copertina (sfondo `hero_card` con la tinta della copertina, cambio ogni 8 s
  con dissolvenza, puntini per scegliere). Sotto, in ordine: scorciatoie, «Continua ad ascoltare»,
  «Nuove uscite», «Novità dai tuoi artisti», «I tuoi artisti», «Mix per te», «Più ascoltati da te»,
  «Ascoltati di recente», «Album», la scheda «Il tuo ascolto» (ore, brani, artista del momento) e
  «Dalla A alla Z». Le sezioni senza dati non compaiono. Sono righe orizzontali di tessere con
  copertina da 184px (`shelf::BIG_COVER`; 148px per gli artisti) e un link «Vedi tutte» a destra
  del titolo.
- **Copertine tonde**: gli artisti hanno la copertina circolare, gli album quadrata con raggio 6.
  Iced 0.13 non arrotonda le immagini: l'angolo si ottiene mascherando l'alfa dei pixel alla
  decodifica (`Covers::handle(url, round)`, `cover()` / `cover_round()` in `ui/cover.rs`).
- **Catalogo** (`ui/pages/library.rs`): schede Album, Artisti, Tutti i brani, Scaricabili, Singoli e
  ordine (recenti, A–Z); griglie grandi di tessere, non elenchi. «Sfoglia tutto» (`browse_grid`) usa
  le tessere a colore pieno `browse_card` con i nove colori `BROWSE`.
- **Novità** (`ui/pages/news.rs`): le uscite degli artisti seguiti, dalla più recente, con la
  pastiglia `badge` (lime, testo nero) sulla voce della barra laterale finché ce ne sono di non viste.
- **Pagina artista**: banner con `aurora_full` (tinta della copertina, nessun disegno aggiunto),
  pulsante Play tondo `fab_button`, «Popolari» (cinque brani: prima i più ascoltati da te, poi i più recenti; solo con più di cinque brani) e
  «Discografia» a griglia.
- **Vista immersiva** (`ui/focus.rs`, tasto F): a tutto schermo, copertina (fino a 640px) e testo
  (fino a 760px) formano un blocco solo, centrato nella finestra; senza testo la copertina sta al
  centro, fino a 760px. Esc, F o la X in alto la chiudono.
- **Testo sincronizzato** (`ui/lyrics.rs`): non è una lista che salta, ogni verso sta dove lo porta
  la riga in corso, che scivola da un verso all'altro. La riga in corso cresce al corpo pieno e si
  accende parola per parola (stima sui caratteri, perché LRC dà solo l'inizio della riga); le altre
  stanno all'84% e sfumano verso i bordi. Le pause strumentali sono tre puntini che si accendono.
  Gli a capo si stimano con le larghezze vere di Inter SemiBold.
- **Splash d'ingresso** (`ui/splash.rs`): sul nero le tre barre del marchio saltano su una dopo
  l'altra, ondeggiano, il nome si accende lettera per lettera, poi il velo sfuma mentre la pagina
  entra. Meno di due secondi; un clic lo chiude.
- **Libreria nella barra laterale**: chip Playlist, Artisti, Album; un clic sul chip attivo li
  mostra tutti. Playlist, album salvati e artisti seguiti stanno qui, con il clic destro per
  smettere di seguire o rimuovere.
- **Timer di spegnimento e riproduzione automatica**: dal menu del player (clic destro sulla barra in
  basso). Il timer mette in pausa dopo 5, 15, 30, 45, 60 minuti o alla fine del brano; l'autoplay
  aggiunge altri brani dello stesso artista (o, se non ce ne sono, dalle uscite recenti) quando la
  coda finisce, tranne durante una jam. «Avvia radio», dal menu di un brano, fa lo stesso a richiesta.
- **Download**: il pulsante compare solo se il brano ha `downloadable: true`. Altrimenti è
  nascosto, non disabilitato: lo spazio resta.
- **Copertine**: l'immagine pubblica dell'item se c'è; altrimenti un campo di colore piatto scelto
  dall'id e l'iniziale del titolo. Nessuna immagine finta.
- **Stati**: scheletri grigi alla dimensione reale del contenuto; stato vuoto con frase e azione;
  errore dentro il blocco che è fallito, con «Riprova».
- **Icone**: Iconly (Light e Bold) per l'interfaccia, i controlli del player disegnati nello stesso
  tratto (`client/assets/icons`, crediti in `CREDITS.md`).
- **Telefono** (`ui/phone.rs`, Android e `PSYTIFY_PHONE=1`): una colonna, dentro gli spazi che
  Android dichiara per le sue barre.
  - **In alto**: «Indietro», il marchio e l'ingranaggio delle Impostazioni.
  - **Al centro**: la pagina a tutta larghezza con la tinta della copertina, senza barre di
    scorrimento: si scorre col dito. In Cerca il campo resta fermo sopra i risultati e si apre la
    tastiera.
  - **In basso**: il mini-player (copertina, titolo, cuore, Play tondo, una linea d'avanzamento; un
    tocco apre il player) e la barra con quattro voci, icona piena e tacca lime sulla voce attiva.
  - **Player a schermo intero**: copertina che respira, titolo, avanzamento, controlli, poi «Testo»
    o «Coda».
  - **Misure**: bersagli di tocco da 44 px (`TOUCH_TARGET`). Sotto 520 px la card «Continua ad
    ascoltare» usa il Play tondo senza scritta; sotto 420 px il riquadro in evidenza mette i
    pulsanti sotto la copertina.
  - **Impostazioni**: ogni controllo va sotto la sua spiegazione.
  - **Scorciatoie della Home**: titoli su due righe.
  - **Gesti**: la pressione lunga apre il menu contestuale. Il tasto Indietro di Android chiude ciò
    che sta sopra, altrimenti torna alla pagina precedente.
- **Finestra senza cornice**: la barra superiore trascina (doppio clic: ingrandisce); riduci a icona,
  schermo intero (anche F11) e chiudi stanno a destra. Il ridimensionamento ha maniglie sui bordi
  destro e basso e sull'angolo: su Wayland non si può spostare l'origine della finestra.

## Movimento

Iced 0.13 non ha animazioni: `ui/motion.rs` le costruisce con un `Tween` o con funzioni del tempo
(`appear`, `stagger`, `pop`, `wave`, `drift`, `spring_in`), e l'app chiede i frame della finestra
solo mentre qualcosa si muove.

Due andamenti: **ease-out** per pagine, liste, menu e avvisi; **molla** (`motion::spring`, supera
l'arrivo del 6% e si posa) solo per ciò che si tocca o rappresenta il brano: copertina, cursore delle
schede, marchio dello splash.

| Movimento | Durata | Token / funzione |
|---|---|---|
| Pagina nuova: sale di 16px | 220 ms | `MOTION_BASE`, `PAGE_RISE` |
| Righe di una lista: il testo compare una dopo l'altra (prime 14) | 28 ms l'una | `STAGGER`, `motion::stagger` |
| Pannello destro: si apre da destra | 260 ms | `MOTION_PANEL` |
| Menu e tasto Play sulla copertina | 120 ms | `MOTION_FAST` |
| Finestra di dialogo: sale e si schiarisce | 260 ms | `MOTION_PANEL` |
| Cuore appena premuto: rimbalzo | 260 ms | `motion::pop` |
| Barre dell'equalizzatore sul brano in corso | continuo | `motion::wave` |
| Sfumatura del pannello «In riproduzione» | respira in 14 s | `motion::drift` |
| Copertina appena arrivata: dissolvenza | 220 ms | `MOTION_BASE` |
| Scheletri | ciclo di 1,4 s | `PULSE_PERIOD` |
| Avviso: entra salendo e schiarendosi | 220 ms | `MOTION_BASE` |
| Carosello della Home: cambio di uscita | ogni 8 s, dissolvenza 420 ms | `HERO_PERIOD`, `HERO_SWAP` |
| Copertina grande: in pausa scende al 90%, al Play torna con la molla | 620 ms | `MOTION_SPRING`, `COVER_REST` |
| Cursore delle schede del pannello (molla), contenuto che sale di 10px | 620 / 260 ms | `MOTION_SPRING`, `MOTION_PANEL` |
| Testo: scorre al verso nuovo, che cresce e si schiarisce | 480 ms a 24 fps | `MOTION_LYRICS`, `AMBIENT_FRAME` |
| Testo: parole accese una a una | col Tick (4 al secondo) | `lyrics::sung_fraction` |
| Tasto Play: l'icona rimbalza al cambio Play/Pausa | 260 ms | `motion::pop` |
| Splash: barre a molla scaglionate di 90 ms, lettere di 45 ms, velo che sfuma | 1,75 s in tutto | `SPLASH_TOTAL`, `SPLASH_EXIT` |

## Limiti di Iced 0.13 e soluzioni

| Limite | Soluzione |
|---|---|
| `Wrapping::None` è ignorato | `responsive` fornisce la larghezza reale; i testi si tagliano con «…» su una stima (`components::line`) |
| Nessun nome accessibile sui widget | I pulsanti a icona hanno un tooltip con il loro nome |
| Nessun `drag_resize`, nessun doppio clic su `mouse_area` | Ridimensionamento a maniglie con eventi del mouse; doppio clic sulla barra contato a mano (`app/chrome.rs`) |
| La posizione del mouse non arriva a chi sta in uno `scrollable` | Un listener globale la ricorda (`ui/cursor.rs`) e il menu si apre lì |
| Opacità solo sui colori, non sui widget | Si sfumano i colori del testo e degli sfondi (`motion::fade`) |
| Le immagini non hanno raggio di curvatura | Maschera sull'alfa dei pixel alla decodifica (copertine tonde e arrotondate) |
| `container.clip(true)` taglia solo in rettangolo | Le forme tonde si ottengono con la maschera, non col clip |
| La finestra non può essere più stretta di 1100px | Sotto 820px di contenuto, la ricerca impila le colonne |
