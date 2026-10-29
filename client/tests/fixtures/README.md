# Client audio fixtures

- `tone-moov-end.m4a`: one second of a 440 Hz sine, stereo, AAC 64 kbit/s, generated with
  `ffmpeg -f lavfi -i sine=frequency=440:duration=1:sample_rate=44100 -ac 2 -c:a aac -b:a 64k`.
  The `moov` box is at the end of the file (the case that makes rodio's decoder fail with an `unreachable!`).
- `tone-faststart.m4a`: the same signal with `-movflags +faststart` (index at the front).
- `tone-4s.flac`: four seconds of a 440 Hz sine, mono, 22,050 Hz, generated with
  `ffmpeg -f lavfi -i sine=frequency=440:duration=4:sample_rate=22050 -ac 1 -c:a flac`. It has no
  seektable: seeking searches the file by bisection and needs the total length.

No personal data, no third-party audio.
