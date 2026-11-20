//! Centralized walkthrough tours for all applications in the Storytold ArtCraft Suite:
//! - WordCraft (Document Authoring)
//! - PhotoCraft (Raster Editing & Retouching)
//! - PdfCraft (PDF Management & Signatures)
//! - DesignCraft (InDesign/Desktop Publishing)
//! - FigCraft (Figma-style Collaborative UI/Vector Design)
//! - GridCraft (Excel-style High-Performance Spreadsheets)
//! - EffectCraft (AfterEffects-style Motion Graphics & VFX)
//! - CADCraft (3D Parametric CAD Modeling)
//! - VectorCraft (Vector Illustration)
//! - FilmCraft (Video Editing & Multitrack Timeline)
//! - SoundCraft (Audio Editing & DAW)
//! - AniCraft (2D/3D Animation)
//! - DeckCraft (Keynote/Presentation Slides)
//! - TextureCraft / MaterialCraft (3D PBR Textures & Shading)
//! - Craft Launcher (Suite Manager & Updates)

use std::sync::OnceLock;

use craft_i18n::{Embedded, I18n, Lang};

use crate::{Step, Tour};

const HE_CATALOG: &str = concat!(
    "\tCreate and style a document from scratch\tיצירת מסמך מ-0 ועיצובו\n",
    "\tWrite your first document, apply heading styles, and change fonts\tכתיבת מסמך ראשון, החלת סגנונות כותרת ושינוי גופנים\n",
    "\tHome tab\tכרטיסיית הבית\n",
    "\tSelect the Home tab on the top ribbon to access basic formatting tools.\tבחר בכרטיסיית הבית בסרגל העליון לגישה לכלי העיצוב הבסיסיים.\n",
    "\tShortcut: Alt+H\tקיצור דרך: Alt+H\n",
    "\tHeading 1 style\tסגנון כותרת 1\n",
    "\tSelect the line and choose Heading 1 to define the document structure.\tסמן את השורה ובחר בכותרת 1 להגדרת מבנה המסמך.\n",
    "\tKeyboard shortcut: Ctrl+Alt+1\tקיצור מקשים: Ctrl+Alt+1\n",
    "\tInsert Table\tהוספת טבלה (Insert Table)\n",
    "\tCreate custom tables, add rows, and format cells\tיצירת טבלאות מותאמות אישית, הוספת שורות ועיצוב תאים\n",
    "\tInsert tab\tכרטיסיית הוספה\n",
    "\tClick the Insert tab on the ribbon.\tלחץ על כרטיסיית הוספה בסרגל הכלים.\n",
    "\tShortcut: Alt+N\tקיצור דרך: Alt+N\n",
    "\tSelect table size\tבחירת טבלה\n",
    "\tDrag to choose the desired number of rows and columns.\tגרור לקביעת מספר השורות והעמודות המבוקש.\n",
    "\tConfigure Master Pages\tהגדרת עמודי אב (Master Pages)\n",
    "\tCreate text grids, running headers, and automatic page numbering for books and newspapers\tיצירת גריד טקסטואלי, כותרות עליונות ומספור עמודים אוטומטי לעימוד ספרים ועיתונים\n",
    "\tPages panel\tחלונית עמודים\n",
    "\tOpen the Pages panel and select A-Master to edit the shared template.\tפתח את חלונית העמודים ובחר ב-A-Master לעריכת התבנית הקבועה.\n",
    "\tThreaded text frame\tתיבת טקסט משורשרת\n",
    "\tDraw text frames and link them to flow copy automatically across pages.\tצייר תיבות טקסט וחבר ביניהן להזרמת מלל אוטומטית בין עמודים.\n",
    "\tPress T to select the Text tool\tהקש T לבחירת כלי הטקסט\n",
    "\tResponsive design with Auto Layout\tעיצוב רספונסיבי ב-Auto Layout\n",
    "\tBuild UI components that adapt to different screen sizes\tבניית רכיבי ממשק UI המתאימים את עצמם לגדלי מסך שונים\n",
    "\tCreate a Frame\tיצירת Frame\n",
    "\tPress F and draw a new screen frame.\tהקש F ושרטט מסגרת מסך חדשה.\n",
    "\tKeyboard shortcut: F\tקיצור מקשים: F\n",
    "\tEnable Auto Layout\tהפעלת Auto Layout\n",
    "\tPress Shift+A to arrange objects into a flowing layout with consistent spacing.\tהקש Shift+A כדי להפוך את האובייקטים למבנה זורם עם ריווחים קבועים.\n",
    "\tFormulas and data analysis\tנוסחאות וניתוח נתונים\n",
    "\tUse advanced functions (XLOOKUP, SUMIFS) and create PivotTables\tשימוש בפונקציות מתקדמות (XLOOKUP, SUMIFS) ויצירת טבלאות ציר\n",
    "\tFormula bar\tשורת הנוסחאות\n",
    "\tType = to choose a function for automatic cell and column calculations.\tהקלד = לבחירת פונקציה לחישוב אוטומטי של תאים ועמודות.\n",
    "\tPivotTable\tטבלת ציר (PivotTable)\n",
    "\tSummarize and analyze large datasets quickly and easily.\tסכם ונתח מסדי נתונים גדולים בקלות ובמהירות שיא.\n",
    "\tKeyframe animation and visual effects\tאנימציית קיפריימים ואפקטים ויזואליים\n",
    "\tAnimate motion, adjust scale, and apply Motion Blur\tהנפשת תנועה, שינוי קנה מידה וטשטוש תנועה (Motion Blur)\n",
    "\tEnable Stopwatch\tהפעלת שעון עצר (Stopwatch)\n",
    "\tClick the stopwatch icon to set the first Position keyframe.\tלחץ על סמל השעון כדי לקבוע קיפריים ראשון למיקום.\n",
    "\tMove the playhead\tהזזת סמן הזמן\n",
    "\tAdvance the playhead along the timeline and adjust object position for smooth motion.\tהזז את הסמן קדימה בציר הזמן ושנה את מיקום האובייקט ליצירת תנועה חלקה.\n",
    "\tCrop and resize image\tחיתוך ושינוי גודל תמונה\n",
    "\tTrim edges, straighten the horizon, and set pixel dimensions\tחיתוך גבולות, יישור קו אופק והגדרת מידות בפיקסלים\n",
    "\tCrop tool\tכלי החיתוך\n",
    "\tSelect the Crop tool on the left toolbar or press C.\tבחר בכלי החיתוך בסרגל השמאלי או הקש C.\n",
    "\tShortcut: C\tקיצור דרך: C\n",
    "\tDimensions and apply crop\tמידות והחלת חיתוך\n",
    "\tSet pixel dimensions and press Enter to confirm and apply the crop.\tהגדר מידות בפיקסלים ולחץ Enter לאישור והחלת החיתוך.\n",
    "\tApply: Enter\tהחלה: Enter\n",
    "\tRemove background and edit masks\tהסרת רקע ועריכת מסיכות\n",
    "\tIsolate the image subject using a non-destructive layer mask\tבידוד נושא התמונה בעזרת מסיכת שכבה לא-הורסת\n",
    "\tSelect the subject\tבחירת הנושא\n",
    "\tUse the Quick Selection tool to outline the subject.\tהשתמש בכלי הבחירה המהירה לסימון האובייקט.\n",
    "\tAdd a layer mask\tהוספת מסיכת שכבה\n",
    "\tClick Add Layer Mask in the Layers panel to isolate the subject non-destructively.\tלחץ על הוספת מסיכת שכבה בחלונית השכבות לבידוד לא-הורס של הנושא.\n",
    "\tMerge and split pages\tמיזוג ופיצול עמודים\n",
    "\tCombine documents and reorder pages\tשילוב מסמכים ושינוי סדר העמודים\n",
    "\tPages tab\tכרטיסיית עמודים\n",
    "\tOpen the Pages tab to manage document structure.\tפתח את כרטיסיית עמודים לניהול מבנה הקובץ.\n",
    "\tKeyboard shortcut: Alt+P\tקיצור מקשים: Alt+P\n",
    "\tCreate a cube and basic 3D model\tיצירת קובייה ומודל 3D בסיסי\n",
    "\tDefine dimensions, sketch a profile, and extrude into 3D space\tהגדרת מידות, שרטוט פרופיל ומשיכה למרחב (Extrude)\n",
    "\tStart a sketch\tהתחלת סקיצה\n",
    "\tChoose a sketch plane (XY/XZ) to begin drawing.\tבחר במישור השרטוט (XY/XZ) להתחלת השרטוט.\n",
    "\tExtrude\tמשיכה לנפח (Extrude)\n",
    "\tSet the extrusion depth to create a solid 3D body.\tהגדר את עומק המשיכה כדי ליצור גוף תלת-ממדי מוצק.\n",
    "\tDraw vector paths with the Pen Tool\tציור מסלולים וקטוריים בעט (Pen Tool)\n",
    "\tPlace anchor points, curve Bezier handles, and stroke paths\tיצירת נקודות עגינה, קימור עקומות בזייר וצביעת קווים\n",
    "\tPen Tool (P)\tכלי העט (P)\n",
    "\tSelect the Pen Tool to place precise anchor points.\tבחר בכלי העט להנחת נקודות עגינה מדויקות.\n",
    "\tKeyboard shortcut: P\tקיצור מקשים: P\n",
    "\tCut and edit video clips on the timeline\tחיתוך ועריכת קטעי וידאו בציר הזמן\n",
    "\tImport footage, split with the Razor tool, and add transitions\tייבוא חומרי גלם, חיתוך עם כלי התער (Razor) והוספת מעברים\n",
    "\tRazor tool\tכלי החיתוך (Razor)\n",
    "\tClick on the timeline at the desired cut point.\tלחץ על ציר הזמן בנקודת החיתוך המבוקשת.\n",
    "\tKeyboard shortcut: C\tקיצור מקשים: C\n",
    "\tInteractive Assistant & FAQ\tסייען אינטראקטיבי ושאלות נפוצות\n",
    "\tAvailable quick walkthroughs:\tמדריכים זמינים לביצוע מהיר:\n",
    "\tStart on-screen guide\tהתחל הדרכה על המסך\n",
);

