package dev.craft.kit

import android.os.Build
import androidx.activity.compose.BackHandler
import androidx.activity.compose.LocalActivity
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconToggleButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.VerticalDivider
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.dynamicDarkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle

/**
 * Material 3 for a Craft app: dark, as editors around an image are (the canvas reads truer on
 * dark chrome), in the device's own colors where it has them.
 */
@Composable
fun CraftTheme(content: @Composable () -> Unit) {
    val scheme: ColorScheme = if (Build.VERSION.SDK_INT >= 31) dynamicDarkColorScheme(LocalContext.current) else darkColorScheme()
    MaterialTheme(colorScheme = scheme, content = content)
}

/**
 * The whole shell: [CraftToolbar] over [CraftCanvas], with the items panel ([CraftItemsPanel])
 * floating over the canvas's trailing edge. [actions] are the app's own toolbar buttons,
 * shown before the shared ones.
 */
@Composable
fun CraftScaffold(
    host: CraftHost,
    appName: String,
    // Native app bar and items panel. Off by default: an app that draws its own menus and panels
    // would show everything twice.
    chrome: Boolean = false,
    actions: @Composable RowScope.() -> Unit = {},
) {
    var itemsShown by rememberSaveable { mutableStateOf(false) }
    val activity = LocalActivity.current
    // Back at the root leaves the app as Home does, documents and all, never finishing it:
    // an editor must not drop unsaved work on a stray swipe. (The later handler wins.)
    BackHandler { activity?.moveTaskToBack(true) }
    BackHandler(enabled = itemsShown) { itemsShown = false }
    Scaffold(
        topBar = { if (chrome) CraftToolbar(host, appName, itemsShown = itemsShown, onItemsShownChange = { itemsShown = it }, actions = actions) },
        containerColor = MaterialTheme.colorScheme.surfaceContainerLowest,
    ) { padding ->
        // The canvas shrinks above the on-screen keyboard, so the app's dialogs stay in view.
        Box(Modifier.padding(padding).consumeWindowInsets(padding).imePadding().fillMaxSize()) {
            CraftCanvas(host, Modifier.fillMaxSize())
            AnimatedVisibility(
                visible = chrome && itemsShown,
                modifier = Modifier.align(Alignment.TopEnd),
                enter = slideInHorizontally { it / 3 } + fadeIn(),
                exit = slideOutHorizontally { it / 3 } + fadeOut(),
            ) {
                CraftItemsPanel(host, Modifier.padding(12.dp).width(320.dp).fillMaxHeight())
            }
        }
    }
}

/** The app bar: the document's name and size, file actions, undo and redo, and the items panel toggle. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun CraftToolbar(
    host: CraftHost,
    appName: String,
    itemsShown: Boolean,
    onItemsShownChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    actions: @Composable RowScope.() -> Unit = {},
) {
    val snapshot by host.snapshot.collectAsStateWithLifecycle()
    val document = snapshot.document
    TopAppBar(
        modifier = modifier,
        colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.surfaceContainer),
        title = {
            Column {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Text(document?.name ?: appName, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.titleMedium)
                    if (document?.dirty == true) {
                        Spacer(Modifier.width(8.dp))
                        // Unsaved changes, as the desktop tab's dot shows them.
                        Box(Modifier.size(8.dp).clip(CircleShape).background(MaterialTheme.colorScheme.primary))
                    }
                }
                if (document != null) {
                    Text(document.detail, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
        },
        actions = {
            actions()
            ToolbarButton(R.drawable.craft_ic_open, "Open") { host.perform(CraftAction.Open) }
            ToolbarButton(R.drawable.craft_ic_save, "Save", enabled = document != null) { host.perform(CraftAction.Save) }
            ToolbarButton(R.drawable.craft_ic_paste, "Paste") { host.paste() }
            VerticalDivider(Modifier.height(24.dp).padding(horizontal = 8.dp))
            ToolbarButton(R.drawable.craft_ic_undo, "Undo", enabled = snapshot.canUndo) { host.perform(CraftAction.Undo) }
            ToolbarButton(R.drawable.craft_ic_redo, "Redo", enabled = snapshot.canRedo) { host.perform(CraftAction.Redo) }
            VerticalDivider(Modifier.height(24.dp).padding(horizontal = 8.dp))
            IconToggleButton(checked = itemsShown, onCheckedChange = onItemsShownChange) {
                Icon(painterResource(R.drawable.craft_ic_layers), contentDescription = snapshot.itemsTitle.ifEmpty { "Items" })
            }
            Spacer(Modifier.width(8.dp))
        },
    )
}

/** An app bar button with an icon from [icon] (craftkit's `craft_ic_*` or the app's own). */
@Composable
fun ToolbarButton(icon: Int, label: String, enabled: Boolean = true, onClick: () -> Unit) {
    IconButton(onClick = onClick, enabled = enabled) { Icon(painterResource(icon), contentDescription = label) }
}

/** The active document's items (layers, tracks, pages): select one, show or hide it. */
@Composable
fun CraftItemsPanel(host: CraftHost, modifier: Modifier = Modifier) {
    val snapshot by host.snapshot.collectAsStateWithLifecycle()
    Surface(modifier, shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceContainerHigh, shadowElevation = 6.dp) {
        Column {
            Row(Modifier.fillMaxWidth().padding(start = 20.dp, end = 16.dp, top = 16.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(snapshot.itemsTitle.ifEmpty { "Items" }, style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                if (snapshot.items.isNotEmpty()) {
                    Text("${snapshot.items.size}", style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            if (snapshot.document == null) {
                Text(
                    "No document open",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 20.dp, vertical = 12.dp),
                )
            }
            LazyColumn(Modifier.fillMaxWidth().padding(horizontal = 8.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                items(snapshot.items, key = { it.id }) { item -> ItemRow(item, host) }
            }
        }
    }
}

@Composable
private fun ItemRow(item: Item, host: CraftHost) {
    val colors = MaterialTheme.colorScheme
    val background = if (item.selected) colors.secondaryContainer else Color.Transparent
    val content = if (item.selected) colors.onSecondaryContainer else colors.onSurface
    val hidden = item.visible == false
    Row(
        Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(background)
            .clickable(role = Role.Button, onClickLabel = "Select") { host.perform(CraftAction.Select(item.id)) }
            .padding(start = 12.dp + 16.dp * item.depth, end = 4.dp, top = 4.dp, bottom = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f).padding(vertical = 6.dp)) {
            Text(
                item.name,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                style = MaterialTheme.typography.bodyLarge,
                fontWeight = if (item.selected) FontWeight.Medium else FontWeight.Normal,
                color = if (hidden) content.copy(alpha = 0.5f) else content,
            )
            if (item.kind.isNotEmpty()) {
                Text(item.kind, style = MaterialTheme.typography.labelSmall, color = colors.onSurfaceVariant)
            }
        }
        item.visible?.let { visible ->
            IconButton(onClick = { host.perform(CraftAction.SetVisible(item.id, !visible)) }) {
                Icon(
                    painterResource(if (visible) R.drawable.craft_ic_visible else R.drawable.craft_ic_hidden),
                    contentDescription = if (visible) "Hide ${item.name}" else "Show ${item.name}",
                    tint = if (visible) content else colors.onSurfaceVariant.copy(alpha = 0.6f),
                )
            }
        }
    }
}
