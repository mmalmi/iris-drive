package to.iris.drive.app

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.widget.Toast
import org.json.JSONObject
import to.iris.drive.app.core.NativeCore
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.*
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation

@Composable
internal fun RestoreCredentialForm(
    signer: Boolean,
    onBack: () -> Unit,
    onRestore: (String) -> Unit,
    onSignerInteraction: (Boolean) -> Unit,
) {
    var input by remember(signer) { mutableStateOf("") }
    var submitted by remember(signer) { mutableStateOf(false) }
    val canSubmit = input.isNotBlank() && (!signer || input.trim().startsWith("bunker://"))
    fun submit() {
        if (canSubmit) {
            submitted = true
            onRestore(input.trim())
        }
    }
    SetupFormHeader(title = if (signer) "Remote signer" else "Secret key", onBack = {
        if (signer && submitted) onSignerInteraction(true)
        onBack()
    })
    OutlinedTextField(
        value = input, onValueChange = { input = it },
        modifier = Modifier.fillMaxWidth().testTag(if (signer) "remoteSignerInput" else "restoreSecretKeyInput"),
        singleLine = true,
        label = { Text(if (signer) "Signer connection link (bunker://…)" else "Secret key") },
        visualTransformation = PasswordVisualTransformation(),
        keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password, imeAction = ImeAction.Done),
        keyboardActions = KeyboardActions(onDone = { submit() }),
    )
    SetupPrimaryButton(
        text = if (signer) "Connect signer" else "Restore", onClick = ::submit, enabled = canSubmit,
        testTag = if (signer) "remoteSignerSubmit" else "restoreSecretKeySubmit",
    )
    if (signer && submitted) {
        SetupSecondaryButton("Open signer", onClick = { onSignerInteraction(false) })
    }
}

internal fun signerInteraction(context: Context, handle: Long, cancel: Boolean) {
    if (handle == 0L) return
    val method = if (cancel) "cancel_signer_recovery" else "signer_approval_link"
    val result = JSONObject(NativeCore.dispatchJson(handle, JSONObject().put("type", method).toString()))
    if (cancel) return
    val link = result.optString("signer_approval_url")
    if (link.startsWith("https://")) {
        runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(link))) }
            .onFailure { Toast.makeText(context, "Could not open signer", Toast.LENGTH_SHORT).show() }
    } else {
        Toast.makeText(context, "Check your signer for an approval request", Toast.LENGTH_SHORT).show()
    }
}
