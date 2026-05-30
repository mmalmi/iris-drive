package to.iris.drive.app

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import org.json.JSONObject
import to.iris.drive.app.core.DeviceLinkRequestState
import to.iris.drive.app.core.DeviceState
import to.iris.drive.app.core.NativeCore

@Composable
internal fun DevicesPanel(
    devices: List<DeviceState>,
    linkInvite: String,
    inboundRequests: List<DeviceLinkRequestState>,
    canApprove: Boolean,
    onCopyLinkInvite: () -> Unit,
    onApproveDevice: (String, String) -> Unit,
    onRevokeDevice: (String) -> Unit,
    onAppointAdmin: (String) -> Unit,
    onDemoteAdmin: (String) -> Unit,
) {
    var request by remember { mutableStateOf("") }
    var label by remember { mutableStateOf("") }
    var showAddDevice by remember { mutableStateOf(false) }

    CardSection(title = "Devices", trailing = "${devices.size}") {
        if (canApprove) {
            OutlinedButton(
                onClick = { showAddDevice = true },
                modifier = Modifier.testTag("addDeviceButton"),
            ) {
                Text("Add Device")
            }
        }
        devices.forEach { device ->
            Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                Icon(
                    painterResource(R.drawable.ic_drive),
                    contentDescription = null,
                    tint = if (device.isOnline) Teal else Muted,
                )
                Spacer(Modifier.size(12.dp))
                Column(Modifier.weight(1f)) {
                    Text(device.label, fontWeight = FontWeight.SemiBold)
                    Text(
                        "${device.role.ifBlank { "member" }} | ${device.state}",
                        color = Muted,
                        style = MaterialTheme.typography.bodySmall,
                    )
                    if (device.isCurrentDevice) {
                        Text(
                            "Device ID: ${device.pubkey}",
                            color = Muted,
                            style = MaterialTheme.typography.bodySmall,
                        )
                    }
                    Text(device.detail, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                if (device.canAppointAdmin) {
                    TextButton(onClick = { onAppointAdmin(device.pubkey) }) {
                        Text("Admin")
                    }
                }
                if (device.canDemoteAdmin) {
                    TextButton(onClick = { onDemoteAdmin(device.pubkey) }) {
                        Text("Member")
                    }
                }
                if (device.canRevoke) {
                    IconButton(onClick = { onRevokeDevice(device.pubkey) }) {
                        Icon(
                            painterResource(R.drawable.ic_delete),
                            contentDescription = "Revoke ${device.label}",
                            tint = Danger,
                        )
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
            label = label,
            onRequestChange = { request = it },
            onLabelChange = { label = it },
            onCopyLinkInvite = onCopyLinkInvite,
            onApproveDevice = onApproveDevice,
            onDismiss = { showAddDevice = false },
            onAdded = {
                request = ""
                label = ""
                showAddDevice = false
            },
        )
    }
}

@Composable
private fun AddDeviceDialog(
    linkInvite: String,
    inboundRequests: List<DeviceLinkRequestState>,
    canApprove: Boolean,
    request: String,
    label: String,
    onRequestChange: (String) -> Unit,
    onLabelChange: (String) -> Unit,
    onCopyLinkInvite: () -> Unit,
    onApproveDevice: (String, String) -> Unit,
    onDismiss: () -> Unit,
    onAdded: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add a device") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                if (linkInvite.isNotBlank()) {
                    Text("Invite device", fontWeight = FontWeight.SemiBold)
                    QrCode(linkInvite, side = 220.dp, modifier = Modifier.align(Alignment.CenterHorizontally))
                    Text(linkInvite, color = Muted, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    OutlinedButton(onClick = onCopyLinkInvite) {
                        Text("Copy invite link")
                    }
                }
                if (inboundRequests.isNotEmpty()) {
                    Text("Devices asking to join", fontWeight = FontWeight.SemiBold)
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
                            Button(
                                onClick = { onApproveDevice(inbound.requestLink, inbound.label) },
                                enabled = canApprove,
                            ) {
                                Text("Add")
                            }
                        }
                    }
                }
                Text(
                    "Paste the Device ID shown on the other device when you link it manually.",
                    color = Muted,
                )
                OutlinedTextField(
                    value = request,
                    onValueChange = onRequestChange,
                    modifier = Modifier.fillMaxWidth().testTag("manualDeviceId"),
                    singleLine = true,
                    label = { Text("Device ID") },
                )
                OutlinedTextField(
                    value = label,
                    onValueChange = onLabelChange,
                    modifier = Modifier.fillMaxWidth().testTag("manualDeviceName"),
                    singleLine = true,
                    label = { Text("Name (optional)") },
                )
            }
        },
        confirmButton = {
            Button(
                onClick = {
                    onApproveDevice(request, label)
                    onAdded()
                },
                enabled = canApprove && request.isNotBlank(),
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
