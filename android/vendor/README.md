# Dipendenze patchate per Android

`iced_winit/` è `iced_winit` 0.13.0 da crates.io (licenza MIT, vedi il suo `Cargo.toml`) con quattro
modifiche, segnate da «Patch di Psytify». La prima: su Android `program::run` crea il ciclo degli eventi di winit
con l'`AndroidApp` ricevuto da `android_main` (`iced_winit::ANDROID_APP`). Senza, winit si ferma con
«An `AndroidApp` as passed to android_main() is required to create an `EventLoop` on Android».

La patch vale solo per il progetto `android/` (`[patch.crates-io]` nel suo `Cargo.toml`): il client
desktop usa l'`iced_winit` originale. Va rifatta se si aggiorna Iced.

Seconda modifica, in `src/conversion.rs`: su Android winit non ha `platform::modifier_supplement`;
lì si usano `logical_key` e `text` come già fa la versione per il web.

Terza modifica, in `src/program.rs`: andando in background Android distrugge la finestra nativa e al
ritorno ne dà una nuova. Iced 0.13 usa `resumed` solo per l'avvio e lasciava lo schermo nero; con la
patch `suspended` ferma i ridisegni (niente lavoro né ciclo di errori in background) e `resumed`
ricrea la superficie di ogni finestra e ridisegna. L'audio intanto continua.

Quarta modifica, `src/program/touch.rs` (agganciata in `src/program.rs`, con
`State::forget_cursor`). I widget di Iced 0.13 sono pensati per il mouse: un pulsante cattura il
dito e la lista che lo contiene non scorre, e uno `scrollable` orizzontale cattura ogni tocco (c'è un
«TODO: bubble up» in iced_widget). Qui il tocco non arriva ai widget come tale, ma diventa:

- un clic se è breve;
- una rotella in pixel, bloccata sull'asse del gesto, se il dito si muove oltre 8 px, con l'inerzia
  al rilascio;
- un clic destro se resta fermo mezzo secondo.

Dopo il gesto il puntatore viene tolto, così non resta un «hover» appeso.
