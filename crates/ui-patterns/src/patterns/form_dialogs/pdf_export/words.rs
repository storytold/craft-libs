//! The words of the Save PDF dialog, and the wording of craft-pdf's checks and warnings.

use super::summary::{RangeError, option_label};
use crate::runtime::env::{Label, WordTemplate};
use craft_pdf::settings::{SettingsError, SettingsWarning};

craft_ui::words! {
    /// The words of the Save PDF dialog and the wording of craft-pdf's checks and warnings. Each id is prefixed by its
    /// module; a `{name}` stands for the value the text names.
    pub struct PdfExportWords {
        sections: "pdfExportDialog.sections" = "Sections";
        /// `{name}`: the preset.
        preset_modified: template "pdfExportDialog.presetModified" = "{name} (modified)";
        /// The heading of the list of differences from the preset.
        summary_changes: "pdfExportDialog.summaryChanges" = "Differences from the preset";
        title: "pdfExportDialog.title" = "Save PDF";
        save: "pdfExportDialog.save" = "Save PDF";
        preset: "pdfExportDialog.preset" = "Preset";
        /// The name suggested for a new preset.
        new_preset_name: "pdfExportDialog.newPresetName" = "New PDF Preset";
        standard: "pdfExportDialog.standard" = "Standard";
        compatibility: "pdfExportDialog.compatibility" = "Compatibility";
        section_general: "pdfExportDialog.sectionGeneral" = "General";
        section_compression: "pdfExportDialog.sectionCompression" = "Compression";
        section_marks: "pdfExportDialog.sectionMarks" = "Marks and Bleeds";
        section_output: "pdfExportDialog.sectionOutput" = "Output";
        section_advanced: "pdfExportDialog.sectionAdvanced" = "Advanced";
        section_security: "pdfExportDialog.sectionSecurity" = "Security";
        section_summary: "pdfExportDialog.sectionSummary" = "Summary";
        options: "pdfExportDialog.options" = "Options";
        preserve_editing: "pdfExportDialog.preserveEditing" = "Preserve editing capabilities";
        thumbnails: "pdfExportDialog.thumbnails" = "Embed page thumbnails";
        fast_web_view: "pdfExportDialog.fastWebView" = "Optimize for fast web view";
        view_after_saving: "pdfExportDialog.viewAfterSaving" = "View PDF after saving";
        create_layers: "pdfExportDialog.createLayers" = "Create PDF layers from layers and sublayers";
        include_non_printing: "pdfExportDialog.includeNonPrinting" = "Include non-printing layers";
        artboards: "pdfExportDialog.artboards" = "Artboards";
        all: "pdfExportDialog.all" = "All";
        range: "pdfExportDialog.range" = "Range";
        range_hint: "pdfExportDialog.rangeHint" = "1-3, 5";
        color_images: "pdfExportDialog.colorImages" = "Color images";
        gray_images: "pdfExportDialog.grayImages" = "Grayscale images";
        mono_images: "pdfExportDialog.monoImages" = "Monochrome images";
        downsampling: "pdfExportDialog.downsampling" = "Downsampling";
        to: "pdfExportDialog.to" = "to";
        above: "pdfExportDialog.above" = "above";
        compression: "pdfExportDialog.compression" = "Compression";
        quality: "pdfExportDialog.quality" = "Quality";
        compress_text: "pdfExportDialog.compressText" = "Compress text and line art";
        marks: "pdfExportDialog.marks" = "Marks";
        all_marks: "pdfExportDialog.allMarks" = "All printer's marks";
        trim_marks: "pdfExportDialog.trimMarks" = "Trim marks";
        registration_marks: "pdfExportDialog.registrationMarks" = "Registration marks";
        color_bars: "pdfExportDialog.colorBars" = "Color bars";
        page_information: "pdfExportDialog.pageInformation" = "Page information";
        mark_type: "pdfExportDialog.markType" = "Printer mark type";
        mark_weight: "pdfExportDialog.markWeight" = "Trim mark weight";
        mark_offset: "pdfExportDialog.markOffset" = "Offset";
        bleeds: "pdfExportDialog.bleeds" = "Bleeds";
        use_document_bleed: "pdfExportDialog.useDocumentBleed" = "Use document bleed settings";
        top: "pdfExportDialog.top" = "Top";
        bottom: "pdfExportDialog.bottom" = "Bottom";
        left: "pdfExportDialog.left" = "Left";
        right: "pdfExportDialog.right" = "Right";
        color: "pdfExportDialog.color" = "Color";
        color_conversion: "pdfExportDialog.colorConversion" = "Color conversion";
        destination: "pdfExportDialog.destination" = "Destination";
        document_profile: "pdfExportDialog.documentProfile" = "Document profile";
        profile_inclusion: "pdfExportDialog.profileInclusion" = "Profile inclusion";
        note_cmyk_only: "pdfExportDialog.noteCmykOnly" = "PDF/X-1a files are CMYK: every colour is converted to the destination (blank: the output intent's CMYK profile), untagged.";
        note_tagged: "pdfExportDialog.noteTagged" = "Files of this standard tag their colours with ICC profiles.";
        output_intent: "pdfExportDialog.outputIntent" = "Output Intent";
        intent_profile: "pdfExportDialog.intentProfile" = "Output intent profile";
        no_profile: "pdfExportDialog.noProfile" = "None";
        cmyk_in_effect: "pdfExportDialog.cmykInEffect" = "CMYK profile in effect";
        output_condition: "pdfExportDialog.outputCondition" = "Output condition";
        condition_identifier: "pdfExportDialog.conditionIdentifier" = "Condition identifier";
        registry_name: "pdfExportDialog.registryName" = "Registry name";
        trapped: "pdfExportDialog.trapped" = "Mark as trapped";
        note_pdfa_intent: "pdfExportDialog.notePdfaIntent" = "PDF/A files carry their own output intent.";
        fonts: "pdfExportDialog.fonts" = "Fonts";
        subset_below: "pdfExportDialog.subsetBelow" = "Subset fonts below";
        of_characters: "pdfExportDialog.ofCharacters" = "of characters used";
        outline_text: "pdfExportDialog.outlineText" = "Convert text to outlines";
        overprint_flattener: "pdfExportDialog.overprintFlattener" = "Overprint and Transparency Flattener";
        overprint: "pdfExportDialog.overprint" = "Overprint";
        flattener_preset: "pdfExportDialog.flattenerPreset" = "Flattener preset";
        note_flatten: "pdfExportDialog.noteFlatten" = "PDF 1.3 files have no transparency: the preset flattens it.";
        flatten_high: "pdfExportDialog.flattenHigh" = "High Resolution";
        flatten_medium: "pdfExportDialog.flattenMedium" = "Medium Resolution";
        flatten_low: "pdfExportDialog.flattenLow" = "Low Resolution";
        open_password_heading: "pdfExportDialog.openPasswordHeading" = "Document open password";
        require_open_password: "pdfExportDialog.requireOpenPassword" = "Require a password to open the document";
        open_password: "pdfExportDialog.openPassword" = "Open password";
        permissions: "pdfExportDialog.permissions" = "Permissions";
        restrict_permissions: "pdfExportDialog.restrictPermissions" = "Restrict printing, editing and other tasks";
        permissions_password: "pdfExportDialog.permissionsPassword" = "Permissions password";
        printing_allowed: "pdfExportDialog.printingAllowed" = "Printing allowed";
        changes_allowed: "pdfExportDialog.changesAllowed" = "Changes allowed";
        allow_copy: "pdfExportDialog.allowCopy" = "Enable copying of text, images and other content";
        allow_reader: "pdfExportDialog.allowReader" = "Enable text access for screen readers";
        plaintext_metadata: "pdfExportDialog.plaintextMetadata" = "Enable plaintext metadata";
        note_standard_password: "pdfExportDialog.noteStandardPassword" = "Files of a PDF standard can't be password-protected.";
        /// `{name}`: the encryption algorithm.
        encryption_note: template "pdfExportDialog.encryptionNote" = "Encryption: {name} (set by Compatibility)";
        encryption: "pdfExportDialog.encryption" = "Encryption";
        summary_warnings: "pdfExportDialog.summaryWarnings" = "Warnings";
        no_warnings: "pdfExportDialog.noWarnings" = "None.";
        no_encryption: "pdfExportDialog.noEncryption" = "None";
        /// `{app}`: the application's name.
        preset_default: template "pdfExportDialog.presetDefault" = "{app} Default";
        preset_high_quality: "pdfExportDialog.presetHighQuality" = "High Quality Print";
        preset_press: "pdfExportDialog.presetPress" = "Press Quality";
        preset_smallest: "pdfExportDialog.presetSmallest" = "Smallest File Size";
        no_downsampling: "pdfExportDialog.noDownsampling" = "No downsampling";
        average: "pdfExportDialog.average" = "Average";
        subsample: "pdfExportDialog.subsample" = "Subsample";
        bicubic: "pdfExportDialog.bicubic" = "Bicubic";
        none: "pdfExportDialog.none" = "None";
        automatic: "pdfExportDialog.automatic" = "Automatic";
        run_length: "pdfExportDialog.runLength" = "Run Length";
        minimum: "pdfExportDialog.minimum" = "Minimum";
        low: "pdfExportDialog.low" = "Low";
        medium: "pdfExportDialog.medium" = "Medium";
        high: "pdfExportDialog.high" = "High";
        maximum: "pdfExportDialog.maximum" = "Maximum";
        roman: "pdfExportDialog.roman" = "Roman";
        japanese: "pdfExportDialog.japanese" = "Japanese";
        no_conversion: "pdfExportDialog.noConversion" = "No conversion";
        convert_destination: "pdfExportDialog.convertDestination" = "Convert to destination";
        convert_preserve: "pdfExportDialog.convertPreserve" = "Convert to destination (preserve numbers)";
        no_profiles: "pdfExportDialog.noProfiles" = "Don't include profiles";
        all_profiles: "pdfExportDialog.allProfiles" = "Include all profiles";
        destination_profile: "pdfExportDialog.destinationProfile" = "Include destination profile";
        tagged_profiles: "pdfExportDialog.taggedProfiles" = "Include tagged source profiles";
        preserve: "pdfExportDialog.preserve" = "Preserve";
        discard: "pdfExportDialog.discard" = "Discard";
        printing_low: "pdfExportDialog.printingLow" = "Low resolution (150 ppi)";
        printing_high: "pdfExportDialog.printingHigh" = "High resolution";
        changes_pages: "pdfExportDialog.changesPages" = "Inserting, deleting and rotating pages";
        changes_forms: "pdfExportDialog.changesForms" = "Filling in form fields and signing";
        changes_comments: "pdfExportDialog.changesComments" = "Commenting, filling in form fields and signing";
        changes_any: "pdfExportDialog.changesAny" = "Any except extracting pages";
        /// `{standard}`, `{version}`.
        check_standard_version: template "pdfExportDialog.checkStandardVersion" = "{standard} files can't be {version}";
        /// `{standard}`.
        check_standard_editing: template "pdfExportDialog.checkStandardEditing" = "{standard} files can't carry editing data: turn off Preserve editing capabilities";
        /// `{standard}`.
        check_standard_layers: template "pdfExportDialog.checkStandardLayers" = "{standard} files can't have PDF layers: turn off Create PDF layers";
        /// `{standard}`.
        check_standard_password: template "pdfExportDialog.checkStandardPassword" = "{standard} files can't be password-protected: remove the passwords or choose no standard";
        check_passwords_equal: "pdfExportDialog.checkPasswordsEqual" = "The open and permissions passwords must differ";
        /// `{version}`.
        check_low_printing: template "pdfExportDialog.checkLowPrinting" = "{version} can't limit printing to low resolution: choose PDF 1.4 or later, or allow high-resolution printing";
        /// The separator between a section, a group and a setting in a Summary line.
        path_separator: "pdfExportDialog.pathSeparator" = " › ";
        /// `{setting}` and `{value}`: a Summary line for a setting that differs from the preset.
        difference: template "pdfExportDialog.difference" = "{setting}: {value}";
        /// `{field}`: the setting; `{range}`: the allowed values with their unit; `{value}`: the value given.
        check_range: template "pdfExportDialog.checkRange" = "{field} must be {range} (got {value})";
        /// `{name}`: a colour profile.
        check_destination_unknown: template "pdfExportDialog.checkDestinationUnknown" = "Destination: no RGB or CMYK profile is called “{name}”";
        /// `{standard}`, `{name}`.
        check_destination_rgb: template "pdfExportDialog.checkDestinationRgb" = "Destination: {standard} files are CMYK, and “{name}” is an RGB profile";
        /// `{standard}`, `{name}`.
        check_intent_rgb: template "pdfExportDialog.checkIntentRgb" = "Output intent profile: {standard} files print in CMYK, and “{name}” is an RGB profile";
        /// `{standard}`, `{name}`.
        check_intent_gray: template "pdfExportDialog.checkIntentGray" = "Output intent profile: “{name}” is a grey profile, which can't be a {standard} output intent";
        /// `{standard}`, `{name}`.
        check_intent_unknown_x4: template "pdfExportDialog.checkIntentUnknownX4" = "Output intent profile: no profile is called “{name}”, and a {standard} output intent embeds its profile";
        /// `{standard}`, `{name}`.
        check_intent_unknown: template "pdfExportDialog.checkIntentUnknown" = "Output intent profile: no profile is called “{name}”: a {standard} output intent embeds a profile, or names a registered printing condition (condition identifier and registry)";
        /// `{version}`, `{limit}`: the number of characters.
        check_password_legacy: template "pdfExportDialog.checkPasswordLegacy" = "{version} passwords can have up to {limit} ASCII characters (PDF 1.7 and later take any)";
        /// `{limit}`: the number of bytes.
        check_password_long: template "pdfExportDialog.checkPasswordLong" = "Passwords can have up to {limit} bytes";
        /// `{range}`, `{part}`, `{count}`.
        range_not_a_number: template "pdfExportDialog.rangeNotANumber" = "Range `{range}`: `{part}` is not a number from 1 to {count}";
        range_no_number: template "pdfExportDialog.rangeNoNumber" = "Range `{range}`: `{part}` names no number";
        range_backwards: template "pdfExportDialog.rangeBackwards" = "Range `{range}`: `{part}` runs backwards";
        range_empty: template "pdfExportDialog.rangeEmpty" = "Range `{range}` is empty";
        on: "pdfExportDialog.on" = "On";
        off: "pdfExportDialog.off" = "Off";
        /// `{preset}`: the preset the settings are compared with.
        summary_matches: template "pdfExportDialog.summaryMatches" = "Every option matches {preset}.";
        warn_layers_version: "pdfExportDialog.warnLayersVersion" = "PDF layers need PDF 1.5 or later: every layer is plain page content";
        warn_font_subset: "pdfExportDialog.warnFontSubset" = "Fonts are embedded as subsets of the characters used: a subset threshold below 100% is not applied";
        warn_permissions_password: "pdfExportDialog.warnPermissionsPassword" = "Permissions need a password: without one they are not applied";
        /// `{version}`.
        warn_plaintext_version: template "pdfExportDialog.warnPlaintextVersion" = "{version} encryption covers the metadata too: plaintext metadata needs PDF 1.5 or later";
        /// `{version}`.
        warn_weak_key: template "pdfExportDialog.warnWeakKey" = "{version} encrypts with a 40-bit key, which can be searched in minutes: choose PDF 1.6 or later for stronger protection";
        warn_pdfa_intent: "pdfExportDialog.warnPdfaIntent" = "PDF/A files carry their own output intent: the output intent and Trapped entries are left out";
        /// `{name}`.
        warn_intent_not_embedded: template "pdfExportDialog.warnIntentNotEmbedded" = "No profile is called “{name}”: the output intent names it without embedding it";
        warn_condition_needs_intent: "pdfExportDialog.warnConditionNeedsIntent" = "An output intent needs a profile or a condition identifier: the output condition and registry are left out";
    }
}

