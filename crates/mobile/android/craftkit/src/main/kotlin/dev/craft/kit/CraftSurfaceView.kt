package dev.craft.kit

import android.annotation.SuppressLint
import android.content.Context
import android.os.Build
import android.text.InputType
import android.view.KeyEvent
import android.view.MotionEvent
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.ViewConfiguration
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager

/**
 * The surface the app draws into. Its lifecycle drives the app's: the first surface starts it,
 * later ones move it (Android takes the surface away whenever the activity is covered, for
 * instance by the document picker), and size changes resize it.
 *
 * Input goes to the app in dp: fingers as touches, a stylus as stylus samples (fingers are
 * ignored while it is down: the palm resting on the screen), a mouse or trackpad as hover,
 * scroll, clicks and right clicks, and keys and on-screen keyboard text as keys and text.
 */
@SuppressLint("ViewConstructor")
internal class CraftSurfaceView(context: Context, private val host: CraftHost) : SurfaceView(context), SurfaceHolder.Callback {
    private val touches = TouchIds()
    private val styli = HashSet<Int>()
    private var drawing = false
    private var keyboardShown = false

    /** A right-button press being swallowed until its release (the click was sent on press). */
    private var secondaryPress = false
    private val density get() = resources.displayMetrics.density
    private val scrollFactors = ViewConfiguration.get(context).let { it.scaledHorizontalScrollFactor to it.scaledVerticalScrollFactor }

    init {
        holder.addCallback(this)
        isFocusable = true
        isFocusableInTouchMode = true
    }

    override fun surfaceCreated(holder: SurfaceHolder) = Unit

