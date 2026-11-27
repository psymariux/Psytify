package dev.psytify.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.setProgress
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.platform.LocalContext
import dev.psytify.app.core.Updates
import dev.psytify.app.design.GhostButton
import dev.psytify.app.design.PrimaryButton
import dev.psytify.app.rust.UpdateControl
import androidx.compose.ui.unit.IntSize
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import dev.psytify.app.core.CoreHub
import dev.psytify.app.design.AnimatedSprite
import dev.psytify.app.design.IconButton
import dev.psytify.app.design.Pill
import dev.psytify.app.design.PixelSwitch
import dev.psytify.app.design.PsytifyColors
import dev.psytify.app.design.PsytifySpace
import dev.psytify.app.design.PsytifySprites
import dev.psytify.app.design.PsytifyText
import dev.psytify.app.design.PsytifyType
import dev.psytify.app.design.SecondaryButton
import dev.psytify.app.design.ap
import dev.psytify.app.rust.AccountLine
import dev.psytify.app.rust.MotionChoice
import dev.psytify.app.rust.PsytifyCore
import dev.psytify.app.rust.Select
import dev.psytify.app.rust.SettingsAction
import dev.psytify.app.rust.SettingsSnapshot
import dev.psytify.app.rust.motionDetail

/**
 * «Impostazioni», from the Profilo tab: groups of rows with the name and the explanation, the
 * control on the right or under them. Every value comes from the core's snapshot and every change
 * goes back as a `SettingsAction`; nothing is kept here but a band while it is dragged.
 */
@Composable
fun SettingsScreen(core: PsytifyCore, onBack: () -> Unit, modifier: Modifier = Modifier) {
    val settings by CoreHub.settings.collectAsStateWithLifecycle()
    val system = rememberSystemMotion()
    val shown = settings
    LazyColumn(modifier, contentPadding = PaddingValues(bottom = PsytifySpace.SPACE_7)) {
        item(key = "bar") {
            Row(
                Modifier.fillMaxWidth().height(PsytifySpace.PHONE_TOP_BAR_H).padding(horizontal = PsytifySpace.SPACE_2),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IconButton(PsytifySprites.ICON_CHEVRON_LEFT, "Indietro", onBack)
            }
        }
        item(key = "title") {
            PsytifyText(
                "Impostazioni",
                PsytifyType.TYPE_PAGE_TITLE,
                PsytifyColors.TEXT_PRIMARY,
                Modifier.padding(horizontal = PsytifySpace.PHONE_GUTTER).semantics { heading() },
            )
        }
        if (shown == null) return@LazyColumn
        val send: (SettingsAction) -> Unit = { core.settings(it) }
        playback(shown, system, send)
        sound(shown, send)
        downloads(shown, send)
        account(shown, send)
        about(shown, core, send)
    }
}

private fun LazyListScope.playback(shown: SettingsSnapshot, system: SystemMotion, send: (SettingsAction) -> Unit) {
    heading("h-quality", "Qualità")
    item(key = "quality") {
        Setting("Qualità d'ascolto", shown.qualityDetail) {
            Choices(shown.quality) { send(SettingsAction.Quality(it.toUInt())) }
        }
    }
    heading("h-playback", "Riproduzione")
    item(key = "autoplay") {
        Setting(
            "Riproduzione automatica",
            "Quando la coda finisce, continua con brani dello stesso artista.",
            trailing = { PixelSwitch(shown.autoplay, { send(SettingsAction.Autoplay(it)) }, "Riproduzione automatica") },
        )
    }
    item(key = "sleep") {
        Setting("Timer di spegnimento", "La musica si ferma da sola dopo il tempo scelto.") {
            Choices(shown.sleep) { send(SettingsAction.Sleep(it.toUInt())) }
        }
    }
    heading("h-motion", "Interfaccia")
    item(key = "motion") {
        val detail = motionDetail(shown.motion, system.animationsOff, system.powerSaver)
        Setting("Riduci il movimento", detail) {
            val choices = MotionChoice.entries
            Choices(Select(shown.motionOptions, choices.indexOf(shown.motion).toUInt())) { index ->
                choices.getOrNull(index)?.let { send(SettingsAction.Motion(it)) }
            }
        }
    }
}

