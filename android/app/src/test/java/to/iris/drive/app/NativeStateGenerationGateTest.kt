package to.iris.drive.app

import org.junit.Assert.assertFalse
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class NativeStateGenerationGateTest {
    @Test
    fun staleRefreshCannotOverwriteLaterApproval() {
        val gate = NativeStateGenerationGate()
        val refresh = gate.request()
        val approval = gate.request()

        assertTrue(gate.shouldApply(approval))
        assertFalse(gate.shouldApply(refresh))
    }

    @Test
    fun authoritativePostApprovalRefreshSupersedesApprovalState() {
        val gate = NativeStateGenerationGate()
        val approval = gate.request()
        val authoritativeRefresh = gate.request()

        assertFalse(gate.shouldApply(approval))
        assertTrue(gate.shouldApply(authoritativeRefresh))
    }

    @Test
    fun inOrderCompletionApplies() {
        val gate = NativeStateGenerationGate()

        assertTrue(gate.shouldApply(gate.request()))
    }

    @Test
    fun reorderedCompletionsStayMonotonicUnderLoad() {
        val gate = NativeStateGenerationGate()
        val generations = (1..1_000).map { gate.request() }

        generations.dropLast(1).reversed().forEach { generation ->
            assertFalse(gate.shouldApply(generation))
        }
        assertTrue(gate.shouldApply(generations.last()))
    }

    @Test
    fun nativeApprovalErrorsAreLoggedWithoutRequestOrKeyMaterial() {
        assertEquals("none", nativeErrorCategory(""))
        assertEquals(
            "invalid_request",
            nativeErrorCategory("app-key approval bootstrap is missing or invalid"),
        )
        assertEquals("approval_apply", nativeErrorCategory("approving device: wrong profile"))
        assertEquals("config_lock", nativeErrorCategory("locking config mutation: busy"))
        assertEquals("config_save", nativeErrorCategory("saving config: disk full"))
    }
}
