//! The Keyboard Shortcuts dialog's words, and what it says in one pass.

craft_ui::words! {
    /// The Keyboard Shortcuts dialog's words.
    pub struct ShortcutWords {
        // Window and footer.
        keyboard_shortcuts: "keyboardShortcuts.keyboardShortcuts" = "Keyboard Shortcuts";
        custom: "keyboardShortcuts.custom" = "Custom";
        ok: "keyboardShortcuts.ok" = "OK";
        cancel: "keyboardShortcuts.cancel" = "Cancel";
        close: "keyboardShortcuts.close" = "Close";
        undo: "keyboardShortcuts.undo" = "Undo";
        redo: "keyboardShortcuts.redo" = "Redo";
        reset_all_more: "keyboardShortcuts.resetAllMore" = "Reset All…";
        restore_every_default_shortcut_in_this: "keyboardShortcuts.restoreEveryDefaultShortcutInThis" = "Restore every default shortcut in this set?";
        reset_all: "keyboardShortcuts.resetAll" = "Reset All";
        // Header.
        show: "keyboardShortcuts.show" = "Show:";
        filter: "keyboardShortcuts.filter" = "Filter:";
        search: "keyboardShortcuts.search" = "Search:";
        search_by_key: "keyboardShortcuts.searchByKey" = "Search by key";
        all_commands: "keyboardShortcuts.allCommands" = "All Commands";
        menu_menu: template "keyboardShortcuts.menuMenu" = "{menu} Menu";
        tools: "keyboardShortcuts.tools" = "Tools";
        panel_panel: template "keyboardShortcuts.panelPanel" = "{panel} Panel";
        other: "keyboardShortcuts.other" = "Other";
        fixed_keys: "keyboardShortcuts.fixedKeys" = "Fixed Keys";
        all: "keyboardShortcuts.all" = "All";
        with_shortcut: "keyboardShortcuts.withShortcut" = "With Shortcut";
        changed: "keyboardShortcuts.changed" = "Changed";
        conflicts: "keyboardShortcuts.conflicts" = "Conflicts";
        no_matching_commands: "keyboardShortcuts.noMatchingCommands" = "No matching commands";
        cycles: "keyboardShortcuts.cycles" = "(cycles)";
        label_cycles: template "keyboardShortcuts.labelCycles" = "{label} (cycles)";
        shares_a_shortcut: "keyboardShortcuts.sharesAShortcut" = "Shares a shortcut with another command";
        label_changed: template "keyboardShortcuts.labelChanged" = "{label} (Changed)";
        // Table and detail pane.
        command: "keyboardShortcuts.command" = "Command";
        commands: "keyboardShortcuts.commands" = "Commands";
        detail_pane_size: "keyboardShortcuts.detailPaneSize" = "Detail pane size";
        select_a_command_to_see_and: "keyboardShortcuts.selectACommandToSeeAnd" = "Select a command to see and change its shortcuts";
        shortcut: "keyboardShortcuts.shortcut" = "Shortcut";
        default: "keyboardShortcuts.default" = "Default";
        chord_default: template "keyboardShortcuts.chordDefault" = "{chord} (Default)";
        chord_scope: template "keyboardShortcuts.chordScope" = "{chord} ({scope})";
        label_scope: template "keyboardShortcuts.labelScope" = "{label} ({scope})";
        application: "keyboardShortcuts.application" = "Application";
        current_shortcuts: "keyboardShortcuts.currentShortcuts" = "Current shortcuts:";
        new_shortcut: "keyboardShortcuts.newShortcut" = "New shortcut:";
        new_shortcut_chord: template "keyboardShortcuts.newShortcutChord" = "New shortcut: {chord}";
        press_keys: "keyboardShortcuts.pressKeys" = "press keys";
        chord_press_again_for_a_double: template "keyboardShortcuts.chordPressAgainForADouble" = "{chord} (press again for a double press)";
        type_more: "keyboardShortcuts.typeMore" = "Type…";
        context: "keyboardShortcuts.context" = "Context:";
        assign: "keyboardShortcuts.assign" = "Assign";
        use_default: "keyboardShortcuts.useDefault" = "Use Default";
        clear: "keyboardShortcuts.clear" = "Clear";
        default_chord: template "keyboardShortcuts.defaultChord" = "Default: {chord}";
        none: "keyboardShortcuts.none" = "None";
        currently_assigned_to: "keyboardShortcuts.currentlyAssignedTo" = "Currently assigned to:";
        not_assigned: "keyboardShortcuts.notAssigned" = "Not assigned";
        label_scope_both_work: template "keyboardShortcuts.labelScopeBothWork" = "{label} ({scope}, both work)";
        add_a_shortcut_to_label: template "keyboardShortcuts.addAShortcutToLabel" = "Add a shortcut to {label}";
        remove_chord_from_label: template "keyboardShortcuts.removeChordFromLabel" = "Remove {chord} from {label}";
        restore_the_default_for_label: template "keyboardShortcuts.restoreTheDefaultForLabel" = "Restore the default for {label}";
        label_chord: template "keyboardShortcuts.labelChord" = "{label}: {chord}";
        label_press_keys: template "keyboardShortcuts.labelPressKeys" = "{label}: press keys";
        now_used_by_label: template "keyboardShortcuts.nowUsedByLabel" = "Now used by {label}";
        label_chord_now_used_by_holder: template "keyboardShortcuts.labelChordNowUsedByHolder" = "{label}: {chord} (Now used by {holder})";
        // Typed shortcuts and saved texts that do not parse.
        the_shortcut_has_no_key: "keyboardShortcuts.theShortcutHasNoKey" = "the shortcut has no key";
        unknown_modifier_key: template "keyboardShortcuts.unknownModifierKey" = "unknown modifier {key}";
        modifier_key_given_twice: template "keyboardShortcuts.modifierKeyGivenTwice" = "modifier {key} given twice";
        key_sequences_are_not_supported: "keyboardShortcuts.keySequencesAreNotSupported" = "key sequences are not supported";
        the_scope_name_is_not_valid: "keyboardShortcuts.theScopeNameIsNotValid" = "the scope name is not valid";
        // On-screen keyboard.
        keyboard: "keyboardShortcuts.keyboard" = "Keyboard";
        keyboard_us: "keyboardShortcuts.keyboardUs" = "Keyboard: US";
        keyboard_us_macos: "keyboardShortcuts.keyboardUsMacos" = "Keyboard: US (macOS)";
        control: "keyboardShortcuts.control" = "Control";
        option: "keyboardShortcuts.option" = "Option";
        shift: "keyboardShortcuts.shift" = "Shift";
        ctrl: "keyboardShortcuts.ctrl" = "Ctrl";
        alt: "keyboardShortcuts.alt" = "Alt";
        caps_lock: "keyboardShortcuts.capsLock" = "Caps Lock";
        win: "keyboardShortcuts.win" = "Win";
        super_key: "keyboardShortcuts.super" = "Super";
        panel_or_context: "keyboardShortcuts.panelOrContext" = "Panel or context";
        both: "keyboardShortcuts.both" = "Both";
        modifier_on: "keyboardShortcuts.modifierOn" = "Modifier on";
        chord_label: template "keyboardShortcuts.chordLabel" = "{chord}: {label}";
        click_a_key_to_see_its: "keyboardShortcuts.clickAKeyToSeeIts" = "Click a key to see its shortcuts";
        key_key: template "keyboardShortcuts.keyKey" = "Key: {key}";
        assign_to_selected: "keyboardShortcuts.assignToSelected" = "Assign to Selected";
        assign_chord_to_label: template "keyboardShortcuts.assignChordToLabel" = "Assign {chord} to {label}";
        select_a_command_first: "keyboardShortcuts.selectACommandFirst" = "Select a command first";
        nothing_uses_this_key: "keyboardShortcuts.nothingUsesThisKey" = "Nothing uses this key";
        // Messages and conflicts.
        chord_is_used_by_label: template "keyboardShortcuts.chordIsUsedByLabel" = "{chord} is used by {label}";
        chord_was_removed_from_label: template "keyboardShortcuts.chordWasRemovedFromLabel" = "{chord} was removed from {label}";
        also_used_by_label: template "keyboardShortcuts.alsoUsedByLabel" = "Also used by {label}";
        chord_is_reserved_by_the_system: template "keyboardShortcuts.chordIsReservedByTheSystem" = "{chord} is reserved by the system";
        a_command_can_have_at_most: template "keyboardShortcuts.aCommandCanHaveAtMost" = "A command can have at most {n} shortcuts";
        the_scope_scope_is_unknown: template "keyboardShortcuts.theScopeScopeIsUnknown" = "The scope {scope} is unknown";
        too_many_shortcuts_are_assigned: "keyboardShortcuts.tooManyShortcutsAreAssigned" = "Too many shortcuts are assigned";
        this_command_is_unknown: "keyboardShortcuts.thisCommandIsUnknown" = "This command is unknown";
        take_it: "keyboardShortcuts.takeIt" = "Take It";
        accept_and_go_to_conflict: "keyboardShortcuts.acceptAndGoToConflict" = "Accept and Go to Conflict";
        keep_both: "keyboardShortcuts.keepBoth" = "Keep Both";
        go_to_conflict: "keyboardShortcuts.goToConflict" = "Go to Conflict";
        undo_changes: "keyboardShortcuts.undoChanges" = "Undo Changes";
        dismiss: "keyboardShortcuts.dismiss" = "Dismiss";
        // Saved shortcuts that could not be used.
        key_no_command_has_this_name: template "keyboardShortcuts.keyNoCommandHasThisName" = "{key}: no command has this name";
        key_text_is_not_a_shortcut: template "keyboardShortcuts.keyTextIsNotAShortcut" = "{key}: “{text}” is not a shortcut ({error})";
        key_text_is_reserved_by_the: template "keyboardShortcuts.keyTextIsReservedByThe" = "{key}: {text} is reserved by the system";
        key_the_scope_in_text_is: template "keyboardShortcuts.keyTheScopeInTextIs" = "{key}: the scope in “{text}” is unknown";
        key_listed_more_than_once: template "keyboardShortcuts.keyListedMoreThanOnce" = "{key}: listed more than once";
        key_part_of_the_entry_was: template "keyboardShortcuts.keyPartOfTheEntryWas" = "{key}: part of the entry was dropped";
        // Sets.
        set: "keyboardShortcuts.set" = "Set:";
        modified: "keyboardShortcuts.modified" = "(modified)";
        new_set_more: "keyboardShortcuts.newSetMore" = "New Set…";
        rename_more: "keyboardShortcuts.renameMore" = "Rename…";
        delete: "keyboardShortcuts.delete" = "Delete";
        save: "keyboardShortcuts.save" = "Save";
        revert_set: "keyboardShortcuts.revertSet" = "Revert Set";
        import_more: "keyboardShortcuts.importMore" = "Import…";
        export: "keyboardShortcuts.export" = "Export";
        export_more: "keyboardShortcuts.exportMore" = "Export…";
        summarize_more: "keyboardShortcuts.summarizeMore" = "Summarize…";
        title_set: template "keyboardShortcuts.titleSet" = "{title}: {set}";
        create: "keyboardShortcuts.create" = "Create";
        rename: "keyboardShortcuts.rename" = "Rename";
        more: "keyboardShortcuts.more" = "More";
        name: "keyboardShortcuts.name" = "Name:";
        based_on: "keyboardShortcuts.basedOn" = "Based on:";
        current_shortcuts_heading: "keyboardShortcuts.currentShortcutsHeading" = "Current shortcuts";
        save_changes_to_set: template "keyboardShortcuts.saveChangesToSet" = "Save changes to “{set}”?";
        dont_save: "keyboardShortcuts.dontSave" = "Don't Save";
        delete_the_set_set: template "keyboardShortcuts.deleteTheSetSet" = "Delete the set “{set}”?";
        keyboard_shortcuts_json: "keyboardShortcuts.keyboardShortcutsJson" = "Keyboard Shortcuts (JSON)";
        the_name_is_empty_too_long: "keyboardShortcuts.theNameIsEmptyTooLong" = "The name is empty, too long or has control characters";
        a_set_with_this_name_exists: "keyboardShortcuts.aSetWithThisNameExists" = "A set with this name exists";
        the_set_was_not_found: "keyboardShortcuts.theSetWasNotFound" = "The set was not found";
        built_in_sets_cannot_be_changed: "keyboardShortcuts.builtInSetsCannotBeChanged" = "Built-in sets cannot be changed";
        there_are_too_many_sets: "keyboardShortcuts.thereAreTooManySets" = "There are too many sets";
        this_is_not_a_shortcut_set: "keyboardShortcuts.thisIsNotAShortcutSet" = "This is not a shortcut set file";
        this_is_not_a_craft_keymap: "keyboardShortcuts.thisIsNotACraftKeymap" = "This is not a craft-keymap file";
        this_file_is_from_a_newer: template "keyboardShortcuts.thisFileIsFromANewer" = "This file is from a newer version (format {n})";
        the_file_is_too_large: "keyboardShortcuts.theFileIsTooLarge" = "The file is too large";
        // Import review.
        import_set: template "keyboardShortcuts.importSet" = "Import “{set}”";
        set_name: "keyboardShortcuts.setName" = "Set name:";
        import: "keyboardShortcuts.import" = "Import";
        cancel_import: "keyboardShortcuts.cancelImport" = "Cancel Import";
        line_n: template "keyboardShortcuts.lineN" = "line {n}";
        will_change: "keyboardShortcuts.willChange" = "Will change";
        not_matched: "keyboardShortcuts.notMatched" = "Not matched";
        unreadable: "keyboardShortcuts.unreadable" = "Unreadable";
        approximated: "keyboardShortcuts.approximated" = "Approximated";
        title_n: template "keyboardShortcuts.titleN" = "{title} ({n})";
        suggestions_n: template "keyboardShortcuts.suggestionsN" = "Suggestions ({n})";
        numeric_keypad_key_read_as_main: "keyboardShortcuts.numericKeypadKeyReadAsMain" = "numeric keypad key read as main key";
        matched_without_change: "keyboardShortcuts.matchedWithoutChange" = "Matched without change";
        command_for_item: template "keyboardShortcuts.commandForItem" = "Command for {item}";
        choose_a_command_more: "keyboardShortcuts.chooseACommandMore" = "Choose a command…";
        no_command_has_this_name: "keyboardShortcuts.noCommandHasThisName" = "no command has this name";
        more_than_one_command_fits: "keyboardShortcuts.moreThanOneCommandFits" = "more than one command fits";
        in_a_context_that_is_not: "keyboardShortcuts.inAContextThatIsNot" = "in a context that is not imported";
        only_a_command_of_the_other: "keyboardShortcuts.onlyACommandOfTheOther" = "only a command of the other kind fits (tool or menu command)";
        empty_shortcut: "keyboardShortcuts.emptyShortcut" = "empty shortcut";
        the_shortcut_is_too_long: "keyboardShortcuts.theShortcutIsTooLong" = "the shortcut is too long";
        unknown_key_key: template "keyboardShortcuts.unknownKeyKey" = "unknown key {key}";
        unsupported_modifier_key: template "keyboardShortcuts.unsupportedModifierKey" = "unsupported modifier {key}";
        label_keeps_its_shortcuts_matched_by: template "keyboardShortcuts.labelKeepsItsShortcutsMatchedBy" = "{label} keeps its shortcuts (matched by a panel menu name)";
        import_as_new_set: "keyboardShortcuts.importAsNewSet" = "Import as New Set";
        imported: "keyboardShortcuts.imported" = "Imported";
        imported_as_set: template "keyboardShortcuts.importedAsSet" = "Imported as “{set}”";
        skipped: "keyboardShortcuts.skipped" = "Skipped";
        taken_from_other_commands: "keyboardShortcuts.takenFromOtherCommands" = "Taken from other commands";
        not_changed: "keyboardShortcuts.notChanged" = "Not changed";
        taken_by_label: template "keyboardShortcuts.takenByLabel" = "taken by {label}";
        taken_by_a_later_entry: "keyboardShortcuts.takenByALaterEntry" = "taken by a later entry";
        reserved_by_the_system: "keyboardShortcuts.reservedByTheSystem" = "reserved by the system";
        the_scope_is_unknown: "keyboardShortcuts.theScopeIsUnknown" = "the scope is unknown";
        the_sets_shortcuts_could_not_be: "keyboardShortcuts.theSetsShortcutsCouldNotBe" = "the set's shortcuts could not be added";
        the_shortcuts_could_not_be_removed: "keyboardShortcuts.theShortcutsCouldNotBeRemoved" = "the shortcuts could not be removed";
        copy_report: "keyboardShortcuts.copyReport" = "Copy Report";
        done: "keyboardShortcuts.done" = "Done";
        // Why a `.kys` file is refused (`KysError`), and the tooltip of a saved text that is not a shortcut.
        the_file_is_larger_than_a: "keyboardShortcuts.theFileIsLargerThanA" = "The file is larger than a keyboard shortcut set can be.";
        the_file_is_not_a_photoshop: template "keyboardShortcuts.theFileIsNotAPhotoshop" = "The file is not a Photoshop keyboard shortcut set: {message}";
        this_saved_text_is_not_a: "keyboardShortcuts.thisSavedTextIsNotA" = "This saved text is not a shortcut and is not used.";
        count_clashes: count "shortcuts.clashes" = "{n} shortcut is assigned more than once" | "{n} shortcuts are assigned more than once";
        count_other_scope: count "shortcuts.otherScope" = "{n} shortcut also works in another scope" | "{n} shortcuts also work in another scope";
        count_unusable: count "shortcuts.unusable" = "{n} saved shortcut could not be used" | "{n} saved shortcuts could not be used";
        count_ignored: count "shortcuts.ignored" = "{n} entry was ignored: too many commands" | "{n} entries were ignored: too many commands";
        count_will_change: count "shortcuts.willChange" = "{n} command will change" | "{n} commands will change";
        count_not_matched: count "shortcuts.notMatched" = "{n} item not matched" | "{n} items not matched";
        count_without_shortcut: count "shortcuts.withoutShortcut" = "{n} item without a shortcut" | "{n} items without a shortcut";
        count_unreadable: count "shortcuts.unreadable" = "{n} item could not be read" | "{n} items could not be read";
        count_approximate: count "shortcuts.approximate" = "{n} key read approximately" | "{n} keys read approximately";
        count_cut: count "shortcuts.cut" = "{n} entry cut at a limit" | "{n} entries cut at a limit";
        count_changed: count "shortcuts.changed" = "{n} command changed" | "{n} commands changed";
    }
}

