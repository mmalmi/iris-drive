package to.iris.drive.app

import android.net.Uri
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Something handed to the share sheet: a file to copy, or text to keep as a note. */
internal data class SharedItem(
    val name: String,
    val mimeType: String,
    val uri: Uri? = null,
    val text: String? = null,
)

/**
 * "Save to Iris Drive" from any app's share sheet. Like Google Drive's, it
 * shows what will be saved and where, and saves only when asked to.
 * `folders` is null when Iris Drive has no profile on this device.
 */
@Composable
internal fun ShareSaveScreen(
    items: List<SharedItem>,
    folders: List<String>?,
    onSave: suspend (List<SharedItem>, String) -> Unit,
    onOpenApp: () -> Unit,
    onClose: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    var name by remember(items) { mutableStateOf(items.singleOrNull()?.name.orEmpty()) }
    var folder by remember { mutableStateOf("") }
    var choosingFolder by remember { mutableStateOf(false) }
    var saving by remember { mutableStateOf(false) }
    var result by remember { mutableStateOf<String?>(null) }
    var failure by remember { mutableStateOf<String?>(null) }
    val destination = if (folder.isBlank()) "My Drive" else folder.substringAfterLast('/')

    Surface(Modifier.fillMaxSize().safeDrawingPadding(), color = MaterialTheme.colorScheme.background) {
        Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(14.dp)) {
            Text("Save to Iris Drive", style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
            when {
                folders == null -> {
                    Text("Open Iris Drive and set it up on this device first.", Modifier.testTag("shareSaveNotSetUp"))
                    Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                        Button(onClick = onOpenApp) { Text("Open Iris Drive") }
                        TextButton(onClick = onClose) { Text("Close") }
                    }
                }
                items.isEmpty() -> {
                    Text("Nothing here can be saved to Iris Drive.", Modifier.testTag("shareSaveError"))
                    TextButton(onClick = onClose) { Text("Close") }
                }
                result != null -> Text(result.orEmpty(), Modifier.testTag("shareSaveResult"), color = Ink)
                else -> {
                    if (items.size == 1) {
                        OutlinedTextField(
                            value = name,
                            onValueChange = { name = it },
                            label = { Text("Name") },
                            singleLine = true,
                            enabled = !saving,
                            modifier = Modifier.fillMaxWidth().testTag("shareSaveName"),
                        )
                    } else {
                        Text("${items.size} items", fontWeight = FontWeight.SemiBold)
                        items.take(5).forEach { Text(it.name, color = Muted, maxLines = 1, overflow = TextOverflow.Ellipsis) }
                        if (items.size > 5) Text("and ${items.size - 5} more", color = Muted)
                    }
                    Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.fillMaxWidth()) {
                        Column(Modifier.weight(1f)) {
                            Text("Save in", color = Muted, style = MaterialTheme.typography.bodySmall)
                            Text(
                                if (folder.isBlank()) "My Drive" else "My Drive/$folder",
                                modifier = Modifier.testTag("shareSaveFolderName"),
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        Box {
                            OutlinedButton(
                                onClick = { choosingFolder = true },
                                enabled = !saving && folders.isNotEmpty(),
                                modifier = Modifier.testTag("shareSaveFolder"),
                            ) { Text("Change") }
                            DropdownMenu(expanded = choosingFolder, onDismissRequest = { choosingFolder = false }) {
                                (listOf("") + folders.take(200)).forEach { option ->
                                    DropdownMenuItem(
                                        text = { Text(if (option.isBlank()) "My Drive" else option) },
                                        onClick = {
                                            folder = option
                                            choosingFolder = false
                                        },
                                    )
                                }
                            }
                        }
                    }
                    failure?.let { Text(it, color = Danger, modifier = Modifier.testTag("shareSaveError")) }
                    Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
                        Button(
                            onClick = {
                                saving = true
                                failure = null
                                val toSave = if (items.size == 1) listOf(items[0].copy(name = name.trim())) else items
                                scope.launch {
                                    try {
                                        withContext(Dispatchers.IO) { onSave(toSave, folder) }
                                        result = if (toSave.size == 1) {
                                            "Saved to $destination"
                                        } else {
                                            "Saved ${toSave.size} items to $destination"
                                        }
                                        delay(900)
                                        onClose()
                                    } catch (error: Exception) {
                                        failure = error.message?.takeIf { it.isNotBlank() } ?: "Could not save to Iris Drive"
                                        saving = false
                                    }
                                }
                            },
                            enabled = !saving && (items.size > 1 || name.isNotBlank()),
                            modifier = Modifier.testTag("shareSaveButton"),
                        ) { Text("Save") }
                        TextButton(onClick = onClose, enabled = !saving) { Text("Cancel") }
                        if (saving) CircularProgressIndicator(Modifier.padding(start = 4.dp))
                    }
                }
            }
        }
    }
}
