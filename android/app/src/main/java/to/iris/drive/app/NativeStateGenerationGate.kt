package to.iris.drive.app

import android.util.Log
import to.iris.drive.app.core.AppState

internal class NativeStateGenerationGate {
    private var latestRequested = 0L

    fun request(): Long {
        latestRequested += 1
        return latestRequested
    }

    fun shouldApply(generation: Long): Boolean = generation == latestRequested
}

internal fun logNativeStateTransition(
    phase: String,
    generation: Long,
    origin: String,
    state: AppState?,
) {
    val labels = state?.devices?.joinToString(",") { it.label }.orEmpty()
    Log.i(
        "IrisDriveState",
        "phase=$phase generation=$generation origin=$origin " +
            "devices=${state?.devices?.size ?: -1} labels=[$labels] " +
            "error=${nativeErrorCategory(state?.error.orEmpty())}",
    )
}

internal fun nativeErrorCategory(error: String): String =
    when {
        error.isBlank() -> "none"
        error.startsWith("locking config mutation:") -> "config_lock"
        error.startsWith("profile admin is required") -> "authorization"
        error.contains("approval bootstrap") || error.contains("approval request") -> "invalid_request"
        error.startsWith("approving device:") -> "approval_apply"
        error.startsWith("saving config:") -> "config_save"
        else -> "other"
    }
