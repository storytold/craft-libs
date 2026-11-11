//! Building the chains: for each family, which faces come in which order.

use craft_fonts_build::{EmbeddedFace, ServedFace};
use craft_i18n::Han;
use craft_theme::InterfaceFont;
use egui::{FontData, FontDefinitions, FontFamily};

use super::{CjkRows, Family, SEMIBOLD, Stack, SystemFonts, UiFonts};
use crate::fonts::defaults::SUITE_FACES;
use crate::fonts::faces::{FaceList, baseline_fallback};
use crate::fonts::script::EMOJI;
use crate::fonts::system::ui_face::{SYSTEM_UI_REGULAR, SYSTEM_UI_SEMIBOLD, SystemUiFace};
use crate::fonts::{FontRef, set_weight};

/// Scripts the app's own UI faces cover unless [`UiFonts::app_scripts`] says otherwise.
pub(super) const APP_SCRIPTS: &[&str] = &["Latn", "Cyrl", "Grek"];

/// Scripts whose characters sit inside the em square, so they fit a row of a Latin face. These are the CJK scripts
/// ([`is_cjk`]); the CJK face tags of craft-fonts-build (`CJK_SCRIPTS`) cover exactly these through [`covers`].
pub(in crate::fonts) const EM_SQUARE_SCRIPTS: &[&str] = &["Hani", "Hira", "Kana", "Hang", "Bopo"];

/// craft-fonts' interface face for Japanese; its manifest also lists serif Japanese faces.
const DEFAULT_PREFER: &[(&str, &str)] = &[("Jpan", "BIZ UDPGothic")];

/// A served face as the stack orders it, without bytes.
pub(super) fn without_bytes(face: &ServedFace) -> EmbeddedFace {
    EmbeddedFace { family: face.family, style: face.style, scripts: face.scripts, index: face.index, bytes: &[] }
}

/// Whether a face tagged `tags` draws `script`; the union tags cover their parts.
pub(in crate::fonts) fn covers(tags: &[&str], script: &str) -> bool {
    tags.iter().any(|tag| {
        *tag == script
            || match *tag {
                "Jpan" => matches!(script, "Hani" | "Hira" | "Kana"),
                "Kore" => matches!(script, "Hang" | "Hani"),
                "Hans" | "Hant-HK" => script == "Hani",
                "Hant" => matches!(script, "Hani" | "Bopo"),
                _ => false,
            }
    })
}

/// Whether `script` is a CJK script: Han, Kana (Hiragana included), Hangul or Bopomofo.
pub(in crate::fonts) fn is_cjk(script: &str) -> bool {
    EM_SQUARE_SCRIPTS.contains(&script)
}

fn han_region(face: &EmbeddedFace) -> Option<Han> {
    face.scripts.iter().find_map(|tag| match *tag {
        "Hans" => Some(Han::Simplified),
        "Hant" => Some(Han::Traditional),
        "Hant-HK" => Some(Han::HongKong),
        "Jpan" => Some(Han::Japanese),
        "Kore" => Some(Han::Korean),
        _ => None,
    })
}

/// `0` for the style a family of `weight` wants, `1` for an acceptable stand-in.
fn style_rank(face: &EmbeddedFace, weight: u16) -> Option<u8> {
    let regular = face.style.eq_ignore_ascii_case("regular");
    let bold = ["bold", "semibold"].iter().any(|s| face.style.eq_ignore_ascii_case(s));
    match (weight >= 600, regular, bold) {
        (false, true, _) | (true, _, true) => Some(0),
        (true, true, _) => Some(1),
        _ => None,
    }
}

fn data(face: &EmbeddedFace) -> FontData {
    baseline_fallback(FontData { index: face.index, ..FontData::from_static(face.bytes) })
}

