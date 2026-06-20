package to.iris.drive.app

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Bundle
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import androidx.activity.ComponentActivity
import to.iris.drive.app.core.NativeCore

@Suppress("DEPRECATION", "OVERRIDE_DEPRECATION")
class IrisWebActivity : ComponentActivity() {
    private lateinit var webView: WebView

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val initialUrl = intent.getStringExtra(EXTRA_URL).orEmpty()
        webView = WebView(this).apply {
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.allowFileAccess = false
            settings.allowContentAccess = false
            settings.javaScriptCanOpenWindowsAutomatically = false
            settings.mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
            webViewClient = IrisWebViewClient()
        }
        setContentView(webView)
        if (initialUrl.isNotBlank()) {
            webView.loadUrl(initialUrl)
        }
    }

    override fun onBackPressed() {
        if (::webView.isInitialized && webView.canGoBack()) {
            webView.goBack()
        } else {
            super.onBackPressed()
        }
    }

    private inner class IrisWebViewClient : WebViewClient() {
        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean =
            handleNavigation(view, request.url)

        override fun shouldOverrideUrlLoading(view: WebView, url: String): Boolean =
            handleNavigation(view, Uri.parse(url))
    }

    private fun handleNavigation(view: WebView, uri: Uri): Boolean {
        val url = uri.toString()
        val classification = NativeCore.classifyLinkInput(url)
        return when (classification.optString("kind")) {
            "iris_web" -> {
                val localUrl = classification.optString("local_open_url").trim()
                if (localUrl.isNotBlank() && localUrl != url) {
                    view.loadUrl(localUrl)
                    true
                } else {
                    false
                }
            }

            "share_dialog", "nhash_file", "mutable_file", "invite", "app_key_approval" -> {
                startActivity(
                    Intent(this, MainActivity::class.java)
                        .setAction(Intent.ACTION_VIEW)
                        .setData(uri)
                        .addFlags(Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP),
                )
                true
            }

            else -> openExternally(uri)
        }
    }

    private fun openExternally(uri: Uri): Boolean {
        val scheme = uri.scheme?.lowercase()
        if (scheme != "http" && scheme != "https") {
            return true
        }
        runCatching {
            startActivity(Intent(Intent.ACTION_VIEW, uri))
        }
        return true
    }

    companion object {
        private const val EXTRA_URL = "to.iris.drive.app.IrisWebActivity.URL"

        fun createIntent(context: Context, url: String): Intent =
            Intent(context, IrisWebActivity::class.java).putExtra(EXTRA_URL, url)
    }
}
