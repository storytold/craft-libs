package dev.craft.kit

import android.content.BroadcastReceiver
import android.content.ClipData
import android.content.ContentValues
import android.content.ClipboardManager
import android.content.ComponentCallbacks2
import android.content.ContentResolver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.res.Configuration
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.net.Uri
import android.content.pm.PackageManager
import android.os.Build
import android.os.Environment
import android.os.PowerManager
import android.os.Process
import android.provider.DocumentsContract
import android.provider.MediaStore
import android.provider.OpenableColumns
import android.util.Log
import android.widget.Toast
import androidx.annotation.RequiresApi
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.nio.ByteBuffer
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/** What the app asked the shell to show: the open picker or the exporter. */
sealed interface FileRequest {
    data class Pick(val multiple: Boolean, val mimeTypes: Array<String>) : FileRequest {
        override fun equals(other: Any?) = other is Pick && multiple == other.multiple && mimeTypes.contentEquals(other.mimeTypes)
        override fun hashCode() = 31 * multiple.hashCode() + mimeTypes.contentHashCode()
    }

    /** Offer [bytes] to save, suggesting [name]. */
    class Export(val name: String, val mimeType: String, val bytes: ByteArray) : FileRequest

    /** Share [bytes] as a file named [name] through the system share sheet. */
    class Share(val name: String, val mimeType: String, val bytes: ByteArray) : FileRequest

    /** Show the system Photo Picker; the photos chosen open as copies. */
    data object PickPhotos : FileRequest
}

/** A document read for the app: its name, contents, and its URI when the app may write it back. */
internal class OpenedFile(val name: String, val bytes: ByteArray, val location: Uri?) {
    /** The location as the app holds it. */
    val locationText: String? get() = location?.toString()
}

/**
 * The running Craft app, as Compose sees it: its panel state, the actions native controls
 * send, and the files that cross between it and the system.
 *
 * Make one per activity (it loads [library], the app's Rust library) and [close] it when the
 * activity finishes. Show it with [CraftCanvas], which also answers its file requests.
 */
class CraftHost(context: Context, library: String) : CraftNative.Handler {
    private val app: Context = context.applicationContext
    private val resolver: ContentResolver = app.contentResolver
    private val clipboard = app.getSystemService(ClipboardManager::class.java)
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val state = MutableStateFlow(Snapshot())

    /** The app's panel state, updated whenever it changes. */
    val snapshot: StateFlow<Snapshot> = state.asStateFlow()

    /** Pickers and exporters to show, in order; [CraftCanvas] shows them. */
    internal val fileRequests = Channel<FileRequest>(Channel.UNLIMITED)

    /** The bytes waiting for the exporter's answer (main thread only). */
    private var exporting: ByteArray? = null

    /** Writes in place, one at a time and in order, off the main thread. */
    @OptIn(ExperimentalCoroutinesApi::class)
    private val writes = Dispatchers.IO.limitedParallelism(1)

    private val keyboardWanted = MutableStateFlow(false)

    /** Whether the app wants the on-screen keyboard (a text field has focus); [CraftCanvas] shows it. */
    val keyboard: StateFlow<Boolean> = keyboardWanted.asStateFlow()

    /** Memory warnings from the system, for the app's caches. */
    private val memory = object : ComponentCallbacks2 {
        override fun onTrimMemory(level: Int) {
            if (MemoryLevels.isWarning(level)) CraftNative.memoryWarning()
        }

        override fun onConfigurationChanged(newConfig: Configuration) = Unit

        @Deprecated("Still sent on older releases", ReplaceWith(""))
        override fun onLowMemory() = CraftNative.memoryWarning()
    }

    private val power = app.getSystemService(PowerManager::class.java)

    /** Thermal status changes (API 29+), on the main thread. */
    private val thermal = PowerManager.OnThermalStatusChangedListener { reportPower() }

