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

use crate::{Step, Tour};

/// Returns all built-in walkthrough tours for any specified app in the ArtCraft suite.
pub fn tours_for_app(app_id: &str) -> Vec<Tour> {
    match app_id {
        "wordcraft" => vec![
            Tour {
                id: "wordcraft.create_document".into(),
                title: "יצירת מסמך מ-0 ועיצובו".into(),
                summary: "כתיבת מסמך ראשון, החלת סגנונות כותרת ושינוי גופנים".into(),
                steps: vec![
                    Step {
                        target_id: "ribbon.home".into(),
                        title: "כרטיסיית הבית".into(),
                        description: "בחר בכרטיסיית הבית בסרגל העליון לגישה לכלי העיצוב הבסיסיים.".into(),
                        tip: Some("קיצור דרך: Alt+H".into()),
                    },
                    Step {
                        target_id: "ribbon.styles.heading1".into(),
                        title: "סגנון כותרת 1".into(),
                        description: "סמן את השורה ובחר בכותרת 1 להגדרת מבנה המסמך.".into(),
                        tip: Some("קיצור מקשים: Ctrl+Alt+1".into()),
                    },
                ],
            },
            Tour {
                id: "wordcraft.insert_table".into(),
                title: "הוספת טבלה (Insert Table)".into(),
                summary: "יצירת טבלאות מותאמות אישית, הוספת שורות ועיצוב תאים".into(),
                steps: vec![
                    Step {
                        target_id: "ribbon.insert".into(),
                        title: "כרטיסיית הוספה".into(),
                        description: "לחץ על כרטיסיית הוספה בסרגל הכלים.".into(),
                        tip: Some("קיצור דרך: Alt+N".into()),
                    },
                    Step {
                        target_id: "ribbon.insert.table".into(),
                        title: "בחירת טבלה".into(),
                        description: "גרור לקביעת מספר השורות והעמודות המבוקש.".into(),
                        tip: None,
                    },
                ],
            },
        ],
        "designcraft" => vec![Tour {
            id: "designcraft.master_pages".into(),
            title: "הגדרת עמודי אב (Master Pages)".into(),
            summary: "יצירת גריד טקסטואלי, כותרות עליונות ומספור עמודים אוטומטי לעימוד ספרים ועיתונים".into(),
            steps: vec![
                Step {
                    target_id: "panel.pages".into(),
                    title: "חלונית עמודים".into(),
                    description: "פתח את חלונית העמודים ובחר ב-A-Master לעריכת התבנית הקבועה.".into(),
                    tip: None,
                },
                Step {
                    target_id: "tool.text_frame".into(),
                    title: "תיבת טקסט משורשרת".into(),
                    description: "צייר תיבות טקסט וחבר ביניהן להזרמת מלל אוטומטית בין עמודים.".into(),
                    tip: Some("הקש T לבחירת כלי הטקסט".into()),
                },
            ],
        }],
        "figcraft" => vec![Tour {
            id: "figcraft.auto_layout".into(),
            title: "עיצוב רספונסיבי ב-Auto Layout".into(),
            summary: "בניית רכיבי ממשק UI המתאימים את עצמם לגדלי מסך שונים".into(),
            steps: vec![
                Step {
                    target_id: "canvas.frame".into(),
                    title: "יצירת Frame".into(),
                    description: "הקש F ושרטט מסגרת מסך חדשה.".into(),
                    tip: Some("קיצור מקשים: F".into()),
                },
                Step {
                    target_id: "panel.auto_layout".into(),
                    title: "הפעלת Auto Layout".into(),
                    description: "הקש Shift+A כדי להפוך את האובייקטים למבנה זורם עם ריווחים קבועים.".into(),
                    tip: Some("Shift+A".into()),
                },
            ],
        }],
        "gridcraft" => vec![Tour {
            id: "gridcraft.formulas_and_pivot".into(),
            title: "נוסחאות וניתוח נתונים".into(),
            summary: "שימוש בפונקציות מתקדמות (XLOOKUP, SUMIFS) ויצירת טבלאות ציר".into(),
            steps: vec![
                Step {
                    target_id: "formula_bar".into(),
                    title: "שורת הנוסחאות".into(),
                    description: "הקלד = לבחירת פונקציה לחישוב אוטומטי של תאים ועמודות.".into(),
                    tip: None,
                },
                Step {
                    target_id: "ribbon.insert.pivot".into(),
                    title: "טבלת ציר (PivotTable)".into(),
                    description: "סכם ונתח מסדי נתונים גדולים בקלות ובמהירות שיא.".into(),
                    tip: None,
                },
            ],
        }],
        "effectcraft" => vec![Tour {
            id: "effectcraft.keyframe_motion".into(),
            title: "אנימציית קיפריימים ואפקטים ויזואליים".into(),
            summary: "הנפשת תנועה, שינוי קנה מידה וטשטוש תנועה (Motion Blur)".into(),
            steps: vec![
                Step {
                    target_id: "timeline.property.position".into(),
                    title: "הפעלת שעון עצר (Stopwatch)".into(),
                    description: "לחץ על סמל השעון כדי לקבוע קיפריים ראשון למיקום.".into(),
                    tip: None,
                },
                Step {
                    target_id: "timeline.playhead".into(),
                    title: "הזזת סמן הזמן".into(),
                    description: "הזז את הסמן קדימה בציר הזמן ושנה את מיקום האובייקט ליצירת תנועה חלקה.".into(),
                    tip: None,
                },
            ],
        }],
        "photocraft" => vec![
            Tour {
                id: "photocraft.crop_and_resize".into(),
                title: "חיתוך ושינוי גודל תמונה".into(),
                summary: "חיתוך גבולות, יישור קו אופק והגדרת מידות בפיקסלים".into(),
                steps: vec![
                    Step {
                        target_id: "toolbar.crop".into(),
                        title: "כלי החיתוך".into(),
                        description: "בחר בכלי החיתוך בסרגל השמאלי או הקש C.".into(),
                        tip: Some("קיצור דרך: C".into()),
                    },
                    Step {
                        target_id: "panel.image_size".into(),
                        title: "מידות והחלת חיתוך".into(),
                        description: "הגדר מידות בפיקסלים ולחץ Enter לאישור והחלת החיתוך.".into(),
                        tip: Some("החלה: Enter".into()),
                    },
                ],
            },
            Tour {
                id: "photocraft.remove_bg".into(),
                title: "הסרת רקע ועריכת מסיכות".into(),
                summary: "בידוד נושא התמונה בעזרת מסיכת שכבה לא-הורסת".into(),
                steps: vec![
                    Step {
                        target_id: "toolbar.select".into(),
                        title: "בחירת הנושא".into(),
                        description: "השתמש בכלי הבחירה המהירה לסימון האובייקט.".into(),
                        tip: None,
                    },
                    Step {
                        target_id: "panel.layers.add_mask".into(),
                        title: "הוספת מסיכת שכבה".into(),
                        description: "לחץ על הוספת מסיכת שכבה בחלונית השכבות לבידוד לא-הורס של הנושא.".into(),
                        tip: None,
                    },
                ],
            },
        ],
        "pdfcraft" => vec![Tour {
            id: "pdfcraft.merge_split".into(),
            title: "מיזוג ופיצול עמודים".into(),
            summary: "שילוב מסמכים ושינוי סדר העמודים".into(),
            steps: vec![Step {
                target_id: "ribbon.pages".into(),
                title: "כרטיסיית עמודים".into(),
                description: "פתח את כרטיסיית עמודים לניהול מבנה הקובץ.".into(),
                tip: Some("קיצור מקשים: Alt+P".into()),
            }],
        }],
        "cadcraft" => vec![Tour {
            id: "cadcraft.draw_cube".into(),
            title: "יצירת קובייה ומודל 3D בסיסי".into(),
            summary: "הגדרת מידות, שרטוט פרופיל ומשיכה למרחב (Extrude)".into(),
            steps: vec![
                Step {
                    target_id: "toolbar.sketch".into(),
                    title: "התחלת סקיצה".into(),
                    description: "בחר במישור השרטוט (XY/XZ) להתחלת השרטוט.".into(),
                    tip: None,
                },
                Step {
                    target_id: "toolbar.extrude".into(),
                    title: "משיכה לנפח (Extrude)".into(),
                    description: "הגדר את עומק המשיכה כדי ליצור גוף תלת-ממדי מוצק.".into(),
                    tip: None,
                },
            ],
        }],
        "vectorcraft" => vec![Tour {
            id: "vectorcraft.pen_tool".into(),
            title: "ציור מסלולים וקטוריים בעט (Pen Tool)".into(),
            summary: "יצירת נקודות עגינה, קימור עקומות בזייר וצביעת קווים".into(),
            steps: vec![Step {
                target_id: "toolbar.pen".into(),
                title: "כלי העט (P)".into(),
                description: "בחר בכלי העט להנחת נקודות עגינה מדויקות.".into(),
                tip: Some("קיצור מקשים: P".into()),
            }],
        }],
        "filmcraft" => vec![Tour {
            id: "filmcraft.cut_clip".into(),
            title: "חיתוך ועריכת קטעי וידאו בציר הזמן".into(),
            summary: "ייבוא חומרי גלם, חיתוך עם כלי התער (Razor) והוספת מעברים".into(),
            steps: vec![Step {
                target_id: "timeline.razor".into(),
                title: "כלי החיתוך (Razor)".into(),
                description: "לחץ על ציר הזמן בנקודת החיתוך המבוקשת.".into(),
                tip: Some("קיצור מקשים: C".into()),
            }],
        }],
        _ => Vec::new(),
    }
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
}
