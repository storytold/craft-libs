package dev.craft.kit

import org.junit.Assert.assertEquals
import org.junit.Test

class FingerTapsTest {
    private fun taps() = FingerTaps(slopPx = 20f)

    @Test
    fun twoAndThreeFingerTapsAreRecognised() {
        val t = taps()
        t.down(0, 100f, 100f, 0)
        t.down(1, 200f, 100f, 20)
        assertEquals("not until the last finger lifts", 0, t.up(0, 120))
        assertEquals(2, t.up(1, 130))
        t.down(0, 0f, 0f, 1000)
        t.down(1, 50f, 0f, 1010)
        t.down(2, 100f, 0f, 1020)
        t.up(0, 1100)
        t.up(1, 1110)
        assertEquals(3, t.up(2, 1120))
    }

    @Test
    fun pinchesPansSlowPressesAndSingleTapsAreNot() {
        val t = taps()
        t.down(0, 100f, 100f, 0)
        t.down(1, 200f, 100f, 10)
        t.move(1, 260f, 100f)
        t.up(0, 100)
        assertEquals("a pinch moved", 0, t.up(1, 110))
        t.down(0, 100f, 100f, 1000)
        t.down(1, 200f, 100f, 1010)
        t.up(0, 1500)
        assertEquals("held too long", 0, t.up(1, 1510))
        t.down(0, 100f, 100f, 2000)
        assertEquals("one finger is egui's tap, not a gesture", 0, t.up(0, 2050))
    }

    @Test
    fun aStylusSpoilsTheGesture() {
        val t = taps()
        t.down(0, 100f, 100f, 0)
        t.down(1, 200f, 100f, 10)
        t.spoil()
        t.up(0, 50)
        assertEquals(0, t.up(1, 60))
    }
}