use std::borrow::Cow;
use std::sync::Arc;

use craft_host::Host;
use craft_keymap::import::kys::KysError;
use craft_keymap::{CommandDef, Key, Platform, Trigger};
use craft_ui::display::{ChordWrite, DisplayWords, Kbd};
use craft_ui::runtime::env::Env;

/// What the dialog says in one pass: its words, the Host's command labels and the interface direction.
pub(crate) struct Say {
    words: Arc<ShortcutWords>,
    keys: Arc<DisplayWords>,
    host: Option<Arc<Host>>,
    rtl: bool,
}

impl Say {
    /// The words of `ui`'s scope, the app's `Host` and the scope's direction.
    pub(crate) fn of(ui: &egui::Ui) -> Say {
        let env = Env::of(ui);
        let view = env.view();
        Say { words: view.words(), keys: view.words(), host: view.host().cloned(), rtl: view.direction().is_rtl() }
    }

    /// The dialog's own words.
    pub(crate) fn words(&self) -> &ShortcutWords {
        &self.words
    }

    /// A command's label.
    pub(crate) fn command(&self, b: &CommandDef) -> String {
        match &self.host {
            Some(host) => host.commands().label(&b.command.id, &b.label),
            None => b.label.clone(),
        }
    }

    /// A menu path part or a scope name: the Host's catalog row for it, if any.
    pub(crate) fn segment<'a>(&'a self, s: &'a str) -> Cow<'a, str> {
        match self.host.as_deref().and_then(|host| Some(host.i18n()?.t(host.lang(), s))) {
            Some(text) => text.into(),
            None => s.into(),
        }
    }

