package dev.craft.kit

/**
 * Touch ids that stay unique for the app's lifetime. Android reuses pointer ids (the next
 * finger down is pointer 0 again); egui must never mistake a new touch for one that ended.
 */
class TouchIds {
    private val live = HashMap<Int, Long>()
    private var next = 1L

    /** A new touch on [pointerId]. */
    fun begin(pointerId: Int): Long = next++.also { live[pointerId] = it }

    /** The touch on [pointerId], if it began. */
    operator fun get(pointerId: Int): Long? = live[pointerId]

    /** The touch on [pointerId] ended; returns its id. */
    fun end(pointerId: Int): Long? = live.remove(pointerId)

    /** Every live touch, ending them all. */
    fun endAll(): List<Long> = live.values.toList().also { live.clear() }
}