/// Script codes with the CLDR unions spelled out.
pub(in crate::fonts) fn expand<S: AsRef<str>>(scripts: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for script in scripts {
        let parts: Vec<&str> = match script.as_ref() {
            "Jpan" => vec!["Hani", "Hira", "Kana"],
            "Kore" => vec!["Hang", "Hani"],
            "Hans" | "Hant" => vec!["Hani"],
            other => vec![other],
        };
        for part in parts {
            if !out.iter().any(|s| s == part) {
                out.push(part.to_owned());
            }
        }
    }
    out
}

fn dedup(names: &mut Vec<String>) {
    let mut seen = std::collections::BTreeSet::new();
    names.retain(|n| !n.is_empty() && seen.insert(n.clone()));
}

/// The first of `prefer`'s `(script tag, family)` entries that names `face`, or the family whose
/// interface face `face` is (`Noto Sans TC` names `Noto Sans TC UI`).
fn preference_of<'p>(mut prefer: impl Iterator<Item = (&'p str, &'p str)>, face: &EmbeddedFace) -> Option<usize> {
    let full_family = face.family.strip_suffix(" UI").unwrap_or(face.family);
    prefer.position(|(tag, family)| face.scripts.contains(&tag) && (face.family == family || full_family == family))
}

/// How well a face's Han region suits the one wanted: lower is better.
fn region_rank(face: Option<Han>, wanted: Option<Han>) -> u8 {
    match (face, wanted) {
        (_, None) => 0,
        (Some(f), Some(w)) if f == w => 0,
        (Some(Han::Traditional), Some(Han::HongKong)) | (Some(Han::HongKong), Some(Han::Traditional)) => 1,
        // The other Chinese forms are closer than Japanese or Korean ones.
        (Some(Han::Simplified), Some(Han::Traditional | Han::HongKong)) | (Some(Han::Traditional | Han::HongKong), Some(Han::Simplified)) => 2,
        (None, Some(_)) => 2,
        _ => 3,
    }
}

/// A face with its egui name.
#[derive(Clone)]
struct Face {
    name: String,
    data: FontData,
    /// The served face this stands for; `data` is empty until it is fetched.
    served: Option<ServedFace>,
    /// The system face's slot, whose data the stack adds when the face resolves.
    system_ui: bool,
}

/// One family's faces, as [`UiFonts::chain`] orders them.
struct Chain {
    configured: Vec<Face>,
    /// Faces asked after the system fonts.
    after: Vec<Face>,
    /// Names of the served faces the UI language needs in this family.
    ui_downloads: Vec<String>,
}

impl Face {
    fn new(name: &str, data: FontData) -> Self {
        Self { name: name.to_owned(), data, served: None, system_ui: false }
    }

    /// Set a variable face to `weight`. A face that isn't at regular weight gets its own name, so
    /// the regular and bold uses of one file don't collide in egui's face store.
    fn weighted(mut self, weight: u16) -> Self {
        if set_weight(&mut self.data, weight) && weight != 400 {
            self.name = format!("{} @{weight}", self.name);
        }
        self
    }
}

