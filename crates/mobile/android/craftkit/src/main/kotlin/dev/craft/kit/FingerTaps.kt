package dev.craft.kit

import kotlin.math.hypot

/**
 * Two- and three-finger taps (undo, redo), the iPad's gestures, from raw finger events. A tap is
 * every finger lifting quickly without travelling; pinches, pans and anything with a stylus are not.
 * Plain Kotlin, so it is tested on the JVM.
 *
 * Feed it each finger going down, moving and up (in view pixels, with event times); [up] returns how
 * many fingers tapped when the last one lifts on a tap, else 0.
 */
internal class FingerTaps(private val slopPx: Float, private val maxMillis: Long = 350) {
    private val starts = HashMap<Int, Pair<Float, Float>>()
    private var downAt = 0L
    private var fingers = 0
    private var spoiled = false

    fun down(pointer: Int, x: Float, y: Float, timeMillis: Long) {
        if (starts.isEmpty()) {
            downAt = timeMillis
            fingers = 0
            spoiled = false
        }
        starts[pointer] = x to y
        fingers = maxOf(fingers, starts.size)
    }

    fun move(pointer: Int, x: Float, y: Float) {
        val (sx, sy) = starts[pointer] ?: return
        if (hypot(x - sx, y - sy) > slopPx) spoiled = true
    }

    /** A stylus touched, or the system cancelled the gesture: nothing in progress is a tap. */
    fun spoil() {
        spoiled = true
    }

    fun up(pointer: Int, timeMillis: Long): Int {
        if (starts.remove(pointer) == null || starts.isNotEmpty()) return 0
        val tapped = !spoiled && timeMillis - downAt <= maxMillis && fingers >= 2
        return if (tapped) fingers else 0
    }

    fun clear() {
        starts.clear()
        spoiled = false
    }
}
