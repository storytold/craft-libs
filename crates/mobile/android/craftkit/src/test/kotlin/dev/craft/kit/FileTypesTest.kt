package dev.craft.kit

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class FileTypesTest {
    @Test
    fun extensionsMapToMimeTypesInAnyCase() {
        assertEquals("image/png", FileTypes.mimeOf("PNG"))
        assertEquals("image/jpeg", FileTypes.mimeOf(".jpg"))
        assertEquals("image/vnd.adobe.photoshop", FileTypes.mimeOf("psd"))
        assertNull(FileTypes.mimeOf("pcraft"))
    }

    @Test
    fun thePickerFiltersByKnownTypesWithoutDuplicates() {
        assertArrayEquals(arrayOf("image/png", "image/jpeg"), FileTypes.openFilter(listOf("png", "jpg", "jpeg")))
    }

    /** Files labelled with any of a type's aliases must show. */
    @Test
    fun thePickerFiltersByEveryAliasOfAType() {
        assertArrayEquals(
            arrayOf("image/vnd.adobe.photoshop", "image/x-photoshop", "application/x-photoshop"),
            FileTypes.openFilter(listOf("psd")),
        )
    }

    /** A type the picker can't name would be hidden, so the picker shows every file instead. */
    @Test
    fun anUnknownOrEmptyListShowsEveryFile() {
        assertArrayEquals(arrayOf("*/*"), FileTypes.openFilter(listOf("png", "pcraft")))
        assertArrayEquals(arrayOf("*/*"), FileTypes.openFilter(listOf("")))
        assertArrayEquals(arrayOf("*/*"), FileTypes.openFilter(emptyList()))
    }

    /** Flat images go to Photos; layered documents and the app's own don't, though PSD is an image type. */
    @Test
    fun onlyFlatImagesAreOfferedSaveToPhotos() {
        assertTrue(FileTypes.isPhoto("Untitled-1.png"))
        assertTrue(FileTypes.isPhoto("Holiday.JPG"))
        assertTrue(FileTypes.isPhoto("a.b.webp"))
        assertFalse(FileTypes.isPhoto("Untitled-1.psd"))
        assertFalse(FileTypes.isPhoto("Untitled-1.pcraft"))
        assertFalse(FileTypes.isPhoto("png"))
    }

    @Test
    fun anExportIsCreatedWithItsNamesType() {
        assertEquals("image/vnd.adobe.photoshop", FileTypes.mimeOfName("Untitled-1.psd"))
        assertEquals("application/octet-stream", FileTypes.mimeOfName("Untitled-1.pcraft"))
        assertEquals("application/octet-stream", FileTypes.mimeOfName("README"))
    }

    @Test
    fun aPhotoIsNamedForItsType() {
        assertEquals("jpg", FileTypes.extensionOf("image/jpeg"))
        assertEquals("png", FileTypes.extensionOf("IMAGE/PNG"))
        assertEquals("heic", FileTypes.extensionOf("image/heic"))
        assertEquals(null, FileTypes.extensionOf("video/mp4"))
    }
}
