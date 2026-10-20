package to.iris.drive.app

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.widget.Toast
import to.iris.drive.app.core.NativeCore

internal class DeviceApprovalLaunchController(
    private val activity: Activity,
    private val nativeReady: () -> Boolean,
    private val approveDevice: (String, String) -> Unit,
) {
    var confirmationPending: Boolean = false
        private set

    private var preNativePromptRequest: String? = null
    private var pendingApproval: Pair<String, String>? = null

    fun promptBeforeNativeStart(intent: Intent?) {
        val request = intent?.data?.toString().orEmpty()
        if (request.isEmpty() || NativeCore.classifyLinkInput(request).optString("kind") != "app_key_approval") {
            return
        }
        preNativePromptRequest = request
        showConfirmation(request)
    }

    fun handleApprovalRequest(request: String) {
        if (preNativePromptRequest == request) {
            preNativePromptRequest = null
            return
        }
        showConfirmation(request)
    }

    fun drainPendingApproval() {
        if (!nativeReady()) return
        val (request, label) = pendingApproval ?: return
        pendingApproval = null
        approveDevice(request, label)
    }

    fun markApprovalComplete() {
        confirmationPending = false
    }

    private fun showConfirmation(request: String) {
        if (!NativeCore.isCompleteDeviceApprovalInput(request)) {
            Toast.makeText(activity, "Invalid device request", Toast.LENGTH_SHORT).show()
            return
        }
        confirmationPending = true
        AlertDialog.Builder(activity)
            .setTitle("Approve this device?")
            .setMessage("This will add the joining device to Iris Drive.")
            .setPositiveButton("Approve") { _, _ ->
                if (nativeReady()) {
                    approveDevice(request, "")
                } else {
                    pendingApproval = request to ""
                }
            }
            .setNegativeButton("Cancel") { _, _ ->
                pendingApproval = null
                confirmationPending = false
            }
            .show()
    }
}
