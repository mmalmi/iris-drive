package to.iris.drive.app.photos

import android.content.Context
import android.net.ConnectivityManager
import android.net.NetworkCapabilities

internal enum class PhotoNetwork {
    Offline, Metered, Unmetered;

    fun allowsUpload(allowMetered: Boolean) = this == Unmetered || (this == Metered && allowMetered)

    companion object {
        fun current(context: Context): PhotoNetwork {
            val network = context.getSystemService(ConnectivityManager::class.java) ?: return Offline
            val caps = network.getNetworkCapabilities(network.activeNetwork) ?: return Offline
            return when {
                !caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET) -> Offline
                caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED) -> Unmetered
                else -> Metered
            }
        }
    }
}

internal fun publicationStatus(failed: Int, published: Boolean, pending: Boolean, network: PhotoNetwork, allowMetered: Boolean): String {
    val status = if (published) null else if (pending && network == PhotoNetwork.Metered && !allowMetered) {
        "Saved in My Drive · waiting for Wi-Fi"
    } else "Saved in My Drive · waiting to sync"
    if (failed == 0) return status ?: "Up to date"
    return listOfNotNull("Could not copy $failed items. Will retry.", status).joinToString(" ")
}
