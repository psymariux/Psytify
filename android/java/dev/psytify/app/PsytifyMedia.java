package dev.psytify.app;

import android.app.Activity;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.content.pm.PackageInstaller;
import android.graphics.Bitmap;
import android.graphics.drawable.Icon;
import android.media.MediaMetadata;
import android.media.session.MediaSession;
import android.media.session.PlaybackState;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.os.SystemClock;
import android.util.Log;

import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

/**
 * La sessione multimediale di Psytify: il brano sulla schermata di blocco, nella notifica e nei
 * controlli rapidi, e i comandi che arrivano da lì e dalle cuffie.
 *
 * È l'unica classe Java dell'app: `MediaSession.Callback` è una classe astratta, e da JNI non si
 * può estendere. La logica resta in Rust (`android/src/media.rs`): qui si mostra ciò che Rust passa
 * e si rimanda a Rust ogni comando con `command`, registrato con `RegisterNatives`. Tutto gira sul
 * thread principale di Android, che ha il suo Looper.
 */
public final class PsytifyMedia extends MediaSession.Callback {
    private static final String CHANNEL = "playback";
    private static final int NOTIFICATION = 1;
    private static final String ACTION = "dev.psytify.app.MEDIA";
    private static final String INSTALL_ACTION = "dev.psytify.app.INSTALL";
    private static final String TAG = "Psytify";
    private static final String CODE = "code";
    /** Gli stessi codici di `phone_command` nel client. */
    private static final int PLAY = 0, PAUSE = 1, NEXT = 2, PREVIOUS = 3, SEEK = 4, TOGGLE = 5;

    private static final Handler MAIN = new Handler(Looper.getMainLooper());
    private static PsytifyMedia instance;

    private final Activity activity;
    private final MediaSession session;
    private final NotificationManager notifications;
    private final PendingIntent open;
    /** La copertina del brano in corso, da riusare quando cambia solo la posizione. */
    private Bitmap art;

    /** Un comando per il player: codice e, per il salto, la posizione in millisecondi. */
    static native void command(int code, long value);