impl PdfExportWords {
    /// The label of choice `id` of the setting `kind` (`"downsample"`, `"standard"`, ...), or `fallback`, craft-pdf's
    /// English label, for names no translator changes (`PDF 1.7`, `ZIP`).
    pub(super) fn choice(&self, kind: &str, id: &str, fallback: &str) -> Label {
        let word = match (kind, id) {
            ("standard" | "monoCodec" | "imageCodec" | "printing" | "changes", "none") => &self.none,
            ("downsample", "none") => &self.no_downsampling,
            ("downsample", "average") => &self.average,
            ("downsample", "subsample") => &self.subsample,
            ("downsample", "bicubic") => &self.bicubic,
            ("imageCodec", "auto") => &self.automatic,
            ("monoCodec", "runLength") => &self.run_length,
            ("jpegQuality", "minimum") => &self.minimum,
            ("jpegQuality", "low") => &self.low,
            ("jpegQuality", "medium") => &self.medium,
            ("jpegQuality", "high") => &self.high,
            ("jpegQuality", "maximum") => &self.maximum,
            ("markKind", "roman") => &self.roman,
            ("markKind", "japanese") => &self.japanese,
            ("colorConversion", "none") => &self.no_conversion,
            ("colorConversion", "destination") => &self.convert_destination,
            ("colorConversion", "preserveNumbers") => &self.convert_preserve,
            ("profileInclusion", "none") => &self.no_profiles,
            ("profileInclusion", "all") => &self.all_profiles,
            ("profileInclusion", "destination") => &self.destination_profile,
            ("profileInclusion", "taggedSource") => &self.tagged_profiles,
            ("overprint", "preserve") => &self.preserve,
            ("overprint", "discard") => &self.discard,
            ("printing", "low") => &self.printing_low,
            ("printing", "high") => &self.printing_high,
            ("changes", "pages") => &self.changes_pages,
            ("changes", "forms") => &self.changes_forms,
            ("changes", "comments") => &self.changes_comments,
            ("changes", "any") => &self.changes_any,
            _ => return Label::from(fallback.to_owned()),
        };
        word.clone()
    }

