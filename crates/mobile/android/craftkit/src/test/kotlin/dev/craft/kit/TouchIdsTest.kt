package dev.craft.kit

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Test

class TouchIdsTest {
    /** Android reuses pointer 0 for the next finger; the app must see a new touch. */
    @Test
    fun aReusedPointerIsANewTouch() {
        val ids = TouchIds()
        val first = ids.begin(0)
        assertEquals(first, ids[0])
        assertEquals(first, ids.end(0))
        assertNull(ids[0])
        assertNotEquals(first, ids.begin(0))
    }

    @Test
    fun cancellingEndsEveryLiveTouch() {
        val ids = TouchIds()
        val a = ids.begin(0)
        val b = ids.begin(1)
        assertEquals(setOf(a, b), ids.endAll().toSet())
        assertNull(ids[1])
    }
}