private fun LazyListScope.sound(shown: SettingsSnapshot, send: (SettingsAction) -> Unit) {
    heading("h-eq", "Equalizzatore")
    item(key = "eq") {
        Setting(
            "Equalizzatore",
            "Dieci bande da 31 Hz a 16 kHz, applicate prima dell'uscita audio.",
            trailing = { PixelSwitch(shown.eqEnabled, { send(SettingsAction.EqEnabled(it)) }, "Equalizzatore") },
        )
    }
    item(key = "eq-preset") {
        Setting("Preset", null) {
            Choices(shown.eqPreset) { index ->
                // «Personalizzato» is not a preset: it only says the bands were moved by hand.
                if (index < shown.eqPreset.options.size && shown.eqPreset.options[index] != "Personalizzato") {
                    send(SettingsAction.EqPreset(index.toUInt()))
                }
            }
        }
    }
    item(key = "eq-bands") {
        Bands(shown, send)
    }
}

private fun LazyListScope.downloads(shown: SettingsSnapshot, send: (SettingsAction) -> Unit) {
    heading("h-downloads", "Download")
    item(key = "download-quality") {
        Setting("Qualità dei download", shown.downloadDetail) {
            Choices(shown.downloadQuality) { send(SettingsAction.DownloadQuality(it.toUInt())) }
        }
    }
    item(key = "folder") { Setting("Cartella", shown.folder) }
    item(key = "space") { Setting("Spazio", shown.space) }
}

private fun LazyListScope.account(shown: SettingsSnapshot, send: (SettingsAction) -> Unit) {
    heading("h-account", "Account")
    item(key = "account") {
        when (val line = shown.account) {
            is AccountLine.SignedIn -> Setting("Account", line.text) {
                SecondaryButton("Esci", { send(SettingsAction.SignOut) })
            }
            AccountLine.SignedOut -> Setting("Account", "Non hai fatto l'accesso: puoi farlo dal Profilo.")
            AccountLine.Working -> Setting("Account", "Accesso in corso…")
            AccountLine.Unavailable -> Setting("Account", "L'accesso non è configurato in questa versione dell'app.")
        }
    }
    shown.shareActivity?.let { on ->
        item(key = "share") {
            Setting(
                "Condividi cosa ascolti",
                "Gli amici vedono il brano in corso e gli ascolti recenti.",
                trailing = { PixelSwitch(on, { send(SettingsAction.ShareActivity(it)) }, "Condividi cosa ascolti") },
            )
        }
    }
}

private fun LazyListScope.about(shown: SettingsSnapshot, core: PsytifyCore, send: (SettingsAction) -> Unit) {
    heading("h-about", "Informazioni")
    item(key = "update") { UpdateRow(shown, core, send) }
    item(key = "about") {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = PsytifySpace.PHONE_GUTTER, vertical = PsytifySpace.SPACE_3),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            AnimatedSprite(PsytifySprites.NASTRO_IDLE, running = true)
            Spacer(Modifier.width(PsytifySpace.SPACE_5))
            Column {
                PsytifyText("Psytify", PsytifyType.TYPE_WORDMARK, PsytifyColors.TEXT_PRIMARY)
                PsytifyText(shown.version, PsytifyType.TYPE_META, PsytifyColors.TEXT_SECONDARY)
            }
        }
    }
    heading("h-credits", "Crediti e licenze")
    shown.credits.forEach { credit ->
        item(key = "credit:${credit.title}") { Setting(credit.title, credit.text) }
    }
}

