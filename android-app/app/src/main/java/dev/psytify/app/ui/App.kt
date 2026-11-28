package dev.psytify.app.ui

import androidx.activity.compose.BackHandler
import androidx.activity.compose.PredictiveBackHandler
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.ui.semantics.clearAndSetSemantics
import dev.psytify.app.rust.SettingsAction
import kotlin.coroutines.cancellation.CancellationException
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBars
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import dev.psytify.app.design.LocalReduceMotion
import dev.psytify.app.rust.MotionChoice
import dev.psytify.app.rust.reduceMotion
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.saveable.rememberSaveableStateHolder
import androidx.compose.runtime.snapshots.SnapshotStateMap
import androidx.compose.ui.Alignment
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.lifecycle.compose.LifecycleStartEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.psytify.app.core.CoreHub
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.rust.HomeAction
import dev.psytify.app.rust.Route
import dev.psytify.app.rust.Tap
import dev.psytify.app.rust.DialogView
import dev.psytify.app.rust.MenuView
import dev.psytify.app.rust.PageAction
import dev.psytify.app.design.IconButton
import dev.psytify.app.design.PsytifySprites
import androidx.compose.runtime.remember
import androidx.compose.ui.platform.LocalContext
import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.os.Build
import dev.psytify.app.rust.routeKey
import dev.psytify.app.rust.routeOfKey

/** Pages kept in one tab's back stack, at most (as the desktop's history). */
private const val MAX_STACK = 50

/** The settings screen in a back stack: not a page of the core, so not a `routeKey`. */
private const val SETTINGS_KEY = "settings-screen"

/** Separates the tab from the route key when the stacks are saved. */
private const val STACK_SEP = '\u001e'

/** Each tab's back stack of route keys (`routeKey`), saved across a process death. */
private val StacksSaver = listSaver<SnapshotStateMap<Tab, List<String>>, String>(
    save = { stacks -> stacks.flatMap { (tab, keys) -> keys.map { "${tab.name}$STACK_SEP$it" } } },
    restore = { saved ->
        val stacks = mutableStateMapOf<Tab, List<String>>()
        saved.forEach { entry ->
            val parts = entry.split(STACK_SEP, limit = 2)
            val tab = Tab.entries.find { it.name == parts.first() }
            if (tab != null && parts.size == 2) stacks[tab] = stacks[tab].orEmpty() + parts[1]
        }
        stacks
    },
)

/**
 * The phone shell: the chosen tab's screen (its first screen, or the top of its back stack), the
 * mini player above the tab bar and, over all of it, the full-screen player. Back closes the queue,
 * then the player, then the open page, then goes back to Home. What is open survives a rotation and
 * a process death (saveable state); what plays and what the pages show come back from the core.
 */
@Composable
fun PsytifyApp() {
    val settings by CoreHub.settings.collectAsStateWithLifecycle()
    val system = rememberSystemMotion()
    val motion = settings?.motion ?: MotionChoice.SYSTEM
    val still = remember(motion, system) { reduceMotion(motion, system.animationsOff, system.powerSaver) }
    CompositionLocalProvider(LocalReduceMotion provides still) { Shell() }
}

