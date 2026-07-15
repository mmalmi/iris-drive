package to.iris.drive.app

import java.net.URLEncoder

internal fun irisDriveShareDialogLink(sourcePath: String, displayName: String = ""): String? {
    val path = sourcePath.trim()
    if (path.isBlank()) return null

    val link = StringBuilder("iris-drive://share?path=")
        .append(urlEncodeQueryValue(path))
    val name = displayName.trim()
    if (name.isNotBlank()) {
        link.append("&name=").append(urlEncodeQueryValue(name))
    }
    return link.toString()
}

private fun urlEncodeQueryValue(value: String): String =
    URLEncoder.encode(value, Charsets.UTF_8.name())
