package to.iris.drive.app

import android.content.Intent
import android.provider.DocumentsContract
import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith

@RunWith(AndroidJUnit4::class)
class IrisDriveFilesIntentTest {
    @Test
    fun openInFilesViewsProviderRootWithoutFolderGrantPicker() {
        val intent = irisDriveFilesIntent("to.iris.drive.documents")

        assertEquals(Intent.ACTION_VIEW, intent.action)
        assertEquals(
            DocumentsContract.buildRootUri("to.iris.drive.documents", "iris-drive"),
            intent.data,
        )
        assertNull(intent.type)
        assertEquals(0, intent.flags and Intent.FLAG_GRANT_PERSISTABLE_URI_PERMISSION)
    }
}
