package to.iris.drive.app

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

internal data class BackupCheckProgress(
    val checked: Int = 0,
    val total: Int = 0,
    val activeTarget: String = "",
) {
    val isRunning: Boolean get() = total > 0
    val fraction: Float get() = if (total > 0) checked.toFloat() / total.toFloat() else 0f
    val label: String get() = if (total > 0) "Checking $checked of $total" else "Checked $checked"
}

@Composable
internal fun BackupProgressIndicator(progress: BackupCheckProgress) {
    LinearProgressIndicator(
        progress = { progress.fraction.coerceIn(0f, 1f) },
        modifier = Modifier.fillMaxWidth(),
    )
    Text(progress.label, color = Muted, style = MaterialTheme.typography.bodySmall)
}