private fun LazyListScope.heading(key: String, title: String) {
    item(key = key) {
        PsytifyText(
            title,
            PsytifyType.TYPE_HEADING,
            PsytifyColors.TEXT_PRIMARY,
            Modifier
                .padding(horizontal = PsytifySpace.PHONE_GUTTER)
                .padding(top = PsytifySpace.SPACE_6, bottom = PsytifySpace.SPACE_2)
                .semantics { heading() },
        )
    }
}

/** A setting: its name and explanation, a control on the right (`trailing`) or under them (`below`). */
@Composable
private fun Setting(
    name: String,
    detail: String?,
    trailing: (@Composable () -> Unit)? = null,
    below: (@Composable () -> Unit)? = null,
) {
    Column(Modifier.fillMaxWidth().padding(horizontal = PsytifySpace.PHONE_GUTTER, vertical = PsytifySpace.SPACE_3)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.weight(1f)) {
                PsytifyText(name, PsytifyType.TYPE_TRACK_TITLE, PsytifyColors.TEXT_PRIMARY)
                if (!detail.isNullOrEmpty()) {
                    PsytifyText(detail, PsytifyType.TYPE_ARTIST, PsytifyColors.TEXT_SECONDARY, maxLines = 6)
                }
            }
            trailing?.invoke()
        }
        if (below != null) {
            Spacer(Modifier.height(PsytifySpace.SPACE_3))
            below()
        }
    }
}

/** A few choices as pills on wrapping lines: the chosen one selected, a radio group for TalkBack. */
@Composable
private fun Choices(select: Select, onChoose: (Int) -> Unit) {
    FlowRow(
        Modifier.selectableGroup(),
        horizontalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_2),
        verticalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_1),
    ) {
        select.options.forEachIndexed { index, label ->
            Pill(label, select.chosen?.toInt() == index, { onChoose(index) }, role = Role.RadioButton)
        }
    }
}

/**
 * «Aggiornamento»: the last check with «Controlla ora», the offer with «Scarica», the ten-step bar
 * while the package downloads, «Installa» once it is in the app's files (Android asks to confirm).
 */
@Composable
private fun UpdateRow(shown: SettingsSnapshot, core: PsytifyCore, send: (SettingsAction) -> Unit) {
    val line = shown.update
    val context = LocalContext.current
    Setting("Aggiornamento", line.text) {
        Row(horizontalArrangement = Arrangement.spacedBy(PsytifySpace.SPACE_3), verticalAlignment = Alignment.CenterVertically) {
            when (val control = line.control) {
                is UpdateControl.Check -> SecondaryButton(
                    if (control.checking) "Controllo…" else "Controlla ora",
                    { send(SettingsAction.CheckUpdate) },
                    enabled = !control.checking,
                )
                UpdateControl.Download -> PrimaryButton("Scarica", { send(SettingsAction.DownloadUpdate) })
                is UpdateControl.Steps -> {
                    val percent = control.percent?.toInt()
                    if (percent == null) {
                        AnimatedSprite(
                            PsytifySprites.LOADER,
                            running = true,
                            modifier = Modifier.semantics { contentDescription = "Download in corso" },
                            tint = PsytifyColors.ICON_IDLE,
                        )
                    } else {
                        UpdateSteps(percent)
                    }
                }
                UpdateControl.Install -> PrimaryButton("Installa", {
                    core.updatePackage()?.let { path -> Updates.install(context, path) }
                })
            }
            if (line.closable) GhostButton("Non ora", { send(SettingsAction.DismissUpdate) })
        }
    }
}

/** The ten steps of 2 art pixels of a transfer: the reached ones white (never lime). */
@Composable
private fun UpdateSteps(percent: Int) {
    val lit = (percent / 10).coerceIn(0, 10)
    Row(
        Modifier.semantics { contentDescription = "$percent%" },
        horizontalArrangement = Arrangement.spacedBy(ap(1)),
    ) {
        repeat(10) { step ->
            Box(Modifier.size(ap(2)).background(if (step < lit) PsytifyColors.WARM_WHITE else PsytifyColors.IRON))
        }
    }
}

