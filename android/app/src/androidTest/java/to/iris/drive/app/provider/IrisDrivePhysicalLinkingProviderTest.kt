package to.iris.drive.app.provider

import android.content.Context
import android.net.Uri
import android.os.Bundle
import android.provider.DocumentsContract
import android.provider.DocumentsContract.Document
import android.util.Base64
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import java.security.MessageDigest
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertNotNull
import org.junit.Test
import org.junit.runner.RunWith
import to.iris.drive.app.BuildConfig

/** Physical-gate client of the shipped DocumentsProvider; it never creates a profile. */
@RunWith(AndroidJUnit4::class)
class IrisDrivePhysicalLinkingProviderTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val context: Context = instrumentation.targetContext
    private val arguments: Bundle = InstrumentationRegistry.getArguments()

    @Test
    fun writePostLinkFileThroughDocumentsProvider() {
        val name = requiredArgument("file_name")
        val expected = expectedContent()
        val root = DocumentsContract.buildDocumentUri(
            BuildConfig.DOCUMENTS_PROVIDER_AUTHORITY,
            IrisDriveDocumentStore.ROOT_DOCUMENT_ID,
        )
        val created = DocumentsContract.createDocument(
            context.contentResolver,
            root,
            "text/plain",
            name,
        )
        assertNotNull("DocumentsContract.createDocument returned null for $name", created)
        context.contentResolver.openOutputStream(created!!, "wt").use { stream ->
            assertNotNull("DocumentsProvider returned no output stream for $name", stream)
            stream!!.write(expected)
        }
        instrumentation.waitForIdleSync()
        assertArrayEquals(expected, readDocument(created))
        evidence("IRIS_ANDROID_PROVIDER_WRITE_SHA256=${sha256(expected)}")
    }

    @Test
    fun readPostLinkFileThroughDocumentsProvider() {
        val name = requiredArgument("file_name")
        val expected = expectedContent()
        val deadline = System.currentTimeMillis() + arguments.getString("wait_millis", "45000").toLong()
        var last: ByteArray? = null
        while (System.currentTimeMillis() < deadline) {
            runCatching { findDocument(name)?.let(::readDocument) }.getOrNull()?.let { content ->
                last = content
                if (content.contentEquals(expected)) {
                    evidence("IRIS_ANDROID_PROVIDER_READ_SHA256=${sha256(content)}")
                    return
                }
            }
            Thread.sleep(200)
        }
        assertArrayEquals(
            "DocumentsProvider content mismatch for $name",
            expected,
            last ?: byteArrayOf(),
        )
    }

    private fun findDocument(name: String): Uri? {
        val authority = BuildConfig.DOCUMENTS_PROVIDER_AUTHORITY
        val children = DocumentsContract.buildChildDocumentsUri(
            authority,
            IrisDriveDocumentStore.ROOT_DOCUMENT_ID,
        )
        context.contentResolver.query(
            children,
            arrayOf(Document.COLUMN_DOCUMENT_ID, Document.COLUMN_DISPLAY_NAME),
            null,
            null,
            null,
        ).use { cursor ->
            while (cursor?.moveToNext() == true) {
                if (cursor.getString(1) == name) {
                    return DocumentsContract.buildDocumentUri(authority, cursor.getString(0))
                }
            }
        }
        return null
    }

    private fun readDocument(uri: Uri): ByteArray =
        context.contentResolver.openInputStream(uri).use { stream ->
            assertNotNull("DocumentsProvider returned no input stream for $uri", stream)
            stream!!.readBytes()
        }

    private fun expectedContent(): ByteArray = Base64.decode(
        requiredArgument("content_b64"),
        Base64.URL_SAFE or Base64.NO_WRAP or Base64.NO_PADDING,
    )

    private fun requiredArgument(name: String): String =
        requireNotNull(arguments.getString(name)?.takeIf(String::isNotBlank)) {
            "physical provider test requires instrumentation argument $name"
        }

    private fun sha256(bytes: ByteArray): String = MessageDigest.getInstance("SHA-256")
        .digest(bytes)
        .joinToString("") { "%02x".format(it) }

    private fun evidence(line: String) {
        instrumentation.sendStatus(0, Bundle().apply { putString("stream", "$line\n") })
    }
}
