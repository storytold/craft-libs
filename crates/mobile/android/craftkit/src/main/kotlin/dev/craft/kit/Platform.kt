package dev.craft.kit

import kotlin.math.PI

/** Android's stylus axes in the terms craft-mobile's `Stylus` uses. */
object StylusAxes {
    private const val HALF_PI = (PI / 2).toFloat()
    private const val TAU = (2 * PI).toFloat()

    /** `AXIS_TILT` (radians from upright) as altitude (radians from the screen: π/2 upright, 0 flat). */
    fun altitude(tilt: Float): Float = HALF_PI - tilt.coerceIn(0f, HALF_PI)

    /**
     * `AXIS_ORIENTATION` (radians clockwise from screen-up toward where the tip points) as azimuth
     * (radians clockwise from +x toward where the stylus leans, its cap): the cap lies opposite
     * the tip, so it is a quarter turn on from the orientation.
     */
    fun azimuth(orientation: Float): Float = (orientation + HALF_PI).mod(TAU)
}

/** `ComponentCallbacks2.onTrimMemory` levels that mean the app should let go of caches. */
object MemoryLevels {
    private const val RUNNING_LOW = 10 // TRIM_MEMORY_RUNNING_LOW
    private const val UI_HIDDEN = 20 // TRIM_MEMORY_UI_HIDDEN
    private const val BACKGROUND = 40 // TRIM_MEMORY_BACKGROUND

    /** Short of memory while running, or in the background where Android ends apps by size; not merely hidden. */
    fun isWarning(level: Int): Boolean = level in RUNNING_LOW until UI_HIDDEN || level >= BACKGROUND
}

/** A write in place (wire kind 8): its location and file name, or `null`. The name has no line break, so it follows the last. */
fun parseWriteInPlace(text: String): Pair<String, String>? {
    val split = text.lastIndexOf('\n')
    if (split <= 0 || split == text.length - 1) return null
    return text.substring(0, split) to text.substring(split + 1)
}

/** The `width×height` of a copied image (wire kind 5), or `null`. */
fun parseImageSize(text: String): Pair<Int, Int>? {
    val parts = text.split('×')
    if (parts.size != 2) return null
    val width = parts[0].toIntOrNull() ?: return null
    val height = parts[1].toIntOrNull() ?: return null
    return if (width > 0 && height > 0) width to height else null
}
