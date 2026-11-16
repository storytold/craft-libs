package dev.craft.kit

import java.util.Locale

/** File extensions as the system pickers see them: MIME types. */
object FileTypes {
    /** The registered type first, then the aliases providers also label files with. */
    private val known = mapOf(
        "png" to listOf("image/png"),
        "jpg" to listOf("image/jpeg"),
        "jpeg" to listOf("image/jpeg"),
        "webp" to listOf("image/webp"),
        "gif" to listOf("image/gif"),
        "bmp" to listOf("image/bmp", "image/x-ms-bmp"),
        "tif" to listOf("image/tiff"),
        "tiff" to listOf("image/tiff"),
        "heic" to listOf("image/heic"),
        "heif" to listOf("image/heif"),
        "avif" to listOf("image/avif"),
        "svg" to listOf("image/svg+xml"),
        // MediaStore labels PSDs image/x-photoshop.
        "psd" to listOf("image/vnd.adobe.photoshop", "image/x-photoshop", "application/x-photoshop"),
        "pdf" to listOf("application/pdf"),
        "json" to listOf("application/json"),
        "zip" to listOf("application/zip"),
    )

    /** Every file, for types the system has no MIME type for (an app's own documents). */
    const val ANY = "*/*"

    /** The MIME type for [extension] (no dot, any case), or `null` when the system has none. */
    fun mimeOf(extension: String): String? = aliasesOf(extension)?.first()

    private fun aliasesOf(extension: String): List<String>? = known[extension.trimStart('.').lowercase(Locale.ROOT)]

    /**
     * What the open picker filters by, for these extensions. Empty, or any extension without a
     * MIME type (which the picker would hide), means every file.
     */
    fun openFilter(extensions: List<String>): Array<String> {
        val types = extensions.filter { it.isNotBlank() }.flatMap { aliasesOf(it) ?: return arrayOf(ANY) }
        return if (types.isEmpty()) arrayOf(ANY) else types.distinct().toTypedArray()
    }

    /** Image types the Photos (MediaStore) library shows: not layered documents (PSD), which are images too. */
    private val photos = setOf("png", "jpg", "jpeg", "webp", "gif", "heic", "heif", "avif")

    /** Whether [fileName] is an image to offer Save to Photos for. */
    fun isPhoto(fileName: String): Boolean = fileName.substringAfterLast('.', "").lowercase(Locale.ROOT) in photos

    /** The extension for a [mimeType] (`image/jpeg` → `jpg`), or `null` when none is known. */
    fun extensionOf(mimeType: String): String? =
        known.entries.firstOrNull { (_, types) -> types.first() == mimeType.lowercase(Locale.ROOT) }?.key

    /** The MIME type to create [fileName] with. */
    fun mimeOfName(fileName: String): String {
        val dot = fileName.lastIndexOf('.')
        val extension = if (dot < 0) "" else fileName.substring(dot + 1)
        return mimeOf(extension) ?: "application/octet-stream"
    }
}
