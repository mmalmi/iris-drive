package to.iris.drive.app

import org.junit.Assert.assertEquals
import org.junit.Test

class IrisDriveRecoveryPhraseTest {
    @Test
    fun fillRecoveryWordsKeepsSingleWordEntryOnCurrentIndex() {
        val result = fillRecoveryWords(List(RecoveryPhraseWordCount) { "" }, 3, "  Apple  ")

        assertEquals(3, result.index)
        assertEquals("apple", result.words[3])
        assertEquals(RecoveryPhraseWordCount, result.words.size)
    }

    @Test
    fun fillRecoveryWordsCanPastePhraseWithoutNeedingAReviewScreen() {
        val pasted = (1..RecoveryPhraseWordCount).joinToString(" ") { "word$it" }
        val result = fillRecoveryWords(List(RecoveryPhraseWordCount) { "" }, 0, pasted)

        assertEquals(RecoveryPhraseWordCount - 1, result.index)
        assertEquals("word1", result.words.first())
        assertEquals("word$RecoveryPhraseWordCount", result.words.last())
        assertEquals(pasted, recoveryPhraseFromWords(result.words))
    }

    @Test
    fun fillRecoveryWordsBoundsPasteToTwelveWords() {
        val pasted = (1..30).joinToString(" ") { "word$it" }
        val result = fillRecoveryWords(emptyList(), 8, pasted)

        assertEquals(RecoveryPhraseWordCount - 1, result.index)
        assertEquals(RecoveryPhraseWordCount, result.words.size)
        assertEquals("word1", result.words[8])
        assertEquals("word4", result.words[RecoveryPhraseWordCount - 1])
    }
}
