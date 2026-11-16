//! Centralized walkthrough tours for all 15 apps in the Storytold ArtCraft Suite.

use crate::{Step, Tour};

/// Returns all built-in walkthrough tours for the specified app id.
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
                title: "הוספת טבלה".into(),
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
                ],
            },
        ],
        "pdfcraft" => vec![
            Tour {
                id: "pdfcraft.merge_split".into(),
                title: "מיזוג ופיצול עמודים".into(),
                summary: "שילוב מסמכים ושינוי סדר העמודים".into(),
                steps: vec![
                    Step {
                        target_id: "ribbon.pages".into(),
                        title: "כרטיסיית עמודים".into(),
                        description: "פתח את כרטיסיית עמודים לניהול מבנה הקובץ.".into(),
                        tip: Some("קיצור מקשים: Alt+P".into()),
                    },
                ],
            },
        ],
        "cadcraft" => vec![
            Tour {
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
            },
        ],
        "vectorcraft" => vec![
            Tour {
                id: "vectorcraft.pen_tool".into(),
                title: "ציור מסלולים וקטוריים בעט (Pen Tool)".into(),
                summary: "יצירת נקודות עגינה, קימור עקומות בזייר וצביעת קווים".into(),
                steps: vec![
                    Step {
                        target_id: "toolbar.pen".into(),
                        title: "כלי העט (P)".into(),
                        description: "בחר בכלי העט להנחת נקודות עגינה מדויקות.".into(),
                        tip: Some("קיצור מקשים: P".into()),
                    },
                ],
            },
        ],
        "filmcraft" => vec![
            Tour {
                id: "filmcraft.cut_clip".into(),
                title: "חיתוך ועריכת קטעי וידאו בציר הזמן".into(),
                summary: "ייבוא חומרי גלם, חיתוך עם כלי התער (Razor) והוספת מעברים".into(),
                steps: vec![
                    Step {
                        target_id: "timeline.razor".into(),
                        title: "כלי החיתוך (Razor)".into(),
                        description: "לחץ על ציר הזמן בנקודת החיתוך המבוקשת.".into(),
                        tip: Some("קיצור מקשים: C".into()),
                    },
                ],
            },
        ],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_catalog_returns_tours() {
        assert!(!tours_for_app("wordcraft").is_empty());
        assert!(!tours_for_app("photocraft").is_empty());
        assert!(!tours_for_app("cadcraft").is_empty());
        assert!(!tours_for_app("vectorcraft").is_empty());
        assert!(!tours_for_app("filmcraft").is_empty());
    }
}
