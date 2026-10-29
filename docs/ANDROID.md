# Psytify su Android

L'APK è lo stesso client del desktop (`client/`, usato come libreria) con l'impaginazione da
telefono (`client/src/ui/phone.rs`): barra di navigazione in basso, mini-player, player a schermo
intero con testo sincronizzato e coda. Il crate `android/` è solo il guscio: riceve l'`AndroidApp`
dalla NativeActivity, sceglie la cartella dei dati e collega la tastiera del sistema.

## Cosa serve

- Android SDK in `~/Android/Sdk` (o `$ANDROID_HOME`) con:
  - `platforms;android-35`
  - `build-tools;35.0.0`
  - `platform-tools`
  - `ndk;27.2.12479018`
- I target Rust: `rustup target add aarch64-linux-android x86_64-linux-android`.
- `cargo install cargo-apk` (0.10).
- Un JDK per `keytool` e `apksigner`.

## Compilare

```bash
scripts/android-apk.sh                          # backend: PSYTIFY_DEFAULT_API_URL di .env.build
QUICK=1 scripts/android-apk.sh                  # solo x86_64 senza LTO, per provare nell'emulatore
scripts/android-apk.sh http://10.0.2.2:8080     # backend: il PC visto dall'emulatore
scripts/android-apk.sh http://192.168.1.20:8080 # backend: il PC sulla rete locale, per un telefono vero
```

L'APK esce in `android/target/release/apk/psytify.apk`, per aarch64 (telefoni) e x86_64
(emulatore). L'indirizzo del backend entra nella configurazione al primo avvio. Per cambiarlo:

- ricompila con un altro URL e disinstalla l'app prima di reinstallarla;
- oppure cancellane i dati (Impostazioni di Android → App → Psytify → Spazio → Cancella dati).

Il backend ascolta già su `0.0.0.0:8080` (`BIND_ADDR`): dal telefono basta l'IP del PC, con il
firewall aperto sulla porta. Il traffico in chiaro (`http://`) è permesso per la rete di casa; in
produzione usa `https://`.

## La chiave di firma

Alla prima compilazione lo script crea due file:

- `~/.android/psytify-release.keystore`, la chiave;
- `~/.android/psytify-release.pass`, una password casuale con permessi 600.

Stanno fuori dal repository e non vanno mai committati. Conservali: Android installa un
aggiornamento solo se è firmato con la stessa chiave. Se li perdi, l'app va disinstallata e
reinstallata.

## Installare

- Telefono via USB, con il debug USB attivo: `adb install -r android/target/release/apk/psytify.apk`.
- Senza cavo: copia l'APK sul telefono e aprilo. Android chiede di consentire le app da fonti
  sconosciute.

## Provare nell'emulatore

```bash
avdmanager create avd -n psytify -k "system-images;android-35;google_apis;x86_64" -d pixel_7
emulator -avd psytify -no-snapshot &
adb install -r android/target/release/apk/psytify.apk
adb shell am start -n dev.psytify.app/android.app.NativeActivity
adb logcat | grep -iE "psytify|wgpu|AndroidRuntime"
```

Se l'emulatore non trova l'AVD, guarda dove l'ha scritto `avdmanager` (per esempio
`~/.config/.android/avd`) e passalo in `ANDROID_AVD_HOME`.

## Tocco e gesti

I widget di Iced 0.13 sono pensati per il mouse. La patch `src/program/touch.rs` della copia di
`iced_winit` decide il gesto prima che arrivi ai widget:

- **tocco breve**: clic;
- **trascinamento**: oltre 8 px scorre, bloccato sull'asse del gesto. La pagina scorre in verticale,
  gli scaffali in orizzontale, e c'è l'inerzia;
- **pressione lunga** (mezzo secondo): menu contestuale;
- **tocco durante l'inerzia**: la ferma senza cliccare.

Vedi `android/vendor/README.md`.

## Ogni dispositivo

- **Barre del sistema**: gli spazi si leggono da Android via JNI (`android/src/insets.rs`):
  - barra di stato e notch;
  - navigazione a gesti o a tre pulsanti.

  Finché la finestra non è attaccata si usa una stima.
- **Cambi di configurazione**: il manifest li dichiara tutti, compresi i due bit che l'SDK non
  accetta per nome (`scripts/android-manifest.py`). Tema scuro, colori dello sfondo, lingua,
  carattere, densità e finestre multiple quindi non ricreano l'activity. Ricrearla bloccherebbe una
  NativeActivity in Rust: `onDestroy` aspetta un `android_main` che non finisce.
- **Allineamento**: le librerie native sono allineate a 16 KB (`zipalign -P 16`), come chiede
  Android 15 sui telefoni con pagine da 16 KB.
- **Larghezze**: sotto i 420 px il riquadro in evidenza mette i pulsanti sotto. Le Impostazioni
  mettono ogni controllo sotto la sua spiegazione, e le loro schede scorrono.

## Verificato (emulatore Pixel 7, Android 15, 27 ottobre 2026)

- Avvio con lo splash, Home dal backend del PC, copertine e tinte.
- Scorrimento col dito con l'inerzia: pagine in verticale e scaffali in orizzontale.
- Tocco sui pulsanti e pressione lunga che apre il menu dentro l'area sicura.
- Riproduzione dallo streaming (AAC), mini-player con avanzamento, player a schermo intero con il
  testo sincronizzato e l'evidenziazione parola per parola.
- La barra in basso a quattro voci, con le Impostazioni sull'icona in alto a destra.
- Il tasto Indietro di Android: chiude il player, poi torna alla pagina precedente.
- Cerca: il campo resta in cima, la tastiera del sistema si apre entrando e si chiude uscendo. Se la
  chiudi con Indietro, tocca di nuovo «Cerca» per riaprirla.
- Coda e pagina salvate fra un avvio e l'altro.
- In background la musica continua e l'app non disegna: CPU allo 0%, circa 164 MB di RSS. Al
  ritorno la schermata si ridisegna; prima della patch restava nera (`android/vendor/README.md`).
- Nessun blocco cambiando colori del tema, tema scuro, dimensione del carattere e densità con l'app
  aperta.
- Misure provate:
  - 360 × 760 dp (telefono piccolo);
  - 412 × 915 dp;
  - 480 × 1040 dp;
  - 800 × 1280 dp (tablet);
  - navigazione a gesti e a tre pulsanti.

## Limiti noti

- **Tastiera**: la NativeActivity non dichiara un campo di testo, perciò Gboard usa la tastiera
  senza suggerimenti né correzione. Le lettere accentate potrebbero non arrivare (non verificato).
- **Dimensioni display**: se si cambiano mentre l'app è aperta, la scala resta quella vecchia
  (winit 0.30 non rilegge la densità) finché l'app non si riapre.
- **Videoclip**: non c'è (usa GStreamer, che su Android manca).
- **Controlli di sistema**: niente controlli nella tendina né sulla schermata di blocco. MPRIS esiste
  solo su Linux.
- **Download**: finiscono nella cartella privata dell'app. Non compaiono nella cartella Download del
  telefono e si perdono disinstallando.
- **Orientamento**: solo verticale. Sui tablet l'impaginazione è quella del telefono, più larga.
- **Pressione durante il tocco**: i pulsanti non mostrano lo stato «premuto» mentre il dito è giù;
  il clic parte quando il dito si stacca senza essersi mosso.
