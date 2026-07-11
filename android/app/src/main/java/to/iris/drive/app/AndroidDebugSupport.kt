package to.iris.drive.app

import android.content.Context
import android.content.Intent
import android.system.Os
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.io.File
import java.net.InetSocketAddress
import java.net.Socket
import to.iris.drive.app.core.NativeCore

internal object AndroidDebugSupport {
    const val ACTION_EXTRA = "to.iris.drive.DEBUG_ACTION"
    const val OWNER_EXTRA = "to.iris.drive.DEBUG_OWNER"
    const val REQUEST_EXTRA = "to.iris.drive.DEBUG_REQUEST"
    const val RELAY_EXTRA = "to.iris.drive.DEBUG_RELAY"
    private const val STATE_FILE = "debug-state.json"
    private const val ENV_FILE = "debug-env.json"
    private const val PROVIDER_LIST_FILE = "debug-provider-list.json"
    private const val NETWORK_PROBE_FILE = "debug-network-probe.json"
    private const val NETWORK_HOST_EXTRA = "to.iris.drive.DEBUG_NETWORK_HOST"
    private const val NETWORK_PORT_EXTRA = "to.iris.drive.DEBUG_NETWORK_PORT"
    private const val ENV_EXTRA_PREFIX = "IRIS_DRIVE_"
    private const val NETWORK_PROBE_TIMEOUT_MS = 2_000

    fun applyEnvironment(context: Context, intent: Intent?) {
        if (!BuildConfig.DEBUG) return
        val extras = intent?.extras ?: return
        val applied = JSONObject()
        val errors = JSONObject()
        extras.keySet()
            .filter { it.startsWith(ENV_EXTRA_PREFIX) }
            .sorted()
            .forEach { key ->
                val value = extras.getString(key) ?: return@forEach
                runCatching {
                    Os.setenv(key, value, true)
                }.onSuccess {
                    applied.put(key, value)
                }.onFailure { error ->
                    errors.put(key, error.toString())
                }
            }
        runCatching {
            File(context.filesDir, ENV_FILE).writeText(
                JSONObject()
                    .put("applied", applied)
                    .put("errors", errors)
                    .toString(),
            )
        }
    }

    fun writeState(context: Context, jsonText: String?) {
        if (!BuildConfig.DEBUG) return
        if (jsonText.isNullOrBlank()) return
        runCatching {
            File(context.filesDir, STATE_FILE).writeText(jsonText)
        }
    }

    fun writeProviderList(context: Context) {
        if (!BuildConfig.DEBUG) return
        runCatching {
            File(context.filesDir, PROVIDER_LIST_FILE)
                .writeText(NativeCore.providerListJson(context.filesDir.absolutePath))
        }
    }

    fun writeNetworkProbe(context: Context, scope: CoroutineScope, intent: Intent?) {
        if (!BuildConfig.DEBUG) return
        val host = intent?.getStringExtra(NETWORK_HOST_EXTRA).orEmpty()
        val port = intent?.getStringExtra(NETWORK_PORT_EXTRA)?.toIntOrNull() ?: 0
        scope.launch(Dispatchers.IO) {
            val startedAt = System.currentTimeMillis()
            val result = JSONObject()
                .put("host", host)
                .put("port", port)
                .put("ok", false)
                .put("started_at_ms", startedAt)
            if (host.isBlank() || port !in 1..65535) {
                result.put("error", "host and port are required")
            } else {
                runCatching {
                    Socket().use { socket ->
                        socket.connect(
                            InetSocketAddress(host, port),
                            NETWORK_PROBE_TIMEOUT_MS,
                        )
                    }
                }.onSuccess {
                    result.put("ok", true)
                }.onFailure { error ->
                    result.put("error", error.toString())
                }
            }
            result.put("finished_at_ms", System.currentTimeMillis())
            runCatching {
                File(context.filesDir, NETWORK_PROBE_FILE).writeText(result.toString())
            }
        }
    }
}