/**
 * The ten bands as vertical sliders (2 AP wide, 64 AP tall, the zero line across): a drag or a tap
 * moves a band at once (`EqBand`), the release saves (`EqCommit`). Dimmed while the equalizer is off.
 */
@Composable
private fun Bands(shown: SettingsSnapshot, send: (SettingsAction) -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(horizontal = PsytifySpace.PHONE_GUTTER, vertical = PsytifySpace.SPACE_3),
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        shown.bands.forEachIndexed { index, band ->
            BandSlider(index, band.label, band.gainDb, band.text, shown.maxGainDb, shown.eqEnabled, send)
        }
    }
}

@Composable
private fun BandSlider(
    index: Int,
    label: String,
    gain: Float,
    text: String,
    max: Float,
    lit: Boolean,
    send: (SettingsAction) -> Unit,
) {
    var size by remember { mutableStateOf(IntSize.Zero) }
    var dragging by remember { mutableStateOf<Float?>(null) }
    val shownGain = dragging ?: gain
    val fraction = ((shownGain + max) / (2 * max)).coerceIn(0f, 1f)
    fun gainAt(y: Float): Float {
        val height = size.height.toFloat().coerceAtLeast(1f)
        val at = (1f - y / height).coerceIn(0f, 1f)
        return -max + at * 2 * max
    }
    val track = if (lit) PsytifyColors.TEXT_PRIMARY else PsytifyColors.STONE
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        Box(
            Modifier
                .width(PsytifySpace.SPACE_6)
                .height(ap(64))
                .onSizeChanged { size = it }
                .semantics {
                    contentDescription = "Banda $label Hz"
                    stateDescription = "$text dB"
                    // TalkBack's adjust gestures move the band too, and save at once.
                    progressBarRangeInfo = ProgressBarRangeInfo(gain, -max..max, steps = (4 * max).toInt() - 1)
                    setProgress { value ->
                        send(SettingsAction.EqBand(index.toUInt(), value.coerceIn(-max, max)))
                        send(SettingsAction.EqCommit)
                        true
                    }
                }
                .pointerInput(index, max) {
                    detectTapGestures { offset ->
                        send(SettingsAction.EqBand(index.toUInt(), gainAt(offset.y)))
                        send(SettingsAction.EqCommit)
                    }
                }
                .pointerInput(index, max) {
                    detectVerticalDragGestures(
                        onDragStart = { offset -> dragging = gainAt(offset.y) },
                        onDragEnd = {
                            dragging = null
                            send(SettingsAction.EqCommit)
                        },
                        onDragCancel = { dragging = null },
                    ) { change, _ ->
                        change.consume()
                        val now = gainAt(change.position.y)
                        dragging = now
                        send(SettingsAction.EqBand(index.toUInt(), now))
                    }
                },
            contentAlignment = Alignment.Center,
        ) {
            Box(Modifier.width(ap(2)).fillMaxHeight().background(PsytifyColors.LINE_STRONG))
            // The zero line.
            Box(Modifier.width(ap(6)).height(ap(1)).background(PsytifyColors.IRON))
            Box(Modifier.fillMaxHeight().width(ap(6))) {
                val thumb = ap(4)
                val travel = ap(64) - thumb
                Box(
                    Modifier
                        .padding(top = travel * (1f - fraction))
                        .size(ap(6), thumb)
                        .background(track),
                )
            }
        }
        Spacer(Modifier.height(PsytifySpace.SPACE_2))
        PsytifyText(text, PsytifyType.TYPE_NUMERIC, if (lit) PsytifyColors.TEXT_PRIMARY else PsytifyColors.TEXT_TERTIARY)
        PsytifyText(label, PsytifyType.TYPE_NUMERIC, PsytifyColors.TEXT_TERTIARY)
    }
}