static GUIDE_CATALOG: &Embedded = &[("he", "", HE_CATALOG)];

fn default_i18n() -> &'static I18n {
    static INSTANCE: OnceLock<I18n> = OnceLock::new();
    INSTANCE.get_or_init(|| I18n::new(GUIDE_CATALOG, &["he"]).0)
}

fn step(i18n: &I18n, lang: Lang, target_id: &str, title: &str, description: &str, tip: Option<&str>) -> Step {
    Step {
        target_id: target_id.into(),
        title: i18n.t(lang, title).into(),
        description: i18n.t(lang, description).into(),
        tip: tip.map(|t| i18n.t(lang, t).into()),
    }
}

fn tour(i18n: &I18n, lang: Lang, id: &str, title: &str, summary: &str, steps: Vec<Step>) -> Tour {
    Tour { id: id.into(), title: i18n.t(lang, title).into(), summary: i18n.t(lang, summary).into(), steps }
}

/// Returns built-in walkthrough tours for `app_id` localized using `i18n` and `lang`.
pub fn tours_for_app_in(app_id: &str, i18n: &I18n, lang: Lang) -> Vec<Tour> {
    match app_id {
        "wordcraft" => vec![
            tour(
                i18n,
                lang,
                "wordcraft.create_document",
                "Create and style a document from scratch",
                "Write your first document, apply heading styles, and change fonts",
                vec![
                    step(
                        i18n,
                        lang,
                        "ribbon.home",
                        "Home tab",
                        "Select the Home tab on the top ribbon to access basic formatting tools.",
                        Some("Shortcut: Alt+H"),
                    ),
                    step(
                        i18n,
                        lang,
                        "ribbon.styles.heading1",
                        "Heading 1 style",
                        "Select the line and choose Heading 1 to define the document structure.",
                        Some("Keyboard shortcut: Ctrl+Alt+1"),
                    ),
                ],
            ),
            tour(
                i18n,
                lang,
                "wordcraft.insert_table",
                "Insert Table",
                "Create custom tables, add rows, and format cells",
                vec![
                    step(i18n, lang, "ribbon.insert", "Insert tab", "Click the Insert tab on the ribbon.", Some("Shortcut: Alt+N")),
                    step(i18n, lang, "ribbon.insert.table", "Select table size", "Drag to choose the desired number of rows and columns.", None),
                ],
            ),
        ],
        "designcraft" => vec![tour(
            i18n,
            lang,
            "designcraft.master_pages",
            "Configure Master Pages",
            "Create text grids, running headers, and automatic page numbering for books and newspapers",
            vec![
                step(i18n, lang, "panel.pages", "Pages panel", "Open the Pages panel and select A-Master to edit the shared template.", None),
                step(
                    i18n,
                    lang,
                    "tool.text_frame",
                    "Threaded text frame",
                    "Draw text frames and link them to flow copy automatically across pages.",
                    Some("Press T to select the Text tool"),
                ),
            ],
        )],
        "figcraft" => vec![tour(
            i18n,
            lang,
            "figcraft.auto_layout",
            "Responsive design with Auto Layout",
            "Build UI components that adapt to different screen sizes",
            vec![
                step(i18n, lang, "canvas.frame", "Create a Frame", "Press F and draw a new screen frame.", Some("Keyboard shortcut: F")),
                step(
                    i18n,
                    lang,
                    "panel.auto_layout",
                    "Enable Auto Layout",
                    "Press Shift+A to arrange objects into a flowing layout with consistent spacing.",
                    Some("Shift+A"),
                ),
            ],
        )],
        "gridcraft" => vec![tour(
            i18n,
            lang,
            "gridcraft.formulas_and_pivot",
            "Formulas and data analysis",
            "Use advanced functions (XLOOKUP, SUMIFS) and create PivotTables",
            vec![
                step(i18n, lang, "formula_bar", "Formula bar", "Type = to choose a function for automatic cell and column calculations.", None),
                step(i18n, lang, "ribbon.insert.pivot", "PivotTable", "Summarize and analyze large datasets quickly and easily.", None),
            ],
        )],
        "effectcraft" => vec![tour(
            i18n,
            lang,
            "effectcraft.keyframe_motion",
            "Keyframe animation and visual effects",
            "Animate motion, adjust scale, and apply Motion Blur",
            vec![
                step(
                    i18n,
                    lang,
                    "timeline.property.position",
                    "Enable Stopwatch",
                    "Click the stopwatch icon to set the first Position keyframe.",
                    None,
                ),
                step(
                    i18n,
                    lang,
                    "timeline.playhead",
                    "Move the playhead",
                    "Advance the playhead along the timeline and adjust object position for smooth motion.",
                    None,
                ),
            ],
        )],
        "photocraft" => vec![
            tour(
                i18n,
                lang,
                "photocraft.crop_and_resize",
                "Crop and resize image",
                "Trim edges, straighten the horizon, and set pixel dimensions",
                vec![
                    step(i18n, lang, "toolbar.crop", "Crop tool", "Select the Crop tool on the left toolbar or press C.", Some("Shortcut: C")),
                    step(
                        i18n,
                        lang,
                        "panel.image_size",
                        "Dimensions and apply crop",
                        "Set pixel dimensions and press Enter to confirm and apply the crop.",
                        Some("Apply: Enter"),
                    ),
                ],
            ),
            tour(
                i18n,
                lang,
                "photocraft.remove_bg",
                "Remove background and edit masks",
                "Isolate the image subject using a non-destructive layer mask",
                vec![
                    step(i18n, lang, "toolbar.select", "Select the subject", "Use the Quick Selection tool to outline the subject.", None),
                    step(
                        i18n,
                        lang,
                        "panel.layers.add_mask",
                        "Add a layer mask",
                        "Click Add Layer Mask in the Layers panel to isolate the subject non-destructively.",
                        None,
                    ),
                ],
            ),
        ],
        "pdfcraft" => vec![tour(
            i18n,
            lang,
            "pdfcraft.merge_split",
            "Merge and split pages",
            "Combine documents and reorder pages",
            vec![step(i18n, lang, "ribbon.pages", "Pages tab", "Open the Pages tab to manage document structure.", Some("Keyboard shortcut: Alt+P"))],
        )],
        "cadcraft" => vec![tour(
            i18n,
            lang,
            "cadcraft.draw_cube",
            "Create a cube and basic 3D model",
            "Define dimensions, sketch a profile, and extrude into 3D space",
            vec![
                step(i18n, lang, "toolbar.sketch", "Start a sketch", "Choose a sketch plane (XY/XZ) to begin drawing.", None),
                step(i18n, lang, "toolbar.extrude", "Extrude", "Set the extrusion depth to create a solid 3D body.", None),
            ],
        )],
        "vectorcraft" => vec![tour(
            i18n,
            lang,
            "vectorcraft.pen_tool",
            "Draw vector paths with the Pen Tool",
            "Place anchor points, curve Bezier handles, and stroke paths",
            vec![step(
                i18n,
                lang,
                "toolbar.pen",
                "Pen Tool (P)",
                "Select the Pen Tool to place precise anchor points.",
                Some("Keyboard shortcut: P"),
            )],
        )],
        "filmcraft" => vec![tour(
            i18n,
            lang,
            "filmcraft.cut_clip",
            "Cut and edit video clips on the timeline",
            "Import footage, split with the Razor tool, and add transitions",
            vec![step(i18n, lang, "timeline.razor", "Razor tool", "Click on the timeline at the desired cut point.", Some("Keyboard shortcut: C"))],
        )],
        _ => Vec::new(),
    }
}