impl UiFonts<'_> {
    /// [`Self::stack`] with the system face resolved as `resolved` (`None`: not yet).
    pub(super) fn stack_with_system_ui(&self, resolved: Option<Option<&SystemUiFace>>) -> Stack {
        let mut stack = Stack {
            definitions: FontDefinitions::empty(),
            after_system: FaceList::default(),
            system: self.system,
            system_provider: self.system_provider.clone(),
            ui_language: self.ui_language.clone(),
            language_scripts: std::iter::once(&self.ui_language)
                .chain(&self.languages)
                .map(|language| (language.clone(), expand(&self.scripts.scripts(language))))
                .collect(),
            weights: Vec::new(),
            served: Vec::new(),
            ui_downloads: Vec::new(),
            cjk_rows: self.cjk_rows,
            cjk_faces: Vec::new(),
            system_ui_pending: false,
            fetcher: self.fetcher.clone(),
        };
        let defaults = crate::fonts::deflated_default_definitions();
        let cjk_names: Vec<String> = match self.cjk_rows {
            CjkRows::Fit => {
                self.craft_faces.iter().filter(|face| EM_SQUARE_SCRIPTS.iter().any(|s| covers(face.scripts, s))).map(EmbeddedFace::name).collect()
            }
            CjkRows::Grow => Vec::new(),
        };
        let mut cjk_faces: Vec<String> = Vec::new();
        for family in self.family_specs() {
            let default_family = if family.monospaced { &FontFamily::Monospace } else { &family.family };
            let default_chain = defaults.families.get(default_family).or_else(|| defaults.families.get(&FontFamily::Proportional));
            // The suite's interface face stands in for an app's own faces only where it has none; behind them, egui's faces follow.
            let own_faces = !self.app_faces_for(&family).is_empty();
            let egui_faces: Vec<Face> = default_chain
                .into_iter()
                .flatten()
                .filter(|name| !(own_faces && SUITE_FACES.contains(&name.as_str())))
                .filter_map(|name| Some(Face::new(name, (**defaults.font_data.get(name)?).clone())))
                .collect();
            let mut chain = self.chain(&family, egui_faces);
            let cjk = chain.configured.iter().chain(&chain.after).filter(|face| cjk_names.contains(&face.name));
            cjk_faces.extend(cjk.map(|face| face.clone().weighted(family.weight).name));
            chain.configured.extend(
                self.egui_faces
                    .iter()
                    .filter(|(_, families)| families.contains(&family.family))
                    .filter_map(|(name, _)| Some(Face::new(name, (**defaults.font_data.get(name)?).clone()))),
            );
            let mut configured: Vec<String> = chain.configured.into_iter().map(|face| stack.add_configured(face.weighted(family.weight))).collect();
            let mut tail: Vec<String> = chain.after.into_iter().map(|face| stack.add_after(face.weighted(family.weight))).collect();
            for face in chain.ui_downloads {
                if !stack.ui_downloads.contains(&face) {
                    stack.ui_downloads.push(face);
                }
            }
            dedup(&mut configured);
            tail.retain(|name| !configured.contains(name));
            dedup(&mut tail);
            stack.definitions.families.insert(family.family.clone(), configured);
            stack.after_system.set_chain(FontRef::from_family(family.family.clone()), tail);
            stack.weights.push((FontRef::from_family(family.family), family.weight));
        }
        stack.fit_into_rows(&cjk_faces);
        stack.cjk_faces = cjk_faces;
        if self.interface == InterfaceFont::System {
            stack.system_ui_pending = true;
            if let Some(face) = resolved {
                stack.provide_system_ui(face);
            }
        }
        stack
    }

    /// Proportional and Monospace, every family an app face names, then the declared families.
    fn family_specs(&self) -> Vec<Family> {
        let mut specs = vec![Family::proportional(), Family::monospace()];
        // With no face of its own for the interface, the app gets the suite's, whose semibold roles draw in Inter SemiBold.
        if !self.app_faces.iter().any(|face| face.families.contains(&FontFamily::Proportional)) {
            specs.push(Family::named(super::SEMIBOLD).weight(600));
        }
        for family in self.app_faces.iter().flat_map(|f| &f.families) {
            if !specs.iter().any(|s| s.family == *family) {
                specs.push(Family { family: family.clone(), weight: super::REGULAR, lead: None, monospaced: false });
            }
        }
        for declared in &self.families {
            match specs.iter_mut().find(|s| s.family == declared.family) {
                Some(spec) => *spec = declared.clone(),
                None => specs.push(declared.clone()),
            }
        }
        specs
    }

    /// One family's faces. `egui_faces` are egui's defaults for it, which stand in for the app's
    /// faces when the app has none for the family.
    fn chain(&self, family: &Family, egui_faces: Vec<Face>) -> Chain {
        let weight = family.weight;
        let ui_scripts = expand(&self.scripts.scripts(&self.ui_language));
        let ui_han = self.han(&self.ui_language);

        let app = self.app_faces_for(family);
        let own_faces = !app.is_empty();
        let (mut base, egui_tail) = if own_faces { (app, egui_faces) } else { (egui_faces, Vec::new()) };
        // egui's default faces cover the scripts app faces are assumed to.
        let covered = |script: &str| if own_faces { self.app_covers(script) } else { APP_SCRIPTS.contains(&script) };

        let lead: Vec<usize> = expand(family.lead.as_slice()).iter().flat_map(|s| self.candidates(s, ui_han, weight)).collect();
        let language: Vec<usize> = ui_scripts.iter().filter(|s| !covered(s)).flat_map(|s| self.candidates(s, ui_han, weight)).collect();
        // Letters of a covered script that the base faces lack, such as Vietnamese ổ and ạ in
        // egui's Ubuntu-Light, come from that script's own faces before any other face with them.
        // CJK faces carry Latin at CJK proportions, so they don't stand in.
        let backup: Vec<usize> = ui_scripts
            .iter()
            .filter(|s| covered(s))
            .flat_map(|s| self.candidates(s, ui_han, weight))
            .filter(|&i| !covers(self.craft_faces[i].scripts, "Hani"))
            .collect();
        let mut others: Vec<usize> = Vec::new();
        for tag in self.languages.iter().filter(|l| **l != self.ui_language) {
            let han = self.han(tag);
            for script in expand(&self.scripts.scripts(tag)).iter().filter(|s| !covered(s)) {
                others.extend(self.candidates(script, han, weight));
            }
        }
        let mut rest: Vec<(u8, usize)> =
            self.craft_faces.iter().enumerate().filter_map(|(i, face)| style_rank(face, weight).map(|rank| (rank, i))).collect();
        rest.sort();
        others.extend(rest.into_iter().map(|(_, i)| i));

        let main_needs_lead = ui_scripts.first().is_some_and(|main| !covered(main) && !EM_SQUARE_SCRIPTS.contains(&main.as_str()));
        let language_first = !family.monospaced && main_needs_lead;
        let craft = |indices: Vec<usize>| indices.into_iter().map(|i| self.craft_face(i)).collect::<Vec<_>>();
        // The best face for each of the UI language's scripts, the ones a web build fetches first.
        let ui_downloads = ui_scripts
            .iter()
            .filter(|s| !covered(s))
            .filter_map(|s| self.candidates(s, ui_han, weight).first().map(|i| self.craft_face(*i)))
            .filter(|face| face.served.is_some())
            .map(|face| face.name)
            .collect();
        base.splice(0..0, self.system_ui_slot(&family.family));

        let mut configured = craft(lead);
        if language_first {
            configured.extend(craft(language));
            configured.extend(base);
            configured.extend(craft(backup));
        } else {
            configured.extend(base);
            configured.extend(craft(backup));
            configured.extend(craft(language));
        }
        let mut after = Vec::new();
        match self.system {
            SystemFonts::BeforeCraft => after.extend(craft(others)),
            SystemFonts::AfterCraft | SystemFonts::Off => configured.extend(craft(others)),
        }
        let emoji = |name: &str, data: &Option<FontData>| Some(Face::new(name, data.clone()?));
        after.extend(emoji("emoji-color", &self.emoji.color));
        after.extend(emoji("emoji-web", &self.emoji.web));
        after.extend(self.emoji.served.map(|face| {
            let face = ServedFace { scripts: &[EMOJI], ..face };
            Face { served: Some(face), ..Face::new(&face.name(), FontData::from_static(&[])) }
        }));
        after.extend(emoji("emoji-monochrome", &self.emoji.monochrome));
        after.extend(egui_tail);
        Chain { configured, after, ui_downloads }
    }

    /// With [`InterfaceFont::System`], the system face's slot ahead of the app's faces in the
    /// proportional and [`SEMIBOLD`] families: its name, without data until
    /// [`Stack::provide_system_ui`].
    fn system_ui_slot(&self, family: &FontFamily) -> Option<Face> {
        let name = match family {
            _ if self.interface != InterfaceFont::System => return None,
            FontFamily::Proportional => SYSTEM_UI_REGULAR,
            FontFamily::Name(name) if &**name == SEMIBOLD => SYSTEM_UI_SEMIBOLD,
            _ => return None,
        };
        Some(Face { system_ui: true, ..Face::new(name, FontData::from_static(&[])) })
    }

    /// The app faces heading `family`; a named family without its own takes the monospace ones if
    /// it is [`Family::monospaced`], the proportional ones otherwise.
    fn app_faces_for(&self, family: &Family) -> Vec<Face> {
        let faces = |family: &FontFamily| -> Vec<Face> {
            self.app_faces.iter().filter(|face| face.families.contains(family)).map(|face| Face::new(&face.name, face.data.clone())).collect()
        };
        match faces(&family.family) {
            own if !own.is_empty() || !matches!(family.family, FontFamily::Name(_)) => own,
            _ if family.monospaced => faces(&FontFamily::Monospace),
            _ => faces(&FontFamily::Proportional),
        }
    }

    /// Craft faces for `script`, best first: the right Han region, then the app's preference,
    /// then the right style, then manifest order. craft-fonts' own preference ([`DEFAULT_PREFER`])
    /// orders only the faces of the best region found.
    fn candidates(&self, script: &str, han: Option<Han>, weight: u16) -> Vec<usize> {
        let found: Vec<(u8, u8, usize)> = self
            .craft_faces
            .iter()
            .enumerate()
            .filter(|(_, face)| covers(face.scripts, script))
            .filter_map(|(i, face)| {
                let style = style_rank(face, weight)?;
                let region = if script == "Hani" { region_rank(han_region(face), han) } else { 0 };
                Some((region, style, i))
            })
            .collect();
        let best = found.iter().map(|(region, ..)| *region).min();
        let mut ranked: Vec<(u8, usize, u8, usize)> = found
            .into_iter()
            .map(|(region, style, i)| {
                let face = &self.craft_faces[i];
                let app = preference_of(self.prefer.iter().map(|(s, f)| (s.as_str(), f.as_str())), face);
                let default = || (Some(region) == best).then(|| preference_of(DEFAULT_PREFER.iter().copied(), face)).flatten();
                let preference = app.or_else(|| default().map(|p| self.prefer.len() + p)).unwrap_or(usize::MAX);
                (region, preference, style, i)
            })
            .collect();
        ranked.sort();
        ranked.into_iter().map(|(.., i)| i).collect()
    }

    fn craft_face(&self, index: usize) -> Face {
        let face = self.craft_faces.get(index);
        Face {
            name: face.map(EmbeddedFace::name).unwrap_or_default(),
            data: face.map_or_else(|| FontData::from_static(&[]), data),
            served: self.served.get(index).copied().flatten(),
            system_ui: false,
        }
    }

    fn app_covers(&self, script: &str) -> bool {
        self.app_scripts.iter().any(|s| s == script)
    }

    fn han(&self, language: &str) -> Option<Han> {
        self.scripts.han(language).or_else(|| {
            self.scripts.scripts(language).iter().find_map(|s| match *s {
                "Hans" => Some(Han::Simplified),
                "Hant" => Some(Han::Traditional),
                "Jpan" => Some(Han::Japanese),
                "Kore" => Some(Han::Korean),
                _ => None,
            })
        })
    }
}

impl Stack {
    fn add_configured(&mut self, face: Face) -> String {
        if !face.system_ui && !self.add_served(&face) {
            self.definitions.font_data.entry(face.name.clone()).or_insert_with(|| std::sync::Arc::new(face.data));
        }
        face.name
    }

    fn add_after(&mut self, face: Face) -> String {
        if self.add_served(&face) { face.name } else { self.after_system.add(face.name, face.data) }
    }

    /// Record a served face, whose data comes later; false for any other face.
    fn add_served(&mut self, face: &Face) -> bool {
        let Some(served) = face.served else { return false };
        if !self.served.iter().any(|s| s.name() == face.name) {
            self.served.push(served);
        }
        true
    }
}
