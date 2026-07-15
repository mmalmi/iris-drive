package to.iris.drive.app

import android.content.Context
import android.content.Intent
import android.graphics.Color
import android.graphics.Typeface
import android.graphics.drawable.GradientDrawable
import android.view.Gravity
import android.view.inputmethod.EditorInfo
import android.net.Uri
import android.os.Bundle
import android.text.InputType
import android.view.ViewGroup
import android.webkit.WebResourceRequest
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.EditText
import android.widget.LinearLayout
import android.widget.TextView
import androidx.activity.ComponentActivity
import to.iris.drive.app.core.NativeCore

@Suppress("DEPRECATION", "OVERRIDE_DEPRECATION")
class IrisWebActivity : ComponentActivity() {
    private lateinit var webView: WebView
    private lateinit var addressField: EditText
    private lateinit var backButton: TextView
    private lateinit var shareButton: TextView
    private var activePortalUrl: String = ""

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        activePortalUrl = intent.getStringExtra(EXTRA_PORTAL_URL).orEmpty()
        val initialUrl = localGatewayUrl(intent.getStringExtra(EXTRA_URL).orEmpty())
        webView = WebView(this).apply {
            settings.javaScriptEnabled = true
            settings.domStorageEnabled = true
            settings.allowFileAccess = false
            settings.allowContentAccess = false
            settings.javaScriptCanOpenWindowsAutomatically = false
            settings.mixedContentMode = WebSettings.MIXED_CONTENT_NEVER_ALLOW
            webViewClient = IrisWebViewClient()
        }
        setContentView(browserLayout())
        if (initialUrl.isNotBlank()) {
            webView.loadUrl(initialUrl)
        }
        updateBrowserChrome(initialUrl)
    }

    override fun onBackPressed() {
        if (::webView.isInitialized && webView.canGoBack()) {
            webView.goBack()
        } else {
            super.onBackPressed()
        }
    }

    private fun browserLayout(): LinearLayout {
        addressField = EditText(this).apply {
            setSingleLine(true)
            setSelectAllOnFocus(true)
            inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_URI
            imeOptions = EditorInfo.IME_ACTION_GO
            setTextColor(Color.WHITE)
            setHintTextColor(Color.rgb(170, 180, 185))
            textSize = 14f
            background = roundedBackground(Color.rgb(38, 45, 48), dp(8).toFloat())
            setPadding(dp(12), 0, dp(12), 0)
            setOnEditorActionListener { _, actionId, _ ->
                if (actionId == EditorInfo.IME_ACTION_GO) {
                    loadAddressBarUrl()
                    true
                } else {
                    false
                }
            }
        }
        backButton = browserIconButton("‹", "Back") {
            if (webView.canGoBack()) {
                webView.goBack()
            }
        }
        shareButton = browserIconButton("↗", "Share") {
            shareCurrentUrl()
        }

        val closeButton = browserIconButton("×", "Close") {
            finish()
        }
        val bar = LinearLayout(this).apply {
            orientation = LinearLayout.HORIZONTAL
            gravity = Gravity.CENTER_VERTICAL
            setPadding(dp(10), dp(8), dp(10), dp(10))
            setBackgroundColor(Color.rgb(20, 24, 26))
            addView(closeButton, iconLayoutParams())
            addView(backButton, iconLayoutParams())
            addView(
                addressField,
                LinearLayout.LayoutParams(0, dp(42), 1f).apply {
                    marginStart = dp(8)
                    marginEnd = dp(8)
                },
            )
            addView(shareButton, iconLayoutParams())
        }

        return LinearLayout(this).apply {
            orientation = LinearLayout.VERTICAL
            setBackgroundColor(Color.BLACK)
            addView(
                webView,
                LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    0,
                    1f,
                ),
            )
            addView(
                bar,
                LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT,
                    ViewGroup.LayoutParams.WRAP_CONTENT,
                ),
            )
        }
    }

    private fun browserIconButton(
        symbol: String,
        description: String,
        onClick: () -> Unit,
    ): TextView =
        TextView(this).apply {
            text = symbol
            contentDescription = description
            gravity = Gravity.CENTER
            textSize = 24f
            typeface = Typeface.DEFAULT_BOLD
            setTextColor(Color.WHITE)
            background = roundedBackground(Color.rgb(38, 45, 48), dp(10).toFloat())
            setOnClickListener { onClick() }
        }

    private fun iconLayoutParams(): LinearLayout.LayoutParams =
        LinearLayout.LayoutParams(dp(42), dp(42)).apply {
            marginEnd = dp(6)
        }

    private fun roundedBackground(color: Int, radius: Float): GradientDrawable =
        GradientDrawable().apply {
            setColor(color)
            cornerRadius = radius
        }

    private fun loadAddressBarUrl() {
        val candidate = browserAddressUrl(addressField.text?.toString().orEmpty())
        if (candidate.isBlank()) return
        addressField.clearFocus()
        webView.loadUrl(candidate)
    }

    private fun browserAddressUrl(value: String): String {
        var candidate = value.trim()
        if (candidate.isBlank()) return value
        if (Uri.parse(candidate).scheme.isNullOrBlank() && candidate.contains(".")) {
            candidate = "https://$candidate"
        }
        val classification = NativeCore.classifyLinkInput(candidate)
        val localUrl = classification.optString("local_open_url").trim()
        return when (classification.optString("kind")) {
            "iris_web", "nhash_file", "mutable_file" ->
                localGatewayUrl(localUrl.ifBlank { candidate })

            else -> localGatewayUrl(candidate)
        }
    }

    private fun updateBrowserChrome(url: String? = webView.url) {
        if (::backButton.isInitialized) {
            backButton.isEnabled = webView.canGoBack()
            backButton.alpha = if (webView.canGoBack()) 1f else 0.45f
        }
        if (::shareButton.isInitialized) {
            shareButton.isEnabled = !url.isNullOrBlank()
            shareButton.alpha = if (url.isNullOrBlank()) 0.45f else 1f
        }
        if (::addressField.isInitialized && !addressField.hasFocus()) {
            addressField.setText(url.orEmpty())
        }
    }

    private fun shareCurrentUrl() {
        val url = webView.url?.trim().orEmpty()
        if (url.isBlank()) return
        startActivity(
            Intent.createChooser(
                Intent(Intent.ACTION_SEND)
                    .setType("text/plain")
                    .putExtra(Intent.EXTRA_TEXT, url),
                null,
            ),
        )
    }

    private inner class IrisWebViewClient : WebViewClient() {
        override fun shouldOverrideUrlLoading(view: WebView, request: WebResourceRequest): Boolean =
            handleNavigation(view, request.url)

        override fun shouldOverrideUrlLoading(view: WebView, url: String): Boolean =
            handleNavigation(view, Uri.parse(url))

        override fun onPageFinished(view: WebView, url: String) {
            updateBrowserChrome(url)
        }

        override fun doUpdateVisitedHistory(view: WebView, url: String?, isReload: Boolean) {
            updateBrowserChrome(url)
        }
    }

    private fun handleNavigation(view: WebView, uri: Uri): Boolean {
        val url = uri.toString()
        val classification = NativeCore.classifyLinkInput(url)
        return when (classification.optString("kind")) {
            "iris_web" -> {
                val localUrl = localGatewayUrl(classification.optString("local_open_url").trim())
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

    private fun localGatewayUrl(value: String): String {
        val uri = Uri.parse(value.trim())
        val host = uri.host?.lowercase() ?: return value
        val activePort = Uri.parse(activePortalUrl).port
        if (activePort <= 0) return value
        val isLocalGatewayHost = host == "iris.localhost" ||
            host.endsWith(".iris.localhost") ||
            host == "hash.localhost" ||
            host.endsWith(".hash.localhost")
        if (!isLocalGatewayHost) return value
        return uri.buildUpon()
            .encodedAuthority("$host:$activePort")
            .build()
            .toString()
    }

    private fun dp(value: Int): Int =
        (value * resources.displayMetrics.density).toInt()

    companion object {
        private const val EXTRA_URL = "to.iris.drive.app.IrisWebActivity.URL"
        private const val EXTRA_PORTAL_URL = "to.iris.drive.app.IrisWebActivity.PORTAL_URL"

        fun createIntent(context: Context, url: String, portalUrl: String = ""): Intent =
            Intent(context, IrisWebActivity::class.java)
                .putExtra(EXTRA_URL, url)
                .putExtra(EXTRA_PORTAL_URL, portalUrl)
    }
}