    override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) {
        if (drawing) {
            CraftNative.resize(width, height, density)
        } else {
            drawing = CraftNative.start(holder.surface, width, height, density)
        }
    }

    override fun surfaceDestroyed(holder: SurfaceHolder) {
        if (drawing) CraftNative.detach()
        drawing = false
    }

    // The on-screen keyboard, while the app has a text field focused.

    fun showKeyboard(show: Boolean) {
        keyboardShown = show
        val ime = context.getSystemService(InputMethodManager::class.java)
        if (show) {
            requestFocus()
            ime.restartInput(this)
            ime.showSoftInput(this, 0)
        } else {
            ime.hideSoftInputFromWindow(windowToken, 0)
            // Drop the connection too: a keyboard still connected to the view shows itself
            // again at the next hardware key (a Ctrl+A on the canvas).
            ime.restartInput(this)
        }
    }

    override fun onCheckIsTextEditor(): Boolean = keyboardShown

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection? {
        if (!keyboardShown) return null
        // Plain characters, committed as typed: no suggestions or composing to undo (egui's text
        // fields edit their own text and can't take a composition).
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT or InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD or InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        outAttrs.imeOptions = EditorInfo.IME_ACTION_DONE or EditorInfo.IME_FLAG_NO_FULLSCREEN or EditorInfo.IME_FLAG_NO_EXTRACT_UI
        return KeyboardConnection()
    }

    /** What the on-screen keyboard types, as text and keys. A composition still in progress is typed, then retyped as it changes. */
    private inner class KeyboardConnection : BaseInputConnection(this, false) {
        private var composing = ""

        override fun commitText(text: CharSequence, newCursorPosition: Int): Boolean {
            replaceComposing(text.toString())
            composing = ""
            return true
        }

        override fun setComposingText(text: CharSequence, newCursorPosition: Int): Boolean {
            replaceComposing(text.toString())
            composing = text.toString()
            return true
        }

        override fun finishComposingText(): Boolean {
            composing = ""
            return true
        }

        override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
            repeat(beforeLength) { tap(KeyEvent.KEYCODE_DEL) }
            repeat(afterLength) { tap(KeyEvent.KEYCODE_FORWARD_DEL) }
            return true
        }

        override fun performEditorAction(actionCode: Int): Boolean {
            tap(KeyEvent.KEYCODE_ENTER)
            return true
        }

        private fun replaceComposing(text: String) {
            repeat(composing.length) { tap(KeyEvent.KEYCODE_DEL) }
            if (text.isNotEmpty()) CraftNative.text(text)
        }

        private fun tap(code: Int) {
            CraftNative.key(code, true, 0)
            CraftNative.key(code, false, 0)
        }
    }

    // Hardware keys.

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        val command = event.isCtrlPressed || event.isMetaPressed
        if (command && keyCode == KeyEvent.KEYCODE_V || keyCode == KeyEvent.KEYCODE_PASTE) {
            // egui reads the clipboard through the shell: paste what it holds.
            if (event.repeatCount == 0) host.paste()
            return true
        }
        val taken = CraftNative.key(keyCode, true, event.metaState)
        val char = event.unicodeChar
        if (!command && char != 0 && !Character.isISOControl(char)) {
            CraftNative.text(String(Character.toChars(char)))
            return true
        }
        return taken || super.onKeyDown(keyCode, event)
    }

    override fun onKeyUp(keyCode: Int, event: KeyEvent): Boolean = CraftNative.key(keyCode, false, event.metaState) || super.onKeyUp(keyCode, event)

    // Touch, stylus and mouse buttons.

    @SuppressLint("ClickableViewAccessibility") // The canvas is the app's own UI; egui handles taps.
    override fun onTouchEvent(event: MotionEvent): Boolean {
        if (event.getToolType(0) == MotionEvent.TOOL_TYPE_MOUSE && mouseButton(event)) return true
        watchFingerTaps(event)
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN -> {
                // Strokes want every sample as it happens, not batched to the next frame.
                if (event.actionMasked == MotionEvent.ACTION_DOWN) requestUnbufferedDispatch(event)
                begin(event, event.actionIndex)
            }
            MotionEvent.ACTION_MOVE -> {
                for (h in 0 until event.historySize) {
                    for (i in 0 until event.pointerCount) sample(event, i, PHASE_MOVED, h)
                }
                for (i in 0 until event.pointerCount) sample(event, i, PHASE_MOVED)
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> {
                // Android cancels a pointer it decides was a palm.
                val phase = if (Build.VERSION.SDK_INT >= 33 && event.flags and MotionEvent.FLAG_CANCELED != 0) PHASE_CANCELLED else PHASE_ENDED
                end(event, event.actionIndex, phase)
            }
            MotionEvent.ACTION_CANCEL -> {
                for (i in 0 until event.pointerCount) end(event, i, PHASE_CANCELLED)
                touches.endAll().forEach { CraftNative.touch(it, PHASE_CANCELLED, 0f, 0f, 0f) }
                styli.clear()
            }
            else -> return false
        }
        return true
    }

    /** Two-finger tap undoes, three-finger tap redoes, as on the iPad. Fingers only. */
    private val fingerTaps = FingerTaps(slopPx = android.view.ViewConfiguration.get(context).scaledTouchSlop.toFloat())

    private fun watchFingerTaps(event: MotionEvent) {
        val index = event.actionIndex
        val finger = { i: Int -> event.getToolType(i) == MotionEvent.TOOL_TYPE_FINGER }
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN, MotionEvent.ACTION_POINTER_DOWN ->
                if (finger(index)) fingerTaps.down(event.getPointerId(index), event.getX(index), event.getY(index), event.eventTime) else fingerTaps.spoil()
            MotionEvent.ACTION_MOVE -> for (i in 0 until event.pointerCount) fingerTaps.move(event.getPointerId(i), event.getX(i), event.getY(i))
            MotionEvent.ACTION_UP, MotionEvent.ACTION_POINTER_UP -> when (fingerTaps.up(event.getPointerId(index), event.eventTime)) {
                2 -> host.perform(CraftAction.Undo)
                3 -> host.perform(CraftAction.Redo)
            }
            MotionEvent.ACTION_CANCEL -> fingerTaps.clear()
        }
    }

    private fun begin(event: MotionEvent, index: Int) {
        val pointer = event.getPointerId(index)
        if (isStylus(event, index)) {
            // Palm rejection: the hand resting on the screen while drawing isn't a gesture.
            if (styli.isEmpty()) cancelFingers()
            styli += pointer
        } else if (styli.isNotEmpty()) {
            return
        }
        touches.begin(pointer)
        sample(event, index, PHASE_BEGAN)
    }

    private fun end(event: MotionEvent, index: Int, phase: Int) {
        val pointer = event.getPointerId(index)
        if (touches[pointer] == null) return
        sample(event, index, phase)
        touches.end(pointer)
        styli -= pointer
    }

    private fun cancelFingers() {
        for (id in touches.endAll()) CraftNative.touch(id, PHASE_CANCELLED, 0f, 0f, 0f)
    }

    /** Pointer [index]'s current reading, or its historical reading [history]. */
    private fun sample(event: MotionEvent, index: Int, phase: Int, history: Int = -1) {
        val id = touches[event.getPointerId(index)] ?: return
        fun axis(axis: Int) = if (history < 0) event.getAxisValue(axis, index) else event.getHistoricalAxisValue(axis, index, history)
        val x = axis(MotionEvent.AXIS_X) / density
        val y = axis(MotionEvent.AXIS_Y) / density
        if (isStylus(event, index)) {
            val pressure = axis(MotionEvent.AXIS_PRESSURE).coerceIn(0f, 1f)
            val altitude = StylusAxes.altitude(axis(MotionEvent.AXIS_TILT))
            val azimuth = StylusAxes.azimuth(axis(MotionEvent.AXIS_ORIENTATION))
            val eraser = event.getToolType(index) == MotionEvent.TOOL_TYPE_ERASER
            CraftNative.stylus(id, phase, x, y, pressure, altitude, azimuth, eraser)
        } else {
            // A finger's "pressure" is contact area, not force.
            CraftNative.touch(id, phase, x, y, 0f)
        }
    }

    private fun isStylus(event: MotionEvent, index: Int) = when (event.getToolType(index)) {
        MotionEvent.TOOL_TYPE_STYLUS, MotionEvent.TOOL_TYPE_ERASER -> true
        else -> false
    }

    /** A mouse's right button is a secondary click, sent on press; true when [event] was it. */
    private fun mouseButton(event: MotionEvent): Boolean {
        val secondaryOnly = event.buttonState and MotionEvent.BUTTON_SECONDARY != 0 && event.buttonState and MotionEvent.BUTTON_PRIMARY == 0
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> if (secondaryOnly) {
                secondaryPress = true
                CraftNative.secondaryClick(event.x / density, event.y / density)
                return true
            }
            MotionEvent.ACTION_BUTTON_PRESS, MotionEvent.ACTION_BUTTON_RELEASE -> return true
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> if (secondaryPress) {
                secondaryPress = false
                return true
            }
        }
        return secondaryPress
    }

    // Hover and scroll: a mouse, a trackpad, or a stylus above the screen.

    override fun onGenericMotionEvent(event: MotionEvent): Boolean {
        val x = event.x / density
        val y = event.y / density
        when (event.actionMasked) {
            MotionEvent.ACTION_HOVER_ENTER, MotionEvent.ACTION_HOVER_MOVE -> CraftNative.hover(x, y, true)
            MotionEvent.ACTION_HOVER_EXIT -> CraftNative.hover(x, y, false)
            MotionEvent.ACTION_SCROLL -> {
                // Android's wheel axes are positive for "scroll up" and "scroll right"; the app's
                // positive moves content down and right, so vertical keeps its sign and horizontal flips.
                val dx = -event.getAxisValue(MotionEvent.AXIS_HSCROLL) * scrollFactors.first / density
                val dy = event.getAxisValue(MotionEvent.AXIS_VSCROLL) * scrollFactors.second / density
                CraftNative.scroll(x, y, dx, dy)
            }
            else -> return super.onGenericMotionEvent(event)
        }
        return true
    }

    private companion object {
        const val PHASE_BEGAN = 0
        const val PHASE_MOVED = 1
        const val PHASE_ENDED = 2
        const val PHASE_CANCELLED = 3
    }
}
