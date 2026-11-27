package dev.psytify.app.core

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Environment
import android.os.SystemClock
import android.util.Log
import dev.psytify.app.rust.AccountSnapshot
import dev.psytify.app.rust.CoreFailure
import dev.psytify.app.rust.CoreListener
import dev.psytify.app.rust.DownloadsSnapshot
import dev.psytify.app.rust.HomeSnapshot
import dev.psytify.app.rust.LyricsSnapshot
import dev.psytify.app.rust.NowPlaying
import dev.psytify.app.rust.PlaybackSnapshot
import dev.psytify.app.rust.PsytifyCore
import dev.psytify.app.rust.Route
import dev.psytify.app.rust.SettingsSnapshot
import dev.psytify.app.rust.SocialMarks
import dev.psytify.app.rust.VideoSnapshot
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * A playback snapshot and when it arrived. The core does not publish the position while a track
 * runs: it is the published one plus the time since arrival (the rule of the core's `Clock::at`).
 */
data class Playback(val snapshot: PlaybackSnapshot, val receivedAt: Long) {
    fun positionMs(now: Long = SystemClock.elapsedRealtime()): Long {
        val anchored = snapshot.positionMs.toLong()
        val position = if (snapshot.running) anchored + (now - receivedAt) else anchored
        val duration = snapshot.track?.durationMs?.toLong() ?: return position.coerceAtLeast(0)
        return position.coerceIn(0, duration)
    }
}

/** A short message for the person, as the core worded it. */
data class Notice(val text: String, val error: Boolean)

/**
 * The process's one core, and its domains as `StateFlow`s. The core lives as long as the process
 * (the activity and the media service both use it), and pushes each domain only when it changed;
 * the callbacks come from the bridge's thread, and a `StateFlow` takes them from any thread.
 */
object CoreHub : CoreListener {
    private const val TAG = "Psytify"

    private val playbackState = MutableStateFlow<Playback?>(null)
    val playback: StateFlow<Playback?> = playbackState.asStateFlow()

    private val homeState = MutableStateFlow<HomeSnapshot?>(null)
    val home: StateFlow<HomeSnapshot?> = homeState.asStateFlow()

    private val nowPlayingState = MutableStateFlow<NowPlaying?>(null)
    /** What the system's media controls show, as the core decided it. */
    val nowPlaying: StateFlow<NowPlaying?> = nowPlayingState.asStateFlow()

    private val failureState = MutableStateFlow<String?>(null)
    /** Why the core did not start again after a stop (the screen shows it with «Riprova»). */
    val failure: StateFlow<String?> = failureState.asStateFlow()

    private val accountState = MutableStateFlow<AccountSnapshot?>(null)
    /** The account and the sign-in form, as the core has them. */
    val account: StateFlow<AccountSnapshot?> = accountState.asStateFlow()

    private val downloadsState = MutableStateFlow(DownloadsSnapshot(false, "", "", null))
    /** The downloads as their notification shows them; while `running`, [DownloadService] is up. */
    val downloads: StateFlow<DownloadsSnapshot> = downloadsState.asStateFlow()

    private val marksState = MutableStateFlow(SocialMarks(false, false, 0u, 0u, 0u))
    /** The jam marker of the player bar and the counts of invites and friend requests. */
    val marks: StateFlow<SocialMarks> = marksState.asStateFlow()

    private val settingsState = MutableStateFlow<SettingsSnapshot?>(null)
    /** What the settings page shows, as the core has it. */
    val settings: StateFlow<SettingsSnapshot?> = settingsState.asStateFlow()

    private val lyricsState = MutableStateFlow<LyricsSnapshot?>(null)
    /** What «Testo» shows for the current track. */
    val lyrics: StateFlow<LyricsSnapshot?> = lyricsState.asStateFlow()

    private val videoState = MutableStateFlow<VideoSnapshot?>(null)
    /** The current track's clip as the media slot sees it (never its address). */
    val video: StateFlow<VideoSnapshot?> = videoState.asStateFlow()

    /** The application context, for the browser the core asks to open. */
    @Volatile
    private var appContext: Context? = null

    private val noticeEvents = MutableSharedFlow<Notice>(extraBufferCapacity = 8)
    /** Notices to show one at a time; dropped while no screen collects them. */
    val notices: SharedFlow<Notice> = noticeEvents.asSharedFlow()

    private val openEvents = MutableSharedFlow<Route>(extraBufferCapacity = 4)
    /** Pages the core asks to open (a catalogue artist now on the server). */
    val opens: SharedFlow<Route> = openEvents.asSharedFlow()

    /** Outlives the activity and the service: a stop started as they go away still finishes. */
    private val stopper = CoroutineScope(SupervisorJob() + Dispatchers.Default)

    @Volatile
    var core: PsytifyCore? = null
        private set

    /** Why the core did not start, if it did not. */
    @Volatile
    var startError: String? = null
        private set

    @Synchronized
    fun start(context: Context) {
        if (core != null || startError != null) return
        appContext = context.applicationContext
        try {
            NativeContext.init(context.applicationContext)
            // App-specific external storage: no permission, the files leave with the app. Null while
            // the storage is not mounted: the core then uses filesDir/Download.
            val music = context.getExternalFilesDir(Environment.DIRECTORY_MUSIC)?.absolutePath
            core = PsytifyCore.start(context.filesDir.absolutePath, music, this)
        } catch (e: CoreFailure) {
            startError = e.message ?: e.toString()
            Log.e(TAG, "The core did not start: $startError")
        } catch (e: UnsatisfiedLinkError) {
            startError = "libpsytify_mobile.so did not load"
            Log.e(TAG, startError, e)
        }
    }

    /** Stops the core now, after its last write of config.json; the next command starts it again. */
    fun stopNow() {
        val core = core ?: return
        stopper.launch { runCatching { core.stop() }.onFailure { Log.w(TAG, "Last save failed: ${it.message}") } }
    }

    override fun playbackChanged(playback: PlaybackSnapshot) {
        playbackState.value = Playback(playback, SystemClock.elapsedRealtime())
    }

    override fun homeChanged(home: HomeSnapshot) {
        homeState.value = home
    }

    override fun nowPlaying(now: NowPlaying?) {
        nowPlayingState.value = now
    }

    override fun notice(text: String, error: Boolean) {
        noticeEvents.tryEmit(Notice(text, error))
    }

    override fun open(route: Route) {
        openEvents.tryEmit(route)
    }

    override fun accountChanged(account: AccountSnapshot) {
        accountState.value = account
    }

    /**
     * Opens a sign-in provider's page in the browser; the core waits for its return on the loopback
     * address. The address carries a one-time state: it is never logged.
     */
    override fun openBrowser(url: String): Boolean {
        val context = appContext ?: return false
        val view = Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
        return try {
            context.startActivity(view)
            true
        } catch (e: ActivityNotFoundException) {
            Log.w(TAG, "No browser to open the sign-in page")
            false
        }
    }

    override fun downloadsChanged(downloads: DownloadsSnapshot) {
        val was = downloadsState.value.running
        downloadsState.value = downloads
        val context = appContext ?: return
        if (downloads.running && !was) DownloadService.start(context)
    }

    override fun marksChanged(marks: SocialMarks) {
        marksState.value = marks
    }

    override fun settingsChanged(settings: SettingsSnapshot) {
        settingsState.value = settings
    }

    override fun lyricsChanged(lyrics: LyricsSnapshot) {
        lyricsState.value = lyrics
    }

    override fun videoChanged(video: VideoSnapshot) {
        videoState.value = video
    }

    override fun installUpdate(path: String) {
        val context = appContext ?: return
        Updates.install(context, path)
    }

    override fun updateOffered(tag: String) {
        val context = appContext ?: return
        Updates.notifyOffered(context, tag)
    }

    override fun coreFailed(reason: String?) {
        failureState.value = reason
        if (reason != null) Log.e(TAG, "The core did not start again: $reason")
    }
}