    /** Battery Saver turning on or off. */
    private val powerSave = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) = reportPower()
    }

    init {
        System.loadLibrary(library)
        // Before the app starts: it finds its recovery copies and settings here.
        CraftNative.setDataDir(app.filesDir.absolutePath)
        current = this
        CraftNative.setHandler(this)
        app.registerComponentCallbacks(memory)
        if (Build.VERSION.SDK_INT >= 29) power.addThermalStatusListener(app.mainExecutor, thermal)
        ContextCompat.registerReceiver(app, powerSave, IntentFilter(PowerManager.ACTION_POWER_SAVE_MODE_CHANGED), ContextCompat.RECEIVER_NOT_EXPORTED)
        // The app hears it once it starts, then on every change.
        reportPower()
    }

    /** Tell the app how hot the device is and whether Battery Saver is on; it lowers its preview costs. */
    private fun reportPower() {
        val status = if (Build.VERSION.SDK_INT >= 29) power.currentThermalStatus else -1
        Log.i(TAG, "power: thermal status $status, power save ${power.isPowerSaveMode}")
        CraftNative.power(status, power.isPowerSaveMode)
    }

    /** Send [action] to the app. False when the app can't take it. */
    fun perform(action: CraftAction): Boolean = CraftNative.action(action.toJson())

    /** Open the files an `ACTION_VIEW`, `ACTION_SEND` or `ACTION_SEND_MULTIPLE` intent carries; false when it carries none. */
    fun openIntent(intent: Intent?): Boolean {
        val uris = intent?.let(::documentsOf).orEmpty()
        uris.forEach(::openUri)
        return uris.isNotEmpty()
    }

    /** Open the document at [uri] in the app, in place when the app may write it back. */
    fun openUri(uri: Uri) {
        scope.launch {
            val file = withContext(Dispatchers.IO) { read(uri) } ?: return@launch
            CraftNative.openFile(file.name, file.bytes, file.locationText)
        }
    }

    /**
     * Stop the app and stop hearing it. Only the newest host does: an activity that finishes
     * after its replacement started (a relaunch, a recreation) leaves the app to the new one.
     */
    fun close() {
        scope.cancel()
        app.unregisterComponentCallbacks(memory)
        if (Build.VERSION.SDK_INT >= 29) power.removeThermalStatusListener(thermal)
        app.unregisterReceiver(powerSave)
        CraftWorkService.release()
        if (current !== this) return
        current = null
        CraftNative.setHandler(null)
        CraftNative.stop()
    }

    override fun onRequest(kind: Int, text: String, data: ByteArray?) {
        // The render thread: only thread-safe hand-offs here.
        when (kind) {
            0, 1 -> fileRequests.trySend(FileRequest.Pick(kind == 1, FileTypes.openFilter(text.split(','))))
            2 -> fileRequests.trySend(FileRequest.Export(text, FileTypes.mimeOfName(text), data ?: ByteArray(0)))
            3 -> Snapshot.parse(text)?.let { state.value = it } ?: Log.w(TAG, "not a snapshot: $text")
            4 -> scope.launch {
                clipboard.setPrimaryClip(ClipData.newPlainText("Copied text", text))
                ownImage = null
            }
            5 -> parseImageSize(text)?.let { (width, height) -> data?.let { scope.launch { copyImage(it, width, height) } } }
            6 -> scope.launch { openUrl(text) }
            7 -> keyboardWanted.value = text == "1"
            8 -> parseWriteInPlace(text)?.let { (location, name) -> scope.launch { writeInPlace(location, name, data ?: ByteArray(0)) } }
            9 -> scope.launch { if (text == "1") CraftWorkService.hold(app) else CraftWorkService.release() }
            10 -> scope.launch { paste() }
            11 -> fileRequests.trySend(FileRequest.Share(text, FileTypes.mimeOfName(text), data ?: ByteArray(0)))
            12 -> fileRequests.trySend(FileRequest.PickPhotos)
            else -> Log.w(TAG, "unknown request $kind")
        }
    }

    /** The app left the screen (`onStop`): it writes recovery copies of unsaved work before this returns. */
    fun suspend() = CraftNative.suspend()

    /** The app is back on screen (`onStart`). */
    fun resume() = CraftNative.resume()

    /**
     * Paste what the clipboard holds, because the user asked to (Ctrl+V, or the app's own Edit › Paste):
     * text first while a text field is focused, else an image first. The image the app copied itself is
     * never read back while it is still there: the app pastes its own copy, which keeps what an image
     * can't. Nothing to paste is empty text, so the app can say so.
     */
    fun paste() {
        val item = clipboard.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)
        val uri = item?.uri
        if (uri != null && uri == ownImage) {
            CraftNative.pasteOwn()
            return
        }
        val typed = item?.text?.toString()
        // A focused text field wants text even when an image came with it.
        if (keyboardWanted.value && typed != null) {
            CraftNative.pasteText(typed)
            return
        }
        val image = uri?.takeIf { resolver.getType(it)?.startsWith("image/") == true }
        if (image == null) {
            CraftNative.pasteText(item?.coerceToText(app)?.toString().orEmpty())
            return
        }
        scope.launch {
            val decoded = withContext(Dispatchers.IO) { decodeRgba(image) }
            if (decoded == null) CraftNative.pasteText("") else CraftNative.pasteImage(decoded.rgba, decoded.width, decoded.height)
        }
    }

    /** The image the app put on the clipboard last, while nothing else was copied since (main thread). */
    private var ownImage: Uri? = null

    /**
     * Open photos from the Photo Picker as copies, in their own encoding: they belong to the library, so
     * Save asks where. The picker hides their file names, so each is named for when it arrived, as the
     * camera names files ("Photo 2026-11-06 at 14.02.jpg").
     */
    internal fun openPhotos(uris: List<Uri>) {
        val arrived = SimpleDateFormat("yyyy-MM-dd 'at' HH.mm", Locale.ROOT).format(Date())
        scope.launch {
            val photos = withContext(Dispatchers.IO) {
                uris.mapNotNull { uri ->
                    val ext = resolver.getType(uri)?.let(FileTypes::extensionOf) ?: "jpg"
                    runCatching { resolver.openInputStream(uri)?.use { it.readBytes() }?.let { "Photo $arrived.$ext" to it } }
                        .onFailure { Log.e(TAG, "reading the photo $uri", it) }.getOrNull()
                }
            }
            photos.forEach { (name, bytes) -> CraftNative.openFile(name, bytes, null) }
        }
    }

    /**
     * The share sheet's intent for [request]: the bytes as a file in the cache (named as the app named
     * them), shared through craftkit's FileProvider; `null` when it can't be written.
     */
    internal suspend fun shareIntent(request: FileRequest.Share): Intent? = withContext(Dispatchers.IO) {
        runCatching {
            val dir = File(app.cacheDir, SHARE_DIR)
            // Only the newest share is kept: an older one has been handed over by now.
            dir.deleteRecursively()
            val file = File(File(dir, System.nanoTime().toString()).apply { mkdirs() }, request.name.replace('/', '_'))
            file.writeBytes(request.bytes)
            val uri = FileProvider.getUriForFile(app, "${app.packageName}.craftkit.files", file)
            val send = Intent(Intent.ACTION_SEND)
                .setType(request.mimeType)
                .putExtra(Intent.EXTRA_STREAM, uri)
                .putExtra(Intent.EXTRA_TITLE, request.name)
                .addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
            // The chooser's preview and the receiving app both read it through the clip.
            send.clipData = ClipData.newUri(resolver, request.name, uri)
            Intent.createChooser(send, request.name)
        }.onFailure { Log.e(TAG, "sharing ${request.name}", it) }.getOrNull()
    }

    /** Put an image on the clipboard: as a PNG in the cache, shared through craftkit's FileProvider. */
    private suspend fun copyImage(rgba: ByteArray, width: Int, height: Int) {
        val uri = withContext(Dispatchers.IO) {
            runCatching {
                if (rgba.size != width * height * 4) error("$width × $height doesn't match ${rgba.size} bytes")
                val bitmap = Bitmap.createBitmap(width, height, Bitmap.Config.ARGB_8888)
                // The app's pixels are unpremultiplied, as PNG stores them: copy them as they are.
                bitmap.isPremultiplied = false
                bitmap.copyPixelsFromBuffer(ByteBuffer.wrap(rgba))
                val dir = File(app.cacheDir, CLIPBOARD_DIR).apply { mkdirs() }
                val file = File(dir, "Copied image.png")
                file.outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
                bitmap.recycle()
                FileProvider.getUriForFile(app, "${app.packageName}.craftkit.files", file)
            }.onFailure { Log.e(TAG, "copying an image", it) }.getOrNull()
        } ?: return
        clipboard.setPrimaryClip(ClipData.newUri(resolver, "Copied image", uri))
        ownImage = uri
    }

    private class Rgba(val rgba: ByteArray, val width: Int, val height: Int)

    /** An image file as RGBA8, unpremultiplied (what craft-mobile takes). */
    private fun decodeRgba(uri: Uri): Rgba? = runCatching {
        val options = BitmapFactory.Options().apply {
            inPremultiplied = false
            inPreferredConfig = Bitmap.Config.ARGB_8888
        }
        val bitmap = resolver.openInputStream(uri)?.use { BitmapFactory.decodeStream(it, null, options) } ?: error("not an image")
        val pixels = ByteBuffer.allocate(bitmap.width * bitmap.height * 4)
        bitmap.copyPixelsToBuffer(pixels)
        Rgba(pixels.array(), bitmap.width, bitmap.height).also { bitmap.recycle() }
    }.onFailure { Log.e(TAG, "reading a pasted image", it) }.getOrNull()

    private fun openUrl(url: String) {
        runCatching { app.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
            .onFailure { Log.w(TAG, "no app opens $url", it) }
    }

    /** The open picker closed with [uris] (none: cancelled); read them and answer the app. */
    internal fun answerPick(uris: List<Uri>) {
        scope.launch {
            val files = withContext(Dispatchers.IO) { uris.mapNotNull(::read) }
            files.forEach { CraftNative.pickAdd(it.name, it.bytes, it.locationText) }
            CraftNative.pickDone()
        }
    }

    /**
     * Write [bytes] back over the document at [location] (main thread), then tell the app. If it can't be
     * written, the exporter offers the same bytes, so nothing is lost.
     */
    private suspend fun writeInPlace(location: String, name: String, bytes: ByteArray) {
        val uri = Uri.parse(location)
        val error = withContext(writes) {
            runCatching { write(uri, bytes) }.onFailure { Log.e(TAG, "writing $uri in place", it) }.exceptionOrNull()
        }
        if (error != null) {
            Toast.makeText(app, "Couldn't save “$name” where it came from. Choose where to save it.", Toast.LENGTH_LONG).show()
            fileRequests.trySend(FileRequest.Export(name, FileTypes.mimeOfName(name), bytes))
        }
        CraftNative.writeDone(location, error?.let { it.message ?: it.javaClass.simpleName })
    }

    /** Replace the document's contents. "wt": a shorter file must not keep the old tail; providers that refuse it take "rwt". */
    private fun write(uri: Uri, bytes: ByteArray) {
        val stream = runCatching { resolver.openOutputStream(uri, "wt") }.getOrNull() ?: resolver.openOutputStream(uri, "rwt")
        stream?.use { it.write(bytes) } ?: error("no stream")
    }

    /**
     * Where [uri] can be written back, or `null` for a copy. Keeps the access across restarts where the
     * grant allows it (the open picker's grants are persistable; "Open with" grants last while the app runs).
     */
    private fun writableLocation(uri: Uri): Uri? {
        if (uri.scheme != ContentResolver.SCHEME_CONTENT) return null
        val both = Intent.FLAG_GRANT_READ_URI_PERMISSION or Intent.FLAG_GRANT_WRITE_URI_PERMISSION
        // Take read and write when both were granted, else at least read; neither when the grant isn't persistable.
        runCatching { resolver.takePersistableUriPermission(uri, both) }
            .recoverCatching { resolver.takePersistableUriPermission(uri, Intent.FLAG_GRANT_READ_URI_PERMISSION) }
        val granted = app.checkUriPermission(uri, Process.myPid(), Process.myUid(), Intent.FLAG_GRANT_WRITE_URI_PERMISSION) ==
            PackageManager.PERMISSION_GRANTED
        return uri.takeIf { granted && supportsWrite(uri) }
    }

    /** Whether the provider lets this document be written (a document URI says so in its flags). */
    private fun supportsWrite(uri: Uri): Boolean {
        if (!DocumentsContract.isDocumentUri(app, uri)) return true
        val flags = runCatching {
            resolver.query(uri, arrayOf(DocumentsContract.Document.COLUMN_FLAGS), null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) cursor.getInt(0) else 0
            }
        }.getOrNull() ?: 0
        return flags and DocumentsContract.Document.FLAG_SUPPORTS_WRITE != 0
    }

    /** The exporter for [request] is about to open. */
    internal fun beginExport(request: FileRequest.Export) {
        exporting = request.bytes
    }

    /** The exporter closed: write the waiting bytes to [uri], or drop them when cancelled. */
    internal fun answerExport(uri: Uri?) {
        val bytes = exporting ?: return
        exporting = null
        if (uri == null) return
        scope.launch {
            withContext(Dispatchers.IO) {
                runCatching {
                    // "wt": a document the user picked to overwrite must not keep a longer old tail.
                    resolver.openOutputStream(uri, "wt")?.use { it.write(bytes) } ?: error("no stream")
                }.onFailure { Log.e(TAG, "writing $uri", it) }
            }
        }
    }

    /**
     * Add an exported image to the device's photos: `Pictures/<app name>` through MediaStore, which needs
     * no permission for the app's own files (API 29+; [canSaveToPhotos]).
     */
    internal fun saveToPhotos(request: FileRequest.Export) {
        if (Build.VERSION.SDK_INT < 29) return
        val album = app.applicationInfo.loadLabel(app.packageManager).toString()
        scope.launch {
            val saved = withContext(Dispatchers.IO) {
                runCatching { insertPhoto(request, album) }.onFailure { Log.e(TAG, "saving ${request.name} to Photos", it) }.isSuccess
            }
            val message = if (saved) "Saved to Photos (Pictures/$album)" else "Couldn't save “${request.name}” to Photos"
            Toast.makeText(app, message, Toast.LENGTH_SHORT).show()
        }
    }

    @RequiresApi(29)
    private fun insertPhoto(request: FileRequest.Export, album: String) {
        val values = ContentValues().apply {
            put(MediaStore.Images.Media.DISPLAY_NAME, request.name)
            put(MediaStore.Images.Media.MIME_TYPE, request.mimeType)
            put(MediaStore.Images.Media.RELATIVE_PATH, "${Environment.DIRECTORY_PICTURES}/$album")
            // Hidden from other apps until it's whole.
            put(MediaStore.Images.Media.IS_PENDING, 1)
        }
        val collection = MediaStore.Images.Media.getContentUri(MediaStore.VOLUME_EXTERNAL_PRIMARY)
        val uri = resolver.insert(collection, values) ?: error("MediaStore made no entry")
        runCatching {
            resolver.openOutputStream(uri, "w")?.use { it.write(request.bytes) } ?: error("no stream")
            resolver.update(uri, ContentValues().apply { put(MediaStore.Images.Media.IS_PENDING, 0) }, null, null)
        }.onFailure { resolver.delete(uri, null, null) }.getOrThrow()
        Log.i(TAG, "saved ${request.name} to $uri")
    }

    /** Whether [request] can go to Photos: an image MediaStore shows, on API 29+ (no storage permission then). */
    internal fun canSaveToPhotos(request: FileRequest.Export): Boolean = Build.VERSION.SDK_INT >= 29 && FileTypes.isPhoto(request.name)

    /** A document's display name, contents and writable location, or `null` when it can't be read. */
    private fun read(uri: Uri): OpenedFile? = runCatching {
        val bytes = resolver.openInputStream(uri)?.use { it.readBytes() } ?: error("no stream")
        OpenedFile(displayName(uri), bytes, writableLocation(uri))
    }.onFailure { Log.e(TAG, "reading $uri", it) }.getOrNull()

    private fun displayName(uri: Uri): String {
        val named = runCatching {
            resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { cursor ->
                if (cursor.moveToFirst()) cursor.getString(0) else null
            }
        }.getOrNull()
        return named ?: uri.lastPathSegment?.substringAfterLast('/') ?: "Untitled"
    }

    private companion object {
        const val TAG = "craftkit"
        const val CLIPBOARD_DIR = "craftkit-clipboard"
        const val SHARE_DIR = "craftkit-share"

        /** The host the app reports to: the newest one (main thread only). */
        var current: CraftHost? = null

        fun documentsOf(intent: Intent): List<Uri> = when (intent.action) {
            Intent.ACTION_VIEW, Intent.ACTION_EDIT -> listOfNotNull(intent.data)
            Intent.ACTION_SEND -> listOfNotNull(intent.streamExtra())
            Intent.ACTION_SEND_MULTIPLE -> intent.streamExtras()
            else -> emptyList()
        }

        @Suppress("DEPRECATION") // The typed overloads are API 33+.
        fun Intent.streamExtra(): Uri? = if (Build.VERSION.SDK_INT >= 33) {
            getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
        } else {
            getParcelableExtra(Intent.EXTRA_STREAM)
        }

        @Suppress("DEPRECATION")
        fun Intent.streamExtras(): List<Uri> = if (Build.VERSION.SDK_INT >= 33) {
            getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java)
        } else {
            getParcelableArrayListExtra(Intent.EXTRA_STREAM)
        }.orEmpty()
    }
}
