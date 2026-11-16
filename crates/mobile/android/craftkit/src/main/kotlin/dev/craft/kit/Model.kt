package dev.craft.kit

import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonElement
import kotlinx.serialization.json.JsonNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/** What the native panels show: craft-mobile's `Snapshot`, the same for every Craft app. */
@Serializable
data class Snapshot(
    val document: DocumentInfo? = null,
    @SerialName("can_undo") val canUndo: Boolean = false,
    @SerialName("can_redo") val canRedo: Boolean = false,
    /** What the items are called in this app: "Layers", "Tracks", "Pages". */
    @SerialName("items_title") val itemsTitle: String = "",
    /** The active document's items, top to bottom. */
    val items: List<Item> = emptyList(),
) {
    companion object {
        private val json = Json { ignoreUnknownKeys = true }

        /** The app's panel state, or `null` when [text] isn't one. */
        fun parse(text: String): Snapshot? = runCatching { json.decodeFromString(serializer(), text) }.getOrNull()
    }
}

@Serializable
data class DocumentInfo(val name: String = "", val detail: String = "", val dirty: Boolean = false)

/** One row of the items panel. [visible] is `null` when the item can't be hidden. */
@Serializable
data class Item(
    val id: Long,
    val name: String = "",
    val kind: String = "",
    val depth: Int = 0,
    val visible: Boolean? = null,
    val selected: Boolean = false,
)

/** What a native control asks of the app: craft-mobile's `Action`. */
sealed interface CraftAction {
    data object Open : CraftAction
    data object Save : CraftAction
    data object Undo : CraftAction
    data object Redo : CraftAction
    data class Select(val id: Long) : CraftAction
    data class SetVisible(val id: Long, val visible: Boolean) : CraftAction

    /** One of the app's own commands; `null` [params] run it as its menu item does (dialogs included). */
    data class Command(val id: String, val params: JsonElement = JsonNull) : CraftAction

    fun toJson(): String = buildJsonObject {
        when (val action = this@CraftAction) {
            Open -> put("action", "open")
            Save -> put("action", "save")
            Undo -> put("action", "undo")
            Redo -> put("action", "redo")
            is Select -> {
                put("action", "select")
                put("id", action.id)
            }
            is SetVisible -> {
                put("action", "set_visible")
                put("id", action.id)
                put("visible", action.visible)
            }
            is Command -> {
                put("action", "command")
                put("id", action.id)
                put("params", action.params)
            }
        }
    }.toString()
}