@Composable
private fun Shell() {
    val core = CoreHub.core
    Box(Modifier.fillMaxSize().background(PsytifyColors.SURFACE_CANVAS)) {
        if (core == null) {
            PsytifyText(
                CoreHub.startError ?: "Il core non è partito.",
                PsytifyType.TYPE_BODY,
                PsytifyColors.TEXT_ERROR,
                Modifier.padding(WindowInsets.statusBars.asPaddingValues()).padding(PsytifySpace.PHONE_GUTTER),
                maxLines = 4,
            )
            return@Box
        }
        val home by CoreHub.home.collectAsStateWithLifecycle()
        val marks by CoreHub.marks.collectAsStateWithLifecycle()
        val playback by CoreHub.playback.collectAsStateWithLifecycle()
        var tab by rememberSaveable { mutableStateOf(Tab.HOME) }
        var playerOpen by rememberSaveable { mutableStateOf(false) }
        var panel by rememberSaveable { mutableStateOf(PlayerPanel.MEDIA) }
        val settings by CoreHub.settings.collectAsStateWithLifecycle()
        // «Video» on the media slot, a UI preference in config.json: shown until the core says it.
        var clipChosen by remember { mutableStateOf<Boolean?>(null) }
        val clipOn = clipChosen ?: settings?.clip ?: false
        LaunchedEffect(settings?.clip) { clipChosen = null }
        val stacks = rememberSaveable(saver = StacksSaver) { mutableStateMapOf() }
        var query by rememberSaveable { mutableStateOf("") }
        val failure by CoreHub.failure.collectAsStateWithLifecycle()
        val saved = rememberSaveableStateHolder()
        val stack = stacks[tab].orEmpty()
        fun open(route: Route) {
            playerOpen = false
            stacks[tab] = (stacks[tab].orEmpty() + routeKey(route)).takeLast(MAX_STACK)
        }
        var menu by remember { mutableStateOf<MenuView?>(null) }
        var dialog by remember { mutableStateOf<DialogView?>(null) }
        val context = LocalContext.current
        fun onTap(tap: Tap) {
            when (tap) {
                is Tap.Open -> open(tap.route)
                is Tap.Search -> query = tap.query
                is Tap.Copy -> copyText(context, tap.text)
                is Tap.Share -> shareText(context, tap.text)
                is Tap.Dialog -> dialog = tap.dialog
                is Tap.Leave -> {
                    val gone = routeKey(tap.route)
                    Tab.entries.forEach { each -> stacks[each]?.let { keys -> stacks[each] = keys.filter { it != gone } } }
                }
                is Tap.Nothing -> {}
            }
        }
        fun showMenu(view: MenuView) {
            menu = view
        }
        LaunchedEffect(Unit) { CoreHub.opens.collect { route -> open(route) } }
        // Each time the app comes back: the Home loads if it has not (the core may have restarted).
        LifecycleStartEffect(core) {
            core.home(HomeAction.ENSURE)
            onStopOrDispose {}
        }
        val current = playback?.takeIf { it.snapshot.track != null }
        val showPlayer = playerOpen && current != null

        BackHandler(enabled = showPlayer && panel != PlayerPanel.MEDIA) { panel = PlayerPanel.MEDIA }
        // Predictive back on the player: it lowers with the gesture and closes when it completes.
        var backProgress by remember { mutableFloatStateOf(0f) }
        PredictiveBackHandler(enabled = showPlayer && panel == PlayerPanel.MEDIA) { events ->
            try {
                events.collect { event -> backProgress = event.progress }
                playerOpen = false
            } catch (cancelled: CancellationException) {
                throw cancelled
            } finally {
                backProgress = 0f
            }
        }
        BackHandler(enabled = !showPlayer && stack.isNotEmpty()) { stacks[tab] = stack.dropLast(1) }
        BackHandler(enabled = !showPlayer && stack.isEmpty() && tab != Tab.HOME) { tab = Tab.HOME }
        // With Home at its root no handler is on: Android plays its own back-to-home animation.

        val covered = menu != null || dialog != null
        // What a sheet or the player covers is out of TalkBack's reach: focus stays on top.
        val hidden = Modifier.clearAndSetSemantics {}

        // The screens stop under the status bar: nothing scrolls behind its icons.
        Column(Modifier.fillMaxSize().then(if (showPlayer || covered) hidden else Modifier).statusBarsPadding()) {
            Box(Modifier.weight(1f)) {
                val screen = Modifier.fillMaxSize()
                val top = stack.lastOrNull()
                val route = top?.let(::routeOfKey)
                // Each place keeps its scroll while another is on top of it.
                saved.SaveableStateProvider("${tab.name}/${stack.size}/${top.orEmpty()}") {
                    val back = { stacks[tab] = stack.dropLast(1) }
                    when {
                        top == SETTINGS_KEY -> SettingsScreen(core, onBack = back, modifier = screen)
                        route is Route.Friends -> FriendsScreen(core, rememberPage(core, route), ::onTap, back, ::showMenu, screen)
                        route is Route.Jam -> JamScreen(core, rememberPage(core, route), ::onTap, back, ::showMenu, screen)
                        route != null -> PageScreen(
                            rememberPage(core, route),
                            ::onTap,
                            screen,
                            onBack = { stacks[tab] = stack.dropLast(1) },
                            onMenu = ::showMenu,
                        )
                        tab == Tab.HOME -> HomeScreen(
                            home = home,
                            onTile = { version, shelf, tile ->
                                onTap(core.activateHome(version, shelf.toUInt(), tile.toUInt()))
                            },
                            onRetry = { core.home(HomeAction.RETRY) },
                            modifier = screen,
                        )
                        tab == Tab.SEARCH -> SearchScreen(
                            rememberPage(core, Route.Search),
                            query,
                            onQuery = { text ->
                                query = text
                                core.searchInput(text)
                            },
                            onTap = ::onTap,
                            onMenu = ::showMenu,
                            modifier = screen,
                        )
                        tab == Tab.LIBRARY -> {
                            val library = rememberPage(core, Route.Library)
                            PageScreen(
                                library,
                                ::onTap,
                                screen,
                                onMenu = ::showMenu,
                                top = {
                                    item(key = "title") {
                                        PageTitle("Libreria") {
                                            IconButton(PsytifySprites.ICON_PLUS, "Nuova playlist", {
                                                library.view.value?.let { view ->
                                                    onTap(library.handle.act(view.version, PageAction.NEW_PLAYLIST))
                                                }
                                            })
                                        }
                                    }
                                },
                            )
                        }
                        tab == Tab.PROFILE -> ProfileTab(
                            core,
                            ::onTap,
                            ::showMenu,
                            onOpen = ::open,
                            onSettings = { stacks[tab] = (stack + SETTINGS_KEY).takeLast(MAX_STACK) },
                            modifier = screen,
                        )
                        else -> Placeholder(tab.label, screen)
                    }
                }
                Notices(Modifier.align(Alignment.BottomCenter))
            }
            failure?.let { reason -> CoreProblem(reason, onRetry = { core.retryStart() }) }
            current?.let {
                MiniPlayer(
                    it,
                    marks,
                    send = core::playback,
                    onOpen = { playerOpen = true },
                    onJam = {
                        // The jam lives in the Profilo tab, as its way in.
                        val jam = routeKey(Route.Jam)
                        val profile = stacks[Tab.PROFILE].orEmpty()
                        if (profile.lastOrNull() != jam) stacks[Tab.PROFILE] = (profile + jam).takeLast(MAX_STACK)
                        tab = Tab.PROFILE
                    },
                )
            }
            TabBar(
                tab,
                onSelect = { chosen ->
                    // The tab that is already open goes back to its first screen.
                    if (chosen == tab) stacks[tab] = emptyList() else tab = chosen
                },
            )
            Box(Modifier.navigationBarsPadding())
        }
        if (showPlayer) {
            Box(if (covered) hidden else Modifier) {
                PlayerScreen(
                    playback = current,
                    core = core,
                    panel = panel,
                    onPanel = { panel = it },
                    clipOn = clipOn,
                    onClipOn = { on ->
                        clipChosen = on
                        core.settings(SettingsAction.Clip(on))
                    },
                    onClose = { playerOpen = false },
                    onMenu = ::showMenu,
                    backProgress = backProgress,
                )
            }
        }
        menu?.let { open ->
            TrackSheet(
                open,
                onChoose = { choice ->
                    menu = null
                    onTap(core.choose(open.id, choice))
                },
                onDismiss = {
                    menu = null
                    core.closeMenu(open.id)
                },
            )
        }
        dialog?.let { open ->
            PlaylistDialog(
                open,
                onConfirm = { text ->
                    dialog = null
                    onTap(core.confirmDialog(open.id, text))
                },
                onCancel = {
                    dialog = null
                    core.cancelDialog(open.id)
                },
            )
        }
    }
}

/** «Copia titolo e artista»: Android 13 and later confirm a copy themselves; before, a notice does. */
private fun copyText(context: Context, text: String) {
    val clipboard = context.getSystemService(ClipboardManager::class.java) ?: return
    clipboard.setPrimaryClip(ClipData.newPlainText("Psytify", text))
    if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) CoreHub.notice("Copiato negli appunti", false)
}

/** «Condividi»: the system's share sheet with the title and the artist. */
private fun shareText(context: Context, text: String) {
    val send = Intent(Intent.ACTION_SEND).setType("text/plain").putExtra(Intent.EXTRA_TEXT, text)
    context.startActivity(Intent.createChooser(send, null))
}
