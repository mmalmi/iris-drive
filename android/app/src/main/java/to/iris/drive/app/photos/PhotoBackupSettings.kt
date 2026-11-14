package to.iris.drive.app.photos

import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.*
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import to.iris.drive.app.CardSection

@Composable
internal fun PhotoBackupSettings(profileId: String) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var access by remember { mutableStateOf(PhotoAccess.get(context)) }
    var status by remember(profileId) { mutableStateOf<String?>(null) }
    var allowMetered by remember(profileId) { mutableStateOf(false) }
    var saved by remember(profileId) { mutableLongStateOf(0L) }
    var loaded by remember(profileId) { mutableStateOf(false) }
    var notice by rememberSaveable(profileId) { mutableStateOf("") }
    var permissionProfile by rememberSaveable { mutableStateOf("") }

    LifecycleResumeEffect(profileId) {
        access = PhotoAccess.get(context)
        onPauseOrDispose {}
    }
    LaunchedEffect(profileId) {
        PhotoBackupJobs.changes.collect {
            val summary = withContext(Dispatchers.IO) {
                PhotoBackupJournal(context).use { Triple(it.status(profileId), it.count(profileId), it.allowMetered(profileId)) }
            }
            status = summary.first
            saved = summary.second
            allowMetered = summary.third
            loaded = true
        }
    }

    fun backUp(enable: Boolean) {
        scope.launch {
            try {
                withContext(Dispatchers.IO) {
                    if (enable) PhotoBackupJournal(context).use { it.enable(profileId) }
                    PhotoBackupJobs.schedule(context)
                }
                notice = ""
            } catch (_: IllegalStateException) {
                notice = "Android could not schedule backup. Try again."
            } finally {
                PhotoBackupJobs.notifyChange()
            }
        }
    }

    val request = rememberLauncherForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
        access = PhotoAccess.get(context)
        if (permissionProfile == profileId) {
            if (access.granted) backUp(enable = status == null) else notice = "Allow photo or video access in app permissions to back up."
        }
        permissionProfile = ""
    }
    fun requestAccess() {
        permissionProfile = profileId
        request.launch(PhotoAccess.permissions())
    }

    CardSection(title = "Photo backup", trailing = "") {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            Text("Back up photos and videos", modifier = Modifier.weight(1f))
            Switch(
                checked = status != null,
                enabled = loaded,
                onCheckedChange = { enabled ->
                    notice = ""
                    if (!enabled) {
                        PhotoBackupJobs.disable(context)
                    } else if (PhotoAccess.get(context).granted) {
                        backUp(enable = true)
                    } else {
                        requestAccess()
                    }
                },
                modifier = Modifier.testTag("photoBackupToggle"),
            )
        }
        Text("Copy this device’s photos and videos to My Drive/Photos. Removing an item from your device keeps its backup.",
            style = MaterialTheme.typography.bodySmall)
        if (status != null) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
                Text("Sync over mobile or metered data", modifier = Modifier.weight(1f))
                Switch(
                    checked = allowMetered,
                    onCheckedChange = { allowed ->
                        PhotoBackupJournal(context).use { it.setAllowMetered(profileId, allowed) }
                        backUp(enable = false)
                    },
                    modifier = Modifier.testTag("photoBackupMeteredToggle"),
                )
            }
            val selection = when {
                !access.granted -> "Photo access required"
                access.images && access.videos -> "All photos and videos"
                access.selected -> "Allowed photos and videos"
                access.images -> "Photos only"
                else -> "Videos only"
            }
            Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(selection, style = MaterialTheme.typography.bodySmall)
                Text("$status · $saved backed up", style = MaterialTheme.typography.bodySmall)
            }
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                if (access.granted) OutlinedButton(onClick = { backUp(enable = false) }) { Text("Back up now") }
                if (Build.VERSION.SDK_INT >= 34 && !(access.images && access.videos)) {
                    TextButton(onClick = ::requestAccess) { Text("Choose media") }
                }
            }
        }
        if (notice.isNotEmpty()) Text(notice, style = MaterialTheme.typography.bodySmall)
        if ((status != null && !(access.images && access.videos)) || notice.isNotEmpty()) {
            TextButton(onClick = {
                context.startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${context.packageName}")))
            }) { Text("App permissions") }
        }
    }
}
