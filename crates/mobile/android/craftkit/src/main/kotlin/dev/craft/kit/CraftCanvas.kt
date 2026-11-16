package dev.craft.kit

import android.content.Context
import android.content.Intent
import android.net.Uri
import android.util.Log
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContract
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.viewinterop.AndroidView
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.CompletableDeferred

/**
 * The app's canvas: a view the Rust app draws into, with touches, the stylus, mouse, keyboard
 * and on-screen keyboard passed on. It also shows the pickers and exporters the app asks for and
 * tells the app when it leaves the screen, so a shell needs nothing else.
 */
@Composable
fun CraftCanvas(host: CraftHost, modifier: Modifier = Modifier) {
    var view by remember { mutableStateOf<CraftSurfaceView?>(null) }
    AndroidView(factory = { CraftSurfaceView(it, host).also { created -> view = created } }, modifier = modifier)
    val keyboard by host.keyboard.collectAsStateWithLifecycle()
    LaunchedEffect(view, keyboard) { view?.showKeyboard(keyboard) }
    LifeCycle(host)
    FileRequests(host)
}

/**
 * Tells the app when it leaves the screen and comes back. `ON_STOP` waits for the app's suspend:
 * once `onStop` returns, Android may freeze or end the process, and the recovery copies of
 * unsaved work must be on disk by then.
 */
@Composable
private fun LifeCycle(host: CraftHost) {
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    DisposableEffect(lifecycle, host) {
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_START -> host.resume()
                Lifecycle.Event.ON_STOP -> host.suspend()
                else -> Unit
            }
        }
        lifecycle.addObserver(observer)
        onDispose { lifecycle.removeObserver(observer) }
    }
}

/**
 * Answers the app's file requests with the system's document UI (Storage Access Framework), the Photo
 * Picker and the share sheet, and offers an exported image Save to Photos too. Only while the app is on
 * screen: a request made in the background (an export that finished there) waits for the user to come back.
 */
@Composable
private fun FileRequests(host: CraftHost) {
    val pickOne = rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { host.answerPick(listOfNotNull(it)) }
    val pickMany = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) { host.answerPick(it) }
    val create = rememberLauncherForActivityResult(CreateTypedDocument()) { host.answerExport(it) }
    val photos = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia()) { host.openPhotos(it) }
    val context = LocalContext.current
    var choosing by remember { mutableStateOf<ExportChoice?>(null) }
    val lifecycle = LocalLifecycleOwner.current.lifecycle
    LaunchedEffect(host, lifecycle) {
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            for (request in host.fileRequests) {
                when (request) {
                    is FileRequest.Pick -> if (request.multiple) pickMany.launch(request.mimeTypes) else pickOne.launch(request.mimeTypes)
                    is FileRequest.PickPhotos -> photos.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageOnly))
                    is FileRequest.Share -> host.shareIntent(request)?.let { chooser ->
                        runCatching { context.startActivity(chooser) }.onFailure { Log.w("craftkit", "no share sheet", it) }
                    }
                    is FileRequest.Export -> {
                        val toPhotos = if (host.canSaveToPhotos(request)) {
                            // Cancelled: the bytes are dropped, as a cancelled exporter drops them.
                            ExportChoice(request).also { choosing = it }.answer.await() ?: continue
                        } else {
                            false
                        }
                        if (toPhotos) {
                            host.saveToPhotos(request)
                        } else {
                            host.beginExport(request)
                            create.launch(request)
                        }
                    }
                }
            }
        }
    }
    choosing?.let { choice ->
        val answer = { toPhotos: Boolean? ->
            choosing = null
            choice.answer.complete(toPhotos)
        }
        AlertDialog(
            onDismissRequest = { answer(null) },
            title = { Text(choice.request.name) },
            text = { Text("Save the image to a folder, or add it to your photos.") },
            confirmButton = { TextButton(onClick = { answer(true) }) { Text("Save to Photos") } },
            dismissButton = { TextButton(onClick = { answer(false) }) { Text("Save to Files") } },
        )
    }
}

/** An image export waiting for Save to Files (`false`), Save to Photos (`true`) or cancel (`null`). */
private class ExportChoice(val request: FileRequest.Export) {
    val answer = CompletableDeferred<Boolean?>()
}

/** `ACTION_CREATE_DOCUMENT` with the MIME type chosen per request (the stock contract fixes it once). */
private class CreateTypedDocument : ActivityResultContract<FileRequest.Export, Uri?>() {
    override fun createIntent(context: Context, input: FileRequest.Export): Intent =
        Intent(Intent.ACTION_CREATE_DOCUMENT)
            .addCategory(Intent.CATEGORY_OPENABLE)
            .setType(input.mimeType)
            .putExtra(Intent.EXTRA_TITLE, input.name)

    override fun parseResult(resultCode: Int, intent: Intent?): Uri? = intent.takeIf { resultCode == android.app.Activity.RESULT_OK }?.data
}
