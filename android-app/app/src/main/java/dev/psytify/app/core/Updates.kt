package dev.psytify.app.core

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageInstaller
import android.os.Build
import android.util.Log
import dev.psytify.app.MainActivity
import dev.psytify.app.R
import java.io.File
import java.io.IOException

/**
 * The app's own update: the core downloads the package into the app's files
 * (`DownloadsCmd::DownloadUpdate`), this hands it to `PackageInstaller`, and Android asks the person
 * to confirm. The package must be signed with the app's key, or Android refuses it.
 */
object Updates {
    private const val TAG = "PsytifyUpdate"
    private const val CHANNEL = "updates"
    private const val NOTIFICATION = 3

    /** Streams the package into an installer session and commits it; the result comes to [Result]. */
    fun install(context: Context, path: String) {
        val app = context.applicationContext
        val installer = app.packageManager.packageInstaller
        val params = PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL)
        params.setAppPackageName(app.packageName)
        val id = try {
            installer.createSession(params)
        } catch (e: IOException) {
            Log.w(TAG, "No installer session: ${e.javaClass.simpleName}")
            CoreHub.notice("L'installazione non è partita", true)
            return
        }
        try {
            installer.openSession(id).use { session ->
                File(path).inputStream().use { input ->
                    session.openWrite("psytify.apk", 0, File(path).length()).use { output ->
                        input.copyTo(output)
                        session.fsync(output)
                    }
                }
                val result = Intent(app, Result::class.java)
                // Android fills the result in: a mutable PendingIntent from Android 12.
                val flags = PendingIntent.FLAG_UPDATE_CURRENT or
                    (if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) PendingIntent.FLAG_MUTABLE else 0)
                session.commit(PendingIntent.getBroadcast(app, id, result, flags).intentSender)
            }
        } catch (e: Exception) {
            installer.abandonSession(id)
            Log.w(TAG, "Install not started: ${e.javaClass.simpleName}")
            CoreHub.notice("L'installazione non è partita", true)
        }
    }

    /** A new release found while the app stayed open for hours: one notification, a tap opens the app. */
    fun notifyOffered(context: Context, tag: String) {
        val app = context.applicationContext
        val manager = app.getSystemService(NotificationManager::class.java) ?: return
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, "Aggiornamenti", NotificationManager.IMPORTANCE_DEFAULT),
        )
        val open = PendingIntent.getActivity(
            app,
            0,
            Intent(app, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        val notification = Notification.Builder(app, CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_psytify)
            .setContentTitle("Nuova versione di Psytify")
            .setContentText("$tag è pronta da scaricare in Impostazioni")
            .setContentIntent(open)
            .setAutoCancel(true)
            .build()
        // Without the permission (Android 13+, refused) the notice in the app is enough.
        runCatching { manager.notify(NOTIFICATION, notification) }
    }

    /** The installer's answer: the confirmation to show, or why it did not install. */
    class Result : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            when (val status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE)) {
                PackageInstaller.STATUS_PENDING_USER_ACTION -> {
                    @Suppress("DEPRECATION")
                    val confirm = intent.getParcelableExtra<Intent>(Intent.EXTRA_INTENT) ?: return
                    context.startActivity(confirm.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
                }
                PackageInstaller.STATUS_SUCCESS -> {}
                PackageInstaller.STATUS_FAILURE_ABORTED -> {}
                else -> {
                    Log.w(TAG, "Update not installed: status $status")
                    CoreHub.notice("Aggiornamento non installato. Riprova da Impostazioni.", true)
                }
            }
        }
    }
}
