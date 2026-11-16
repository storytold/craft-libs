package dev.craft.kit

import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ModelTest {
    /** As craft-mobile's `Snapshot` serializes it. */
    @Test
    fun aSnapshotParsesFromTheAppsJson() {
        val json = """
            {"document":{"name":"Untitled-1","detail":"800 × 600 px","dirty":true},"can_undo":true,"can_redo":false,
             "items_title":"Layers","items":[{"id":3,"name":"Background","kind":"Pixel","depth":0,"visible":true,"selected":true},
             {"id":4,"name":"Locked","kind":"Group","depth":1,"visible":null,"selected":false}],"future_field":1}
        """
        val snapshot = Snapshot.parse(json)
        assertEquals(DocumentInfo("Untitled-1", "800 × 600 px", dirty = true), snapshot?.document)
        assertEquals(true, snapshot?.canUndo)
        assertEquals("Layers", snapshot?.itemsTitle)
        assertEquals(listOf(Item(3, "Background", "Pixel", 0, true, true), Item(4, "Locked", "Group", 1, null, false)), snapshot?.items)
        assertNull(Snapshot.parse("not json"))
        assertEquals(Snapshot(itemsTitle = "Layers"), Snapshot.parse("""{"document":null,"items_title":"Layers"}"""))
    }

    /** As craft-mobile's `Action` deserializes it (`#[serde(tag = "action", rename_all = "snake_case")]`). */
    @Test
    fun actionsBecomeTaggedJson() {
        assertEquals("""{"action":"undo"}""", CraftAction.Undo.toJson())
        assertEquals("""{"action":"select","id":7}""", CraftAction.Select(7).toJson())
        assertEquals("""{"action":"set_visible","id":7,"visible":false}""", CraftAction.SetVisible(7, false).toJson())
        assertEquals("""{"action":"command","id":"file.new","params":null}""", CraftAction.Command("file.new").toJson())
        val params = buildJsonObject { put("width", 800) }
        assertEquals("""{"action":"command","id":"file.new","params":{"width":800}}""", CraftAction.Command("file.new", params).toJson())
    }
}
