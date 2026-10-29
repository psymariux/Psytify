# Fixture audio del client

- `tone-moov-end.m4a`: un secondo di sinusoide 440 Hz, stereo, AAC 64 kbit/s, generato con
  `ffmpeg -f lavfi -i sine=frequency=440:duration=1:sample_rate=44100 -ac 2 -c:a aac -b:a 64k`.
  Il box `moov` è in fondo al file (il caso che fa fallire il decoder di rodio con un `unreachable!`).
- `tone-faststart.m4a`: lo stesso segnale con `-movflags +faststart` (indice in testa).
- `tone-4s.flac`: quattro secondi di sinusoide 440 Hz, mono, 22 050 Hz, generato con
  `ffmpeg -f lavfi -i sine=frequency=440:duration=4:sample_rate=22050 -ac 1 -c:a flac`. Non ha
  seektable: il seek cerca nel file per bisezione e ha bisogno della lunghezza totale.

Nessun dato personale, nessun audio di terzi.
