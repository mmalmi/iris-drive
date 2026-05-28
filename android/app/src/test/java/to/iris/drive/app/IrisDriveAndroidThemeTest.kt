package to.iris.drive.app

import androidx.compose.ui.graphics.Color
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Test

class IrisDriveAndroidThemeTest {
    @Test
    fun colorSchemeTracksLightAndDarkModes() {
        val light = irisDriveColorScheme(darkTheme = false)
        val dark = irisDriveColorScheme(darkTheme = true)

        assertEquals(Color(0xFF167C80), light.primary)
        assertEquals(light.primary, dark.primary)
        assertEquals(Color(0xFFF7FAF8), light.background)
        assertEquals(Color(0xFF101815), dark.background)
        assertNotEquals(light.surface, dark.surface)
        assertNotEquals(light.onSurface, dark.onSurface)
    }
}