    /** Crea la sessione, una volta sola, e chiede il permesso delle notifiche (Android 13 e oltre). */
    public static void start(final Activity activity) {
        MAIN.post(() -> {
            if (instance == null) {
                instance = new PsytifyMedia(activity);
                if (Build.VERSION.SDK_INT >= 33
                        && activity.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS)
                                != android.content.pm.PackageManager.PERMISSION_GRANTED) {
                    activity.requestPermissions(new String[] {android.Manifest.permission.POST_NOTIFICATIONS}, 1);
                }
            }
        });
    }

    /**
     * Mostra il brano. `artChanged` dice se la copertina è cambiata: allora `argb` (lato `side`) è la
     * nuova, o `null` se il brano non ne ha. Altrimenti si tiene quella di prima.
     */
    public static void show(final String title, final String artist, final String album, final long durationMs,
            final long positionMs, final boolean playing, final boolean artChanged, final int side,
            final int[] argb) {
        MAIN.post(() -> {
            if (instance != null) {
                instance.update(title, artist, album, durationMs, positionMs, playing, artChanged, side, argb);
            }
        });
    }

    /** Niente in riproduzione: via la notifica, la sessione resta pronta. */
    public static void hide() {
        MAIN.post(() -> {
            if (instance != null) {
                instance.clear();
            }
        });
    }

    private PsytifyMedia(Activity activity) {
        this.activity = activity;
        session = new MediaSession(activity, "Psytify");
        session.setCallback(this, MAIN);
        Intent launch = activity.getPackageManager().getLaunchIntentForPackage(activity.getPackageName());
        open = launch == null ? null
                : PendingIntent.getActivity(activity, 0, launch,
                        PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        if (open != null) {
            session.setSessionActivity(open);
        }
        notifications = activity.getSystemService(NotificationManager.class);
        NotificationChannel channel =
                new NotificationChannel(CHANNEL, "Riproduzione", NotificationManager.IMPORTANCE_LOW);
        channel.setShowBadge(false);
        notifications.createNotificationChannel(channel);
        // Prima di Android 13 i pulsanti della notifica sono azioni sue: tornano qui.
        BroadcastReceiver receiver = new BroadcastReceiver() {
            @Override
            public void onReceive(Context context, Intent intent) {
                command(intent.getIntExtra(CODE, -1), 0);
            }
        };
        IntentFilter filter = new IntentFilter(ACTION);
        if (Build.VERSION.SDK_INT >= 33) {
            activity.registerReceiver(receiver, filter, Context.RECEIVER_NOT_EXPORTED);
        } else {
            activity.registerReceiver(receiver, filter);
        }
        // Il risultato dell'installazione: se Android chiede la conferma, la si mostra qui.
        BroadcastReceiver installResult = new BroadcastReceiver() {
            @Override
            public void onReceive(Context context, Intent intent) {
                int status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
                if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
                    Intent confirm = intent.getParcelableExtra(Intent.EXTRA_INTENT);
                    if (confirm != null) {
                        confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
                        context.startActivity(confirm);
                    }
                } else if (status != PackageInstaller.STATUS_SUCCESS) {
                    Log.w(TAG, "Aggiornamento non installato: " + intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE));
                }
            }
        };
        IntentFilter installFilter = new IntentFilter(INSTALL_ACTION);
        if (Build.VERSION.SDK_INT >= 33) {
            activity.registerReceiver(installResult, installFilter, Context.RECEIVER_NOT_EXPORTED);
        } else {
            activity.registerReceiver(installResult, installFilter);
        }
    }

    /** Passa l'APK scaricato a `PackageInstaller`: Android chiede la conferma all'utente. */
    public static void install(final String path) {
        MAIN.post(() -> {
            if (instance == null) {
                return;
            }
            try {
                instance.installPackage(path);
            } catch (IOException e) {
                Log.w(TAG, "Installazione non avviata", e);
            }
        });
    }

    private void installPackage(String path) throws IOException {
        PackageInstaller installer = activity.getPackageManager().getPackageInstaller();
        int id = installer.createSession(
                new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL));
        try (PackageInstaller.Session session = installer.openSession(id)) {
            try (InputStream in = new FileInputStream(path); OutputStream out = session.openWrite("psytify", 0, -1)) {
                byte[] buffer = new byte[64 * 1024];
                for (int read; (read = in.read(buffer)) != -1; ) {
                    out.write(buffer, 0, read);
                }
                session.fsync(out);
            }
            // Il sistema riempie il risultato: serve un PendingIntent mutabile da Android 12.
            Intent result = new Intent(INSTALL_ACTION).setPackage(activity.getPackageName());
            int flags = PendingIntent.FLAG_UPDATE_CURRENT
                    | (Build.VERSION.SDK_INT >= 31 ? PendingIntent.FLAG_MUTABLE : 0);
            PendingIntent pending = PendingIntent.getBroadcast(activity, id, result, flags);
            session.commit(pending.getIntentSender());
        } catch (IOException | RuntimeException e) {
            installer.abandonSession(id);
            throw e instanceof IOException ? (IOException) e : new IOException(e);
        }
    }

    private void update(String title, String artist, String album, long durationMs, long positionMs,
            boolean playing, boolean artChanged, int side, int[] argb) {
        if (artChanged) {
            art = argb != null && side > 0 && argb.length == side * side
                    ? Bitmap.createBitmap(argb, side, side, Bitmap.Config.ARGB_8888)
                    : null;
        }
        MediaMetadata.Builder meta = new MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, title)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, artist)
                .putLong(MediaMetadata.METADATA_KEY_DURATION, durationMs);
        if (album != null) {
            meta.putString(MediaMetadata.METADATA_KEY_ALBUM, album);
        }
        if (art != null) {
            meta.putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, art);
        }
        session.setMetadata(meta.build());
        long actions = PlaybackState.ACTION_PLAY | PlaybackState.ACTION_PAUSE | PlaybackState.ACTION_PLAY_PAUSE
                | PlaybackState.ACTION_SKIP_TO_NEXT | PlaybackState.ACTION_SKIP_TO_PREVIOUS
                | PlaybackState.ACTION_SEEK_TO | PlaybackState.ACTION_STOP;
        session.setPlaybackState(new PlaybackState.Builder()
                .setActions(actions)
                .setState(playing ? PlaybackState.STATE_PLAYING : PlaybackState.STATE_PAUSED, positionMs,
                        playing ? 1f : 0f, SystemClock.elapsedRealtime())
                .build());
        session.setActive(true);

        Notification.Builder notification = new Notification.Builder(activity, CHANNEL)
                .setSmallIcon(smallIcon())
                .setContentTitle(title)
                .setContentText(artist)
                .setLargeIcon(art)
                .setOngoing(playing)
                .setShowWhen(false)
                .setVisibility(Notification.VISIBILITY_PUBLIC)
                .addAction(action(android.R.drawable.ic_media_previous, "Precedente", PREVIOUS))
                .addAction(playing
                        ? action(android.R.drawable.ic_media_pause, "Pausa", TOGGLE)
                        : action(android.R.drawable.ic_media_play, "Riproduci", TOGGLE))
                .addAction(action(android.R.drawable.ic_media_next, "Successivo", NEXT))
                .setStyle(new Notification.MediaStyle()
                        .setMediaSession(session.getSessionToken())
                        .setShowActionsInCompactView(0, 1, 2));
        if (open != null) {
            notification.setContentIntent(open);
        }
        notifications.notify(NOTIFICATION, notification.build());
    }

    private void clear() {
        session.setPlaybackState(new PlaybackState.Builder()
                .setState(PlaybackState.STATE_STOPPED, 0, 0f)
                .build());
        session.setActive(false);
        notifications.cancel(NOTIFICATION);
        art = null;
    }

    private Notification.Action action(int icon, String label, int code) {
        Intent intent = new Intent(ACTION).setPackage(activity.getPackageName()).putExtra(CODE, code);
        PendingIntent pending = PendingIntent.getBroadcast(activity, code, intent,
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        return new Notification.Action.Builder(Icon.createWithResource(activity, icon), label, pending).build();
    }

    /** Il marchio bianco di `res/drawable`; senza, l'icona di Play di Android. */
    private int smallIcon() {
        int id = activity.getResources().getIdentifier("ic_stat_psytify", "drawable", activity.getPackageName());
        return id != 0 ? id : android.R.drawable.ic_media_play;
    }

    @Override
    public void onPlay() {
        command(PLAY, 0);
    }

    @Override
    public void onPause() {
        command(PAUSE, 0);
    }

    @Override
    public void onStop() {
        command(PAUSE, 0);
    }

    @Override
    public void onSkipToNext() {
        command(NEXT, 0);
    }

    @Override
    public void onSkipToPrevious() {
        command(PREVIOUS, 0);
    }

    @Override
    public void onSeekTo(long position) {
        command(SEEK, position);
    }
}