    /// The name a built-in preset is shown by; other names (saved presets, the PDF/X ones) stay.
    pub(super) fn preset_name(&self, name: &str, app: &str) -> Label {
        if name == craft_pdf::settings::default_preset_name(app) {
            return Label::from(self.preset_default.fill(&[("app", app)]));
        }
        match name {
            "High Quality Print" => self.preset_high_quality.clone(),
            "Press Quality" => self.preset_press.clone(),
            "Smallest File Size" => self.preset_smallest.clone(),
            _ => Label::from(name.to_owned()),
        }
    }

    /// The words of a refused setting, filled from the values the check reports; craft-pdf's English message for an id
    /// this table doesn't know.
    pub fn error_text(&self, error: &SettingsError) -> String {
        let value = |key: &str| error.value(key).unwrap_or_default().to_owned();
        let fill = |word: &WordTemplate| {
            word.fill(&[("standard", &value("standard")), ("version", &value("version")), ("name", &value("name")), ("limit", &value("limit"))])
        };
        match error.id {
            "standardVersion" => fill(&self.check_standard_version),
            "standardEditing" => fill(&self.check_standard_editing),
            "standardLayers" => fill(&self.check_standard_layers),
            "standardPassword" => fill(&self.check_standard_password),
            "lowPrintingVersion" => fill(&self.check_low_printing),
            "passwordsEqual" => self.check_passwords_equal.as_str().to_owned(),
            "passwordLegacy" => fill(&self.check_password_legacy),
            "passwordLong" => fill(&self.check_password_long),
            "destinationUnknown" => fill(&self.check_destination_unknown),
            "destinationRgb" => fill(&self.check_destination_rgb),
            "intentRgb" => fill(&self.check_intent_rgb),
            "intentGray" => fill(&self.check_intent_gray),
            "intentUnknownX4" => fill(&self.check_intent_unknown_x4),
            "intentUnknown" => fill(&self.check_intent_unknown),
            "compressionPpi" | "compressionAbovePpi" | "marksWeight" | "marksOffset" | "bleedSide" | "fontSubsetPercent" => {
                self.check_range.fill(&[("field", &option_label(self, &value("field"))), ("range", &value("range")), ("value", &value("value"))])
            }
            _ => error.message.clone(),
        }
    }

