package dev.craft.kit

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.PI

class PlatformTest {
    private val halfPi = (PI / 2).toFloat()

    @Test
    fun anUprightStylusStandsAtAQuarterTurnAndAFlatOneLies() {
        assertEquals(halfPi, StylusAxes.altitude(0f), 1e-6f)
        assertEquals(0f, StylusAxes.altitude(halfPi), 1e-6f)
        assertEquals(0f, StylusAxes.altitude(5f), 1e-6f)
    }

    /** The cap leans opposite the tip: a tip pointing up leans toward the user (+y, a quarter turn from +x). */
    @Test
    fun theAzimuthIsWhereTheCapLeans() {
        assertEquals(halfPi, StylusAxes.azimuth(0f), 1e-6f) // tip up, cap down
        assertEquals(PI.toFloat(), StylusAxes.azimuth(halfPi), 1e-6f) // tip right, cap left
        assertEquals(0f, StylusAxes.azimuth(-halfPi), 1e-6f) // tip left, cap right
        assertEquals((3 * PI / 2).toFloat(), StylusAxes.azimuth(PI.toFloat()), 1e-5f) // tip down, cap up
    }

    @Test
    fun aWriteInPlaceNamesItsLocationAndFile() {
        assertEquals("content://a/document/1" to "a.psd", parseWriteInPlace("content://a/document/1\na.psd"))
        assertEquals("content://a\nb" to "c.png", parseWriteInPlace("content://a\nb\nc.png"))
        assertNull(parseWriteInPlace("content://a"))
        assertNull(parseWriteInPlace("\na.psd"))
        assertNull(parseWriteInPlace("content://a\n"))
    }

    @Test
    fun hiddenIsNotShortOfMemory() {
        assertTrue(MemoryLevels.isWarning(10))
        assertTrue(MemoryLevels.isWarning(15))
        assertFalse(MemoryLevels.isWarning(5))
        assertFalse(MemoryLevels.isWarning(20))
        assertTrue(MemoryLevels.isWarning(40))
        assertTrue(MemoryLevels.isWarning(80))
    }

    @Test
    fun aCopiedImagesSizeParses() {
        assertEquals(12 to 34, parseImageSize("12×34"))
        assertNull(parseImageSize("12x34"))
        assertNull(parseImageSize("0×3"))
        assertNull(parseImageSize("a×3"))
    }
}
