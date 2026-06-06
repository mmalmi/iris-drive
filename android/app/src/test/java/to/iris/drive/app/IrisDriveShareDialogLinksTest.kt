package to.iris.drive.app

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class IrisDriveShareDialogLinksTest {
    @Test
    fun shareDialogLinkEncodesProviderFolderPathAndName() {
        assertEquals(
            "iris-drive://share?path=Projects%2FAlpha+%26+Beta&name=Alpha+%2B+Beta",
            irisDriveShareDialogLink(" Projects/Alpha & Beta ", " Alpha + Beta "),
        )
    }

    @Test
    fun shareDialogLinkRejectsBlankFolderPath() {
        assertNull(irisDriveShareDialogLink("   ", "Shared"))
    }
}