    /// The words of a warning, craft-pdf's English message where no word exists.
    pub fn warning_text(&self, warning: &SettingsWarning) -> String {
        let version = warning.value("version").unwrap_or_default();
        match warning.id {
            "layersVersion" => self.warn_layers_version.as_str().to_owned(),
            "fontSubsetThreshold" => self.warn_font_subset.as_str().to_owned(),
            "permissionsNeedPassword" => self.warn_permissions_password.as_str().to_owned(),
            "plaintextMetadataVersion" => self.warn_plaintext_version.fill(&[("version", version)]),
            "weakKey" => self.warn_weak_key.fill(&[("version", version)]),
            "pdfaOutputIntent" => self.warn_pdfa_intent.as_str().to_owned(),
            "intentNotEmbedded" => self.warn_intent_not_embedded.fill(&[("name", warning.value("name").unwrap_or_default())]),
            "conditionNeedsIntent" => self.warn_condition_needs_intent.as_str().to_owned(),
            _ => warning.message.clone(),
        }
    }

    /// The words of an artboard range the document doesn't have, for a document of `count` artboards.
    pub fn range_error_text(&self, text: &str, count: usize, error: &RangeError) -> String {
        let count = count.to_string();
        let fill = |word: &WordTemplate, part: &str| word.fill(&[("range", text), ("part", part), ("count", &count)]);
        match error {
            RangeError::NotANumber { part } => fill(&self.range_not_a_number, part),
            RangeError::NoNumber { part } => fill(&self.range_no_number, part),
            RangeError::Backwards { part } => fill(&self.range_backwards, part),
            RangeError::Empty => fill(&self.range_empty, ""),
        }
    }
}
