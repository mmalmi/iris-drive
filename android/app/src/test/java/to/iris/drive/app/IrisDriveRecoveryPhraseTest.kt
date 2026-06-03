package to.iris.drive.app

import org.junit.Assert.assertEquals
import org.junit.Test

class IrisDriveRecoveryPhraseTest {
    @Test
    fun fillRecoveryWordsKeepsSingleWordEntryOnCurrentIndex() {
        val result = fillRecoveryWords(List(24) { "" }, 3, "  Apple  ")

        assertEquals(3, result.index)
        assertEquals("apple", result.words[3])
        assertEquals(24, result.words.size)
    }

    @Test
    fun fillRecoveryWordsCanPastePhraseWithoutNeedingAReviewScreen() {
        val pasted = (1..24).joinToString(" ") { "word$it" }
        val result = fillRecoveryWords(List(24) { "" }, 0, pasted)

        assertEquals(23, result.index)
        assertEquals("word1", result.words.first())
        assertEquals("word24", result.words.last())
        assertEquals(pasted, recoveryPhraseFromWords(result.words))
    }

    @Test
    fun fillRecoveryWordsBoundsPasteToTwentyFourWords() {
        val pasted = (1..30).joinToString(" ") { "word$it" }
        val result = fillRecoveryWords(emptyList(), 20, pasted)

        assertEquals(23, result.index)
        assertEquals(24, result.words.size)
        assertEquals("word1", result.words[20])
        assertEquals("word4", result.words[23])
    }
}
