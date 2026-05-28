package to.iris.drive.app.core

import org.json.JSONArray
import org.json.JSONObject

internal data class AppState(
    val roots: List<SyncRoot> = emptyList(),
    val error: String = "",
) {
    companion object {
        fun fromJson(jsonText: String): AppState {
            val json = runCatching { JSONObject(jsonText) }.getOrElse {
                return AppState(error = it.message ?: "invalid native state JSON")
            }
            val ui = json.optJSONObject("ui") ?: JSONObject()
            return AppState(
                roots = ui.optJSONArray("roots").toRoots(),
                error = json.optString("error"),
            )
        }
    }
}

internal data class SyncRoot(
    val name: String,
    val localPath: String,
    val status: String,
)

internal object NativeActions {
    fun refresh(): String = JSONObject().put("type", "refresh").toString()

    fun addRoot(name: String, localPath: String): String =
        JSONObject()
            .put("type", "add_root")
            .put("name", name)
            .put("local_path", localPath)
            .toString()

    fun removeRoot(name: String): String =
        JSONObject()
            .put("type", "remove_root")
            .put("name", name)
            .toString()
}

private fun JSONArray?.toRoots(): List<SyncRoot> {
    if (this == null) return emptyList()
    return buildList {
        for (index in 0 until length()) {
            val item = optJSONObject(index) ?: continue
            add(
                SyncRoot(
                    name = item.optString("name"),
                    localPath = item.optString("local_path"),
                    status = item.optString("status"),
                ),
            )
        }
    }
}
