package to.iris.drive.app

import androidx.compose.foundation.background
import androidx.compose.foundation.Image
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import kotlinx.coroutines.flow.StateFlow
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.SyncRoot

private val Background = Color(0xFFF7FAF8)
private val Ink = Color(0xFF172321)
private val Muted = Color(0xFF657370)
private val Teal = Color(0xFF167C80)
private val SoftTeal = Color(0xFFE7F4F0)
private val Amber = Color(0xFFF5A524)
private val Danger = Color(0xFFB42318)

@Composable
internal fun IrisDriveAndroidApp(
    stateFlow: StateFlow<AppState>,
    onRefresh: () -> Unit,
    onAddRoot: (String, String) -> Unit,
    onRemoveRoot: (String) -> Unit,
    onStartSync: () -> Unit,
    onStopSync: () -> Unit,
) {
    val state by stateFlow.collectAsStateWithLifecycle()
    IrisDriveTheme {
        var addRootOpen by remember { mutableStateOf(false) }
        Scaffold(
            containerColor = Background,
            topBar = {
                AppTopBar(
                    onRefresh = onRefresh,
                    onAddRoot = { addRootOpen = true },
                )
            },
        ) { padding ->
            LazyColumn(
                modifier = Modifier
                    .fillMaxSize()
                    .padding(padding),
                contentPadding = PaddingValues(18.dp),
                verticalArrangement = Arrangement.spacedBy(12.dp),
            ) {
                if (state.error.isNotBlank()) {
                    item { Notice(state.error) }
                }
                item {
                    SyncPanel(onStartSync = onStartSync, onStopSync = onStopSync)
                }
                item {
                    ProviderPanel()
                }
                item {
                    SectionHeader("Roots", "${state.roots.size}")
                }
                if (state.roots.isEmpty()) {
                    item { EmptyRoots(onAddRoot = { addRootOpen = true }) }
                } else {
                    items(state.roots, key = { it.name }) { root ->
                        RootRow(root = root, onRemoveRoot = onRemoveRoot)
                    }
                }
            }
        }
        if (addRootOpen) {
            AddRootDialog(
                onDismiss = { addRootOpen = false },
                onAdd = { name, path ->
                    addRootOpen = false
                    onAddRoot(name, path)
                },
            )
        }
    }
}

@Composable
private fun IrisDriveTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = lightColorScheme(
            primary = Teal,
            secondary = Amber,
            background = Background,
            surface = Color.White,
            error = Danger,
            onPrimary = Color.White,
            onSecondary = Ink,
            onBackground = Ink,
            onSurface = Ink,
        ),
        content = content,
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AppTopBar(onRefresh: () -> Unit, onAddRoot: () -> Unit) {
    TopAppBar(
        title = {
            Column {
                Text("Iris Drive", fontWeight = FontWeight.SemiBold)
                Text("Android", color = Muted, style = MaterialTheme.typography.labelMedium)
            }
        },
        actions = {
            IconButton(onClick = onRefresh) {
                Icon(painterResource(R.drawable.ic_refresh), contentDescription = "Refresh")
            }
            FilledIconButton(onClick = onAddRoot) {
                Icon(painterResource(R.drawable.ic_add), contentDescription = "Add root")
            }
        },
        colors = TopAppBarDefaults.topAppBarColors(
            containerColor = Color.White,
            titleContentColor = Ink,
            actionIconContentColor = Teal,
        ),
    )
}

@Composable
private fun SyncPanel(onStartSync: () -> Unit, onStopSync: () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        SectionHeader("Sync", "service")
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            Button(onClick = onStartSync) {
                Icon(painterResource(R.drawable.ic_play), contentDescription = null)
                Spacer(Modifier.size(8.dp))
                Text("Start")
            }
            OutlinedButton(onClick = onStopSync) {
                Icon(painterResource(R.drawable.ic_stop), contentDescription = null)
                Spacer(Modifier.size(8.dp))
                Text("Stop")
            }
        }
    }
}

@Composable
private fun ProviderPanel() {
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        SectionHeader("Files", "DocumentsProvider")
        Box(
            modifier = Modifier
                .fillMaxWidth()
                .background(SoftTeal, RoundedCornerShape(8.dp))
                .padding(14.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(painterResource(R.drawable.ic_drive), contentDescription = null, tint = Teal)
                Spacer(Modifier.size(12.dp))
                Column {
                    Text("Iris Drive", fontWeight = FontWeight.SemiBold)
                    Text(
                        "to.iris.drive.documents",
                        color = Muted,
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
            }
        }
    }
}

@Composable
private fun RootRow(root: SyncRoot, onRemoveRoot: (String) -> Unit) {
    Card(
        shape = RoundedCornerShape(8.dp),
        colors = CardDefaults.cardColors(containerColor = Color.White),
    ) {
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .padding(14.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(painterResource(R.drawable.ic_drive), contentDescription = null, tint = Teal)
            Spacer(Modifier.size(12.dp))
            Column(Modifier.weight(1f)) {
                Text(
                    root.name,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Text(root.status, color = Muted, style = MaterialTheme.typography.bodySmall)
                Text(
                    root.localPath,
                    color = Muted,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    style = MaterialTheme.typography.bodySmall,
                )
            }
            IconButton(onClick = { onRemoveRoot(root.name) }) {
                Icon(
                    painterResource(R.drawable.ic_delete),
                    contentDescription = "Remove ${root.name}",
                    tint = Danger,
                )
            }
        }
    }
}

@Composable
private fun EmptyRoots(onAddRoot: () -> Unit) {
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 18.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Image(
            painter = painterResource(id = R.drawable.brand_icon),
            contentDescription = "Iris Drive",
            modifier = Modifier.size(96.dp),
        )
        Text("No roots", color = Muted)
        OutlinedButton(onClick = onAddRoot) {
            Icon(painterResource(R.drawable.ic_add), contentDescription = null)
            Spacer(Modifier.size(8.dp))
            Text("Add")
        }
    }
}

@Composable
private fun Notice(text: String) {
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .background(Color(0xFFFEE4E2), RoundedCornerShape(8.dp))
            .padding(12.dp),
    ) {
        Text(text, color = Danger)
    }
}

@Composable
private fun SectionHeader(title: String, trailing: String) {
    Row(
        modifier = Modifier.fillMaxWidth(),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(title, fontWeight = FontWeight.SemiBold)
        Text(trailing, color = Muted, style = MaterialTheme.typography.labelMedium)
    }
}

@Composable
private fun AddRootDialog(
    onDismiss: () -> Unit,
    onAdd: (String, String) -> Unit,
) {
    var name by remember { mutableStateOf("My Drive") }
    var path by remember { mutableStateOf("content://to.iris.drive.documents/root") }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Add Root") },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                OutlinedTextField(
                    value = name,
                    onValueChange = { name = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    label = { Text("Name") },
                )
                OutlinedTextField(
                    value = path,
                    onValueChange = { path = it },
                    modifier = Modifier.fillMaxWidth(),
                    singleLine = true,
                    label = { Text("Path") },
                )
            }
        },
        confirmButton = {
            TextButton(onClick = { onAdd(name, path) }) {
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
