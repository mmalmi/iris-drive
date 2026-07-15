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
        assertEquals(Color(0xFFF5F5F4), dark.primary)
        assertEquals(Color(0xFF5EEAD4), dark.secondary)
        assertEquals(Color(0xFFF7FAF8), light.background)
        assertEquals(Color(0xFF0C0A09), dark.background)
        assertEquals(Color(0xFF1C1917), dark.surface)
        assertNotEquals(light.surface, dark.surface)
        assertNotEquals(light.onSurface, dark.onSurface)
    }
}
