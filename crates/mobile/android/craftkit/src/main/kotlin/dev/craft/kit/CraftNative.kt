package dev.craft.kit

import android.view.Surface

/**
 * The Rust side (craft-mobile's `android.rs`), as the app's library exports it. Every call
 * returns at once: the render thread owns the app and does the work, except [detach], which
 * waits until the surface is no longer used, and [suspend], which waits for the app's recovery
 * copies.
 *
 * Load the app's library before the first call ([CraftHost] does).
 */
object CraftNative {
    /** Hears what the app asks of the shell, on the render thread. */
    fun interface Handler {
        /**
         * [kind] 0: pick one file, 1: pick files ([text]: allowed extensions, `,`-joined);
         * 2: export [data] ([text]: the suggested file name); 3: panel state ([text]: a snapshot as JSON);
         * 4: copy [text]; 5: copy the image [data] (RGBA8, [text]: `width×height`); 6: open the URL [text];
         * 7: show ([text] `1`) or hide (`0`) the on-screen keyboard; 8: write [data] over the document at a
         * location ([text]: the location, a line break, the file name), answered with [writeDone];
         * 9: the app is busy ([text] `1`) with work that must finish even in the background, or done (`0`);
         * 10: the user asked to paste: read the clipboard and answer with [pasteText], [pasteImage] or [pasteOwn];
         * 11: share [data] ([text]: its file name) through the share sheet; 12: show the Photo Picker.
         */
        fun onRequest(kind: Int, text: String, data: ByteArray?)
    }

    /** Start the app drawing into [surface], or move the running app there. Sizes in pixels. */
    @JvmStatic external fun start(surface: Surface, widthPx: Int, heightPx: Int, scale: Float): Boolean

    /** Where requests go; `null` to stop hearing them. */
    @JvmStatic external fun setHandler(handler: Handler?)

    /** The surface is going away; returns once the app no longer draws into it. The app keeps running. */
    @JvmStatic external fun detach()

    /** The app's own folder (`Context.filesDir`). Call before [start]. */
    @JvmStatic external fun setDataDir(path: String)

    /** The app left the screen (`onStop`); returns once it has written its recovery data. */
    @JvmStatic external fun suspend()

    /** The app is back on screen (`onStart`). */
    @JvmStatic external fun resume()

    /**
     * The device's power state: [thermalStatus] as `PowerManager.getCurrentThermalStatus` (-1 below API 29),
     * [powerSave] whether Battery Saver is on. Send it at start and whenever either changes.
     */
    @JvmStatic external fun power(thermalStatus: Int, powerSave: Boolean)

    /** The system is short of memory: the app drops its caches. */
    @JvmStatic external fun memoryWarning()

    /** Drop the app. */
    @JvmStatic external fun stop()

    @JvmStatic external fun resize(widthPx: Int, heightPx: Int, scale: Float)

    /** A touch in dp from the view's top-left. [phase]: 0 began, 1 moved, 2 ended, 3 cancelled. */
    @JvmStatic external fun touch(id: Long, phase: Int, x: Float, y: Float, force: Float)

    /**
     * A file the system handed over ("Open with", Share). [location] is its content URI when the app may
     * write it back (Save then writes in place), else `null`.
     */
    @JvmStatic external fun openFile(name: String, data: ByteArray, location: String?)

    /** One file chosen in the open picker, [location] as for [openFile]; finish with [pickDone]. */
    @JvmStatic external fun pickAdd(name: String, data: ByteArray, location: String?)

    /** A write in place (request kind 8) to [location] ended: [error] is `null` when the file was written. */
    @JvmStatic external fun writeDone(location: String, error: String?)

    /** The open picker closed: the files added since it opened are the answer; none means cancelled. */
    @JvmStatic external fun pickDone()

    /** A hardware key, as `KeyEvent` reports it. False when the app doesn't take the key (Back, volume). */
    @JvmStatic external fun key(keyCode: Int, pressed: Boolean, metaState: Int): Boolean

    /** Text typed, by the on-screen keyboard or a hardware key. */
    @JvmStatic external fun text(text: String)

    /** Paste [text], after the user asked to. */
    @JvmStatic external fun pasteText(text: String)

    /** Paste, after the user asked to: the clipboard still holds the image the app copied last. */
    @JvmStatic external fun pasteOwn()

    /** Paste an image, RGBA8 unpremultiplied, after the user asked to. */
    @JvmStatic external fun pasteImage(rgba: ByteArray, width: Int, height: Int)

    /** A pointer hovering without a button down, in dp; [inside] false when it left the view. */
    @JvmStatic external fun hover(x: Float, y: Float, inside: Boolean)

    /** A scroll at (x, y), in dp; positive moves content right and down. */
    @JvmStatic external fun scroll(x: Float, y: Float, dx: Float, dy: Float)

    /** A right click, in dp. */
    @JvmStatic external fun secondaryClick(x: Float, y: Float)

    /**
     * A stylus sample instead of a touch: phases and units as [touch]; [altitude] radians from the
     * screen (π/2 upright), [azimuth] radians clockwise from +x toward where the stylus leans.
     */
    @JvmStatic external fun stylus(id: Long, phase: Int, x: Float, y: Float, pressure: Float, altitude: Float, azimuth: Float, eraser: Boolean)

    /** A native control was used: a [CraftAction] as JSON. False when it isn't one. */
    @JvmStatic external fun action(json: String): Boolean
}
