package to.iris.drive.app

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.PlainTooltip
import androidx.compose.material3.Text
import androidx.compose.material3.TooltipAnchorPosition
import androidx.compose.material3.TooltipBox
import androidx.compose.material3.TooltipDefaults
import androidx.compose.material3.rememberTooltipState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import to.iris.drive.app.core.DeviceStorageFolderState
import to.iris.drive.app.core.DeviceStorageRowState
import to.iris.drive.app.core.DeviceStorageState

/** Each device of the profile and how much of the Drive it holds. Wording comes from the app core. */
@Composable
internal fun DeviceStorageSection(storage: DeviceStorageState) {
    if (storage.devices.isEmpty()) return
    Spacer(Modifier.height(12.dp))
    Box(Modifier.testTag("deviceStorageSection")) {
        CardSection(title = "Storage", trailing = "") {
            if (storage.summary.isNotBlank()) {
                Text(storage.summary, color = Muted)
            }
            storage.devices.forEach { device ->
                key(device.pubkey) {
                    DeviceStorageRow(device)
                }
            }
        }
    }
}

@Composable
private fun DeviceStorageRow(device: DeviceStorageRowState) {
    var expanded by rememberSaveable { mutableStateOf(false) }
    val toggle = if (device.hasReport) {
        Modifier.clickable(onClickLabel = if (expanded) "Hide folders" else "Show folders") {
            expanded = !expanded
        }
    } else {
        Modifier
    }
    Row(
        verticalAlignment = Alignment.Top,
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .then(toggle)
            .padding(vertical = 4.dp)
            .testTag("deviceStorageRow"),
    ) {
        Icon(
            painterResource(device.iconRes),
            contentDescription = null,
            tint = Muted,
            modifier = Modifier.size(20.dp),
        )
        Spacer(Modifier.size(12.dp))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
                    Text(
                        device.name,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f, fill = false),
                    )
                    if (device.isCurrent) {
                        Spacer(Modifier.size(6.dp))
                        Text(
                            "This device",
                            color = Muted,
                            style = MaterialTheme.typography.labelSmall,
                            modifier = Modifier
                                .clip(RoundedCornerShape(4.dp))
                                .background(Muted.copy(alpha = 0.12f))
                                .padding(horizontal = 6.dp, vertical = 1.dp),
                        )
                    }
                }
                Spacer(Modifier.size(8.dp))
                Text(
                    device.heldLabel,
                    color = if (device.holdsAll) OnlineGreen else Muted,
                    style = MaterialTheme.typography.bodySmall,
                    maxLines = 1,
                )
                if (device.hasReport) {
                    Icon(
                        painterResource(R.drawable.ic_expand_more),
                        contentDescription = null,
                        tint = Muted,
                        modifier = Modifier
                            .size(20.dp)
                            .rotate(if (expanded) 180f else 0f),
                    )
                }
            }
            if (!device.isCurrent) {
                DeviceStorageStatusLine(device)
            }
            if (device.hasReport) {
                StorageBar(permille = device.heldPermille, height = 4.dp)
                Row(verticalAlignment = Alignment.CenterVertically) {
                    HintedText(
                        text = device.spaceLabel,
                        hint = device.spaceDetail,
                        color = Muted,
                        modifier = Modifier.weight(1f),
                    )
                    Spacer(Modifier.size(8.dp))
                    SmallText(device.ageLabel, Muted)
                }
            } else if (device.isCurrent && device.ageLabel.isNotBlank()) {
                SmallText(device.ageLabel, Muted)
            }
            if (device.hasReport && expanded) {
                DeviceStorageFolders(device)
            }
        }
    }
}

@Composable
private fun DeviceStorageStatusLine(device: DeviceStorageRowState) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Box(
            modifier = Modifier
                .size(8.dp)
                .background(
                    color = if (device.connected) OnlineGreen else Muted,
                    shape = CircleShape,
                ),
        )
        Spacer(Modifier.size(6.dp))
        HintedText(text = device.connectionLabel, hint = device.connectionDetail, color = Muted)
        if (device.syncLabel.isNotBlank()) {
            SmallText(" · ", Muted)
            HintedText(text = device.syncLabel, hint = device.syncDetail, color = syncColor(device.syncState))
        }
    }
}

@Composable
private fun DeviceStorageFolders(device: DeviceStorageRowState) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(top = 4.dp)
            .testTag("deviceStorageFolders"),
        verticalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        device.folders.forEach { folder -> DeviceStorageFolderLine(folder) }
        device.notes.forEach { note -> SmallText(note, Muted) }
    }
}

@Composable
private fun DeviceStorageFolderLine(folder: DeviceStorageFolderState) {
    Column(
        modifier = Modifier.padding(start = (folder.depth.coerceAtLeast(0) * 12).dp),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(
                folder.label,
                style = MaterialTheme.typography.bodySmall,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            Spacer(Modifier.size(8.dp))
            SmallText(folder.detail, Muted)
        }
        StorageBar(permille = folder.heldPermille, height = 2.dp)
    }
}

@Composable
private fun StorageBar(permille: Int, height: Dp) {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .height(height)
            .clip(RoundedCornerShape(height))
            .background(Muted.copy(alpha = 0.2f)),
    ) {
        Box(
            modifier = Modifier
                .fillMaxWidth(permille.coerceIn(0, 1000) / 1000f)
                .fillMaxHeight()
                .background(Teal),
        )
    }
}

@Composable
private fun SmallText(text: String, color: Color, modifier: Modifier = Modifier) {
    Text(
        text,
        color = color,
        style = MaterialTheme.typography.bodySmall,
        maxLines = 1,
        overflow = TextOverflow.Ellipsis,
        modifier = modifier,
    )
}

/** Small text whose detail shows on long press and is read by accessibility services. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun HintedText(text: String, hint: String, color: Color, modifier: Modifier = Modifier) {
    if (text.isBlank()) return
    if (hint.isBlank()) {
        SmallText(text, color, modifier)
        return
    }
    TooltipBox(
        positionProvider = TooltipDefaults.rememberTooltipPositionProvider(TooltipAnchorPosition.Above),
        tooltip = { PlainTooltip { Text(hint) } },
        state = rememberTooltipState(),
        modifier = modifier,
    ) {
        SmallText(text, color, Modifier.semantics { contentDescription = "$text. $hint" })
    }
}

@Composable
private fun syncColor(syncState: String): Color =
    when (syncState) {
        "in-sync" -> OnlineGreen
        "behind", "apart" -> StorageWarning
        else -> Muted
    }

private val StorageWarning = Color(0xFFF5A524)

private val DeviceStorageRowState.iconRes: Int
    get() = when (icon) {
        "web" -> R.drawable.ic_device_web
        "phone" -> R.drawable.ic_device_phone
        "computer" -> R.drawable.ic_device_computer
        else -> R.drawable.ic_device_drive
    }