/// Returns built-in walkthrough tours for `app_id` in the requested BCP 47 `locale` (falling back to English).
pub fn tours_for_app_locale(app_id: &str, locale: &str) -> Vec<Tour> {
    let i18n = default_i18n();
    let lang = i18n.from_code(locale).unwrap_or(Lang::EN);
    tours_for_app_in(app_id, i18n, lang)
}

/// Returns all built-in walkthrough tours for any specified app in the ArtCraft suite.
pub fn tours_for_app(app_id: &str) -> Vec<Tour> {
    tours_for_app_locale(app_id, "he")
}

/// Translates a guide UI label `source` into `locale`, falling back to English.
pub fn tr_locale(locale: &str, source: &str) -> String {
    let i18n = default_i18n();
    let lang = i18n.from_code(locale).unwrap_or(Lang::EN);
    i18n.t(lang, source).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_suite_catalog_returns_tours_for_all_apps() {
        let apps =
            ["wordcraft", "designcraft", "figcraft", "gridcraft", "effectcraft", "photocraft", "pdfcraft", "cadcraft", "vectorcraft", "filmcraft"];
        for app in apps {
            assert!(!tours_for_app(app).is_empty(), "Missing tours for app: {app}");
        }
    }

    #[test]
    fn catalog_localizes_through_craft_i18n() {
        let i18n = default_i18n();
        assert!(i18n.check_all().is_empty(), "embedded guide catalog must have no parse problems: {:?}", i18n.check_all());

        let en = tours_for_app_locale("wordcraft", "en");
        assert_eq!(en[1].title, "Insert Table");

        let he = tours_for_app_locale("wordcraft", "he");
        assert_eq!(he[1].title, "הוספת טבלה (Insert Table)");

        let fallback = tours_for_app_locale("wordcraft", "fr");
        assert_eq!(fallback[1].title, "Insert Table");
    }
}
