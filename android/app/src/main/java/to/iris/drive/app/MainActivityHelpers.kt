package to.iris.drive.app

import android.os.Build
import to.iris.drive.app.core.AppState
import to.iris.drive.app.core.NativeCore

internal fun normalizeProviderPath(path: String): String =
    NativeCore.normalizedProviderPath(path).orEmpty()

internal fun stateFromJson(json: String): AppState = AppState.fromJson(json)

internal fun resolveDeviceLabel(label: String): String =
    label.trim().ifBlank { defaultDeviceLabel() }

internal fun defaultDeviceLabel(): String {
    val model = Build.MODEL.orEmpty().trim()
    val manufacturer = Build.MANUFACTURER.orEmpty().trim()
    val label = when {
        model.isBlank() -> "Android"
        manufacturer.isBlank() -> model
        model.startsWith(manufacturer, ignoreCase = true) -> model
        model.contains("Pixel", ignoreCase = true) -> model
        else -> "$manufacturer $model"
    }
    return label.replace(Regex("\\s+"), " ").takeIf { it.isNotBlank() } ?: "Android"
}