    /// Extra lines for a command's tooltip.
    pub(crate) fn hint(&self, b: &CommandDef) -> Option<String> {
        self.host.as_ref()?.commands().hint(&b.command.id)
    }

    /// A trigger as the platform shows it, with this language's key names.
    pub(crate) fn trigger(&self, trigger: &Trigger, platform: Platform) -> String {
        trigger_text(&self.keys, trigger, platform)
    }

    /// A key's name for `platform`, in this language.
    pub(crate) fn key_word(&self, k: Key, platform: Platform) -> String {
        Kbd::key_name(k, platform, &self.keys)
    }

    /// Whether the interface is drawn right to left.
    pub(crate) fn rtl(&self) -> bool {
        self.rtl
    }
}

/// A trigger as `platform` shows it, with key names from `words`. Modifiers come in the order of
/// `Chord::display`; a double press is written twice.
pub fn trigger_text(words: &DisplayWords, trigger: &Trigger, platform: Platform) -> String {
    Kbd::text(trigger, platform, words, ChordWrite::default())
}

/// `s` with `chord`, text inside it, left as it is. Right to left, the parts before and after the chord
/// change sides.
pub(crate) fn around_chord(rtl: bool, s: &str, chord: &str) -> String {
    let Some(i) = s.find(chord).filter(|_| !chord.is_empty()) else { return s.to_string() };
    let (before, rest) = s.split_at(i);
    let after = &rest[chord.len()..];
    if rtl { format!("{after}{chord}{before}") } else { format!("{before}{chord}{after}") }
}

/// The sentence for a refused `.kys` file.
pub fn kys_error_text(e: &KysError, w: &ShortcutWords) -> String {
    match e {
        KysError::TooLarge => w.the_file_is_larger_than_a.as_str().to_owned(),
        KysError::NotASet(message) => w.the_file_is_not_a_photoshop.fill(&[("message", message.as_str())]),
        other => other.to_string(),
    }
}

impl Say {
    /// A command's label, or its id when the keymap does not hold it.
    pub(crate) fn label_of(&self, keymap: &craft_keymap::Keymap, command: &craft_keymap::CommandRef) -> String {
        keymap.bindable(command).map_or_else(|| command.id.clone(), |b| self.command(b))
    }

    /// A scope's name: Application, else the Host's row for the scope.
    pub(crate) fn scope_name(&self, scope: &craft_keymap::Scope) -> String {
        if scope.is_app() { self.words.application.as_str().to_owned() } else { self.segment(scope.as_str()).into_owned() }
    }
}
