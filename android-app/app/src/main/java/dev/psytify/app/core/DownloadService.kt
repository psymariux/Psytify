package dev.psytify.app.core

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.util.Log
import dev.psytify.app.MainActivity
import dev.psytify.app.rust.DownloadsSnapshot
import kotlinx.coroutines.Job
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * The foreground service of the downloads. A process in the background with no foreground service is
 * cached, then frozen or killed, and a long transfer would stall; this one is up while the core says
 * downloads wait or run (`DownloadsSnapshot.running`) and shows them. The transfer itself is the
 * core's: the service only keeps the process alive and draws the notification.
 */
class DownloadService : Service() {
    private val scope = MainScope()
    private var follow: Job? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        val manager = getSystemService(NotificationManager::class.java)
        manager?.createNotificationChannel(
            NotificationChannel(CHANNEL, "Download", NotificationManager.IMPORTANCE_LOW).apply {
                description = "I brani che si stanno scaricando"
                setShowBadge(false)
            },
        )
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val shown = CoreHub.downloads.value
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                startForeground(ID, notification(shown), ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
            } else {
                startForeground(ID, notification(shown))
            }
        } catch (e: IllegalStateException) {
            // Android 12+ refuses a start from the background; the downloads go on while they can.
            Log.w(TAG, "The downloads could not go to the foreground: ${e.message}")
            stopSelf()
            return START_NOT_STICKY
        }
        follow?.cancel()
        follow = scope.launch {
            CoreHub.downloads.collect { downloads ->
                if (!downloads.running) {
                    stopForeground(STOP_FOREGROUND_REMOVE)
                    stopSelf()
                } else {
                    getSystemService(NotificationManager::class.java)?.notify(ID, notification(downloads))
                }
            }
        }
        return START_NOT_STICKY
    }

    /** Android 15 limits a data sync service to six hours a day: then it must stop. */
    override fun onTimeout(startId: Int, fgsType: Int) {
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    override fun onDestroy() {
        scope.cancel()
        super.onDestroy()
    }

    private fun notification(downloads: DownloadsSnapshot): Notification {
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val builder = Notification.Builder(this, CHANNEL)
            .setSmallIcon(android.R.drawable.stat_sys_download)
            .setContentTitle(downloads.title)
            .setContentText(downloads.text)
            .setContentIntent(open)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(Notification.CATEGORY_PROGRESS)
        val percent = downloads.percent
        if (percent != null) builder.setProgress(100, percent.toInt(), false) else builder.setProgress(0, 0, true)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            builder.setForegroundServiceBehavior(Notification.FOREGROUND_SERVICE_IMMEDIATE)
        }
        return builder.build()
    }

    companion object {
        private const val TAG = "Psytify"
        private const val CHANNEL = "downloads"
        private const val ID = 2

        /** Starts the service; the downloads were asked from the app, so it is on screen. */
        fun start(context: Context) {
            val intent = Intent(context, DownloadService::class.java)
            try {
                context.startForegroundService(intent)
            } catch (e: IllegalStateException) {
                Log.w(TAG, "The download service did not start: ${e.message}")
            }
        }
    }
}
