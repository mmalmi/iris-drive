package to.iris.drive.app

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import org.json.JSONObject
import to.iris.drive.app.core.AppKeyLinkRequestState
import to.iris.drive.app.core.DeviceState
import to.iris.drive.app.core.NativeCore

@Composable
internal fun DevicesPanel(
    devices: List<DeviceState>,
    linkInvite: String,
    inboundRequests: List<AppKeyLinkRequestState>,
    canApprove: Boolean,
    onCopyLinkInvite: () -> Unit,
    onResetInvite: () -> Unit,
    onApproveDevice: (String, String) -> Unit,
    onRejectDevice: (String) -> Unit,
    onAddRecoveryKey: (String) -> Unit,
    onDeleteDevice: (String) -> Unit,
    onAppointAdmin: (String) -> Unit,
    onDemoteAdmin: (String) -> Unit,
) {
    var request by remember { mutableStateOf("") }
    var label by remember { mutableStateOf("") }
    var showAddDevice by remember { mutableStateOf(false) }
    var showAddRecoveryKey by remember { mutableStateOf(false) }
    var devicePendingDelete by remember { mutableStateOf<DeviceState?>(null) }
    val manualRequestIsComplete = remember(request) {
        NativeCore.isCompleteLinkInput(request)
    }

    CardSection(title = "Devices", trailing = "${devices.size}") {
        if (canApprove) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(
                    onClick = { showAddDevice = true },
                    modifier = Modifier.testTag("addDeviceButton"),
                ) {
                    Text("Add Device")
                }
                OutlinedButton(
                    onClick = { showAddRecoveryKey = true },
                    modifier = Modifier.testTag("addRecoveryKeyButton"),
                ) {
                    Text("Add Recovery Key")
                }
            }
        }
        devices.forEach { device ->
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                DeviceStatusDot(device = device)
                Spacer(Modifier.size(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(device.displayLabel, fontWeight = FontWeight.SemiBold)
                    Text(
                        "${device.roleLabel} | ${device.stateLabel} | ${device.connectionLabel}",
                        color = Muted,
                        style = MaterialTheme.typography.bodySmall,
                    )
                    if (device.isCurrentDevice) {
                        Text(
                            "Device key: ${device.pubkey}",
                            color = Muted,
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                    Text(device.detail, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                if (device.canAppointAdmin) {
                    TextButton(onClick = { onAppointAdmin(device.pubkey) }) {
                        Text("Make admin")
                    }
                }
                if (device.canDemoteAdmin) {
                    TextButton(onClick = { onDemoteAdmin(device.pubkey) }) {
                        Text("Remove admin")
                    }
                }
                if (device.canRevoke) {
                    TextButton(onClick = { devicePendingDelete = device }) {
                        Icon(
                            painterResource(R.drawable.ic_delete),
                            contentDescription = null,
                            tint = Danger,
                        )
                        Text("Remove", color = Danger)
                    }
                }
            }
        }
    }

    if (showAddDevice) {
        AddDeviceDialog(
            linkInvite = linkInvite,
            inboundRequests = inboundRequests,
            canApprove = canApprove,
            request = request,
            manualRequestIsComplete = manualRequestIsComplete,
            label = label,
            onRequestChange = { request = it },
            onLabelChange = { label = it },
            onCopyLinkInvite = onCopyLinkInvite,
            onResetInvite = onResetInvite,
            onApproveDevice = onApproveDevice,
            onRejectDevice = onRejectDevice,
            onDismiss = { showAddDevice = false },
            onAdded = {
                request = ""
                label = ""
                showAddDevice = false
            },
        )
    }

    devicePendingDelete?.let { device ->
        DeleteDeviceDialog(
            device = device,
            onDismiss = { devicePendingDelete = null },
            onConfirm = {
                onDeleteDevice(device.pubkey)
                devicePendingDelete = null
            },
        )
    }

    if (showAddRecoveryKey) {
        AddRecoveryKeyDialog(
            onDismiss = { showAddRecoveryKey = false },
            onAddRecoveryKey = { recoveryPubkey ->
                onAddRecoveryKey(recoveryPubkey)
                showAddRecoveryKey = false
            },
        )
    }
}

@Composable
private fun DeviceStatusDot(device: DeviceState) {
    Box(
        modifier = Modifier
            .size(10.dp)
            .background(
                color = if (device.isOnline) OnlineGreen else Muted,
                shape = CircleShape,
            )
            .semantics { contentDescription = device.onlineIndicatorDescription }
            .testTag(if (device.isOnline) "deviceStatusDotOnline" else "deviceStatusDotOffline"),
    )
}

private val OnlineGreen = Color(0xFF16A34A)

private val DeviceState.onlineIndicatorDescription: String
    get() {
        val title = displayLabel.ifBlank { "Device" }
        return "$title ${if (isOnline) "online" else "offline"}"
    }

@Composable
private fun DeleteDeviceDialog(
    device: DeviceState,
    onDismiss: () -> Unit,
    onConfirm: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Remove Device?") },
        text = {
            Text("Remove ${device.label} from Iris Drive? This removes its access to future syncs.")
        },
        confirmButton = {
            TextButton(
                onClick = onConfirm,
                modifier = Modifier.testTag("confirmDeleteDevice"),
            ) {
                Text("Remove", color = Danger)
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}

@Composable
private fun AddDeviceDialog(
    linkInvite: String,
    inboundRequests: List<AppKeyLinkRequestState>,
    canApprove: Boolean,
    request: String,
    manualRequestIsComplete: Boolean,
    label: String,
    onRequestChange: (String) -> Unit,
    onLabelChange: (String) -> Unit,
    onCopyLinkInvite: () -> Unit,
    onResetInvite: () -> Unit,
    onApproveDevice: (String, String) -> Unit,
    onRejectDevice: (String) -> Unit,
    onDismiss: () -> Unit,
    onAdded: () -> Unit,
) {
    fun submitManualDevice() {
        if (!canApprove || !manualRequestIsComplete) return
        onApproveDevice(request, label)
        onAdded()
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add a Device") },
        text = {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                if (linkInvite.isNotBlank()) {
                    Text("Invite device", fontWeight = FontWeight.SemiBold)
                    QrCode(linkInvite, side = 220.dp, modifier = Modifier.align(Alignment.CenterHorizontally))
                    Text(linkInvite, color = Muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    OutlinedButton(onClick = onCopyLinkInvite) {
                        Text("Copy invite link")
                    }
                    OutlinedButton(onClick = onResetInvite) {
                        Text("Reset invite")
                    }
                }
                if (inboundRequests.isNotEmpty()) {
                    Text("Device requests", fontWeight = FontWeight.SemiBold)
                    inboundRequests.forEach { inbound ->
                        Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                            Column(Modifier.weight(1f)) {
                                Text(inbound.label.ifBlank { "New device" }, fontWeight = FontWeight.SemiBold)
                                Text(
                                    inbound.devicePubkey,
                                    color = Muted,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            }
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                TextButton(
                                    onClick = { onRejectDevice(inbound.requestLink) },
                                    enabled = canApprove,
                                ) {
                                    Text("Reject", color = Danger)
                                }
                                Button(
                                    onClick = { onApproveDevice(inbound.requestLink, inbound.label) },
                                    enabled = canApprove,
                                ) {
                                    Text("Add")
                                }
                            }
                        }
                    }
                }
                Text(
                    "Paste the device key or request link.",
                    color = Muted,
                )
                OutlinedTextField(
                    value = request,
                    onValueChange = onRequestChange,
                    modifier = Modifier.fillMaxWidth().testTag("manualDeviceId"),
                    singleLine = true,
                    label = { Text("Device key") },
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Next),
                )
                OutlinedTextField(
                    value = label,
                    onValueChange = onLabelChange,
                    modifier = Modifier.fillMaxWidth().testTag("manualDeviceName"),
                    singleLine = true,
                    label = { Text("Name (optional)") },
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                    keyboardActions = KeyboardActions(onDone = { submitManualDevice() }),
                )
            }
        },
        confirmButton = {
            Button(
                onClick = { submitManualDevice() },
                enabled = canApprove && manualRequestIsComplete,
                modifier = Modifier.testTag("manualDeviceAdd"),
            ) {
                Text("Add")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}

@Composable
private fun AddRecoveryKeyDialog(
    onDismiss: () -> Unit,
    onAddRecoveryKey: (String) -> Unit,
) {
    var mode by remember { mutableStateOf("choose") }
    var error by remember { mutableStateOf("") }
    var generatedWords by remember { mutableStateOf<List<String>>(emptyList()) }
    var generatedPubkey by remember { mutableStateOf("") }
    var generatedWordIndex by remember { mutableStateOf(0) }
    var importedWords by remember { mutableStateOf(List(12) { "" }) }
    var importedWordIndex by remember { mutableStateOf(0) }

    fun startGenerate() {
        val payload = runCatching { JSONObject(NativeCore.generateRecoveryKeyJson()) }
            .getOrElse { JSONObject().put("error", it.message ?: "Recovery key generation failed") }
        error = payload.optString("error")
        generatedWords = payload.optJSONArray("words").toStringList()
        generatedPubkey = payload.optString("recovery_pubkey")
        generatedWordIndex = 0
        if (error.isBlank() && (generatedWords.size != 12 || generatedPubkey.isBlank())) {
            error = "Recovery key generation failed"
        }
        mode = "generate"
    }

    fun startImport() {
        error = ""
        importedWords = List(12) { "" }
        importedWordIndex = 0
        mode = "import"
    }

    fun addImportedRecoveryKey() {
        val phrase = importedWords.joinToString(" ") { it.trim().lowercase() }
        val payload = runCatching { JSONObject(NativeCore.recoveryPubkeyForPhraseJson(phrase)) }
            .getOrElse { JSONObject().put("error", it.message ?: "Recovery key import failed") }
        val importError = payload.optString("error")
        val recoveryPubkey = payload.optString("recovery_pubkey")
        if (importError.isNotBlank() || recoveryPubkey.isBlank()) {
            error = importError.ifBlank { "Recovery key import failed" }
            return
        }
        onAddRecoveryKey(recoveryPubkey)
    }

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add Recovery Key") },
        text = {
            Column(
                modifier = Modifier.verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                if (error.isNotBlank()) {
                    Text(error, color = Danger)
                }
                when (mode) {
                    "choose" -> {
                        OutlinedButton(onClick = { startGenerate() }, modifier = Modifier.fillMaxWidth()) {
                            Text("Generate New")
                        }
                        OutlinedButton(onClick = { startImport() }, modifier = Modifier.fillMaxWidth()) {
                            Text("Import Existing")
                        }
                    }
                    "generate" -> {
                        if (error.isBlank()) {
                            Text("Write down each word. Iris Drive will only save the public recovery key.", color = Muted)
                            Text("Word ${generatedWordIndex + 1} of 12", fontWeight = FontWeight.SemiBold)
                            Text(
                                generatedWords.getOrNull(generatedWordIndex).orEmpty(),
                                style = MaterialTheme.typography.headlineMedium,
                                fontWeight = FontWeight.SemiBold,
                            )
                        }
                    }
                    "import" -> {
                        Text("Enter the recovery phrase one word at a time.", color = Muted)
                        OutlinedTextField(
                            value = importedWords[importedWordIndex],
                            onValueChange = { value ->
                                importedWords = importedWords.toMutableList().also {
                                    it[importedWordIndex] = value.trim().lowercase()
                                }
                            },
                            modifier = Modifier.fillMaxWidth(),
                            singleLine = true,
                            label = { Text("Word ${importedWordIndex + 1} of 12") },
                            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Next),
                        )
                    }
                }
            }
        },
        confirmButton = {
            when (mode) {
                "choose" -> {}
                "generate" -> {
                    Button(
                        onClick = {
                            if (generatedWordIndex < 11) {
                                generatedWordIndex += 1
                            } else {
                                onAddRecoveryKey(generatedPubkey)
                            }
                        },
                        enabled = error.isBlank() && generatedWords.size == 12 && generatedPubkey.isNotBlank(),
                    ) {
                        Text(if (generatedWordIndex == 11) "Add Recovery Key" else "Next")
                    }
                }
                "import" -> {
                    Button(
                        onClick = {
                            if (importedWordIndex < 11) {
                                importedWordIndex += 1
                            } else {
                                addImportedRecoveryKey()
                            }
                        },
                        enabled = importedWords[importedWordIndex].isNotBlank() &&
                            (importedWordIndex < 11 || importedWords.all { it.isNotBlank() }),
                    ) {
                        Text(if (importedWordIndex == 11) "Add Recovery Key" else "Next")
                    }
                }
            }
        },
        dismissButton = {
            TextButton(
                onClick = {
                    if (mode == "choose") {
                        onDismiss()
                    } else {
                        mode = "choose"
                        error = ""
                    }
                },
            ) {
                Text(if (mode == "choose") "Cancel" else "Back")
            }
        },
    )
}

private fun org.json.JSONArray?.toStringList(): List<String> {
    if (this == null) return emptyList()
    return List(length()) { index -> optString(index) }
}

@Composable
private fun QrCode(
    value: String,
    modifier: Modifier = Modifier,
    side: Dp = 180.dp,
) {
    val qr = remember(value) {
        runCatching { JSONObject(NativeCore.qrMatrixJson(value)) }.getOrElse { JSONObject() }
    }
    val width = qr.optInt("width")
    val cells = qr.optJSONArray("cells")
    Canvas(
        modifier = modifier
            .size(side)
            .clip(RoundedCornerShape(8.dp))
            .background(Color.White),
    ) {
        drawRect(Color.White)
        if (width <= 0 || cells == null) return@Canvas
        val quiet = 3
        val modules = width + quiet * 2
        val cell = size.minDimension / modules
        for (y in 0 until width) {
            for (x in 0 until width) {
                if (cells.optBoolean(y * width + x)) {
                    drawRect(
                        color = Color(0xFF111827),
                        topLeft = androidx.compose.ui.geometry.Offset((x + quiet) * cell, (y + quiet) * cell),
                        size = Size(cell, cell),
                    )
                }
            }
        }
    }
}
