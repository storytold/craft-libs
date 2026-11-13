//! Reading and validating a theme file.

use std::collections::BTreeMap;

use serde_json::{Map, Value};

use super::{EDGE, ErrorKind, FormatError, FormatErrors, KNOB, LENGTH, MAX_THEME_BYTES, OFFSET, ROWS, SCHEMA, SCHEMA_VERSION, ThemeFile, WELLS};
use crate::color::Rgba;
use crate::scale::Radius;
use crate::theme::{
    AppOverrides, ContrastIntent, EdgeSeed, ModePins, ModeSeeds, Pins, RadiusOffset, SeedOverrides, Seeds, ShapeSeed, ThemeDef, Variants,
};
use crate::tokens::{ColorToken, KnobShape, RadiusSet, RowStyle, Status, WellShape};
use crate::{AppId, ThemeId};

/// Parses and validates a theme file. Unknown keys are allowed anywhere except in colour maps, and
/// kept for [`write`](super::write). Every field error is reported, not
/// only the first.
pub fn parse(bytes: &[u8]) -> Result<ThemeFile, FormatErrors> {
    let stop = |path: &[&str], kind| Err(FormatErrors(vec![FormatError { path: path.iter().map(|s| (*s).to_owned()).collect(), kind }]));
    if bytes.len() > MAX_THEME_BYTES {
        return stop(&[], ErrorKind::TooLarge { len: bytes.len() });
    }
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let value: Value = match serde_json::from_slice(bytes) {
        Ok(v) => v,
        Err(e) => return stop(&[], ErrorKind::Json(e.to_string())),
    };
    let Value::Object(raw) = value else { return stop(&[], ErrorKind::NotObject) };
    match raw.get("schema") {
        Some(Value::String(s)) if s == SCHEMA => {}
        Some(Value::String(s)) => return stop(&["schema"], ErrorKind::OtherSchema { schema: s.clone() }),
        Some(_) => return stop(&["schema"], ErrorKind::WrongType { expected: "a string" }),
        None => return stop(&["schema"], ErrorKind::Missing),
    }
    match raw.get("schema_version").and_then(Value::as_u64) {
        Some(SCHEMA_VERSION) => {}
        Some(v) if v > SCHEMA_VERSION => return stop(&["schema_version"], ErrorKind::TooNew { schema_version: v }),
        _ => return stop(&["schema_version"], ErrorKind::BadSchemaVersion),
    }

    let mut p = Parser::new();
    let theme = p.theme(&raw);
    match theme {
        Some(theme) if p.errors.is_empty() => {
            let cycles = theme.reference_errors();
            if cycles.is_empty() { Ok(ThemeFile { theme, raw }) } else { Err(FormatErrors(cycles)) }
        }
        _ => Err(FormatErrors(p.errors)),
    }
}

/// The token path inside a reference, `"{accent.solid}"` giving `accent.solid`.
fn reference_name(s: &str) -> Option<&str> {
    s.strip_prefix('{')?.strip_suffix('}')
}

/// Collects errors while walking the file; `path` is the position being read.
pub(super) struct Parser {
    pub(super) errors: Vec<FormatError>,
    path: Vec<String>,
}

impl Parser {
    pub(super) fn new() -> Parser {
        Parser { errors: Vec::new(), path: Vec::new() }
    }

    fn fail(&mut self, kind: ErrorKind) {
        self.errors.push(FormatError { path: self.path.clone(), kind });
    }

    /// Runs `f` with `key` pushed onto the path.
    fn at<T>(&mut self, key: &str, f: impl FnOnce(&mut Self) -> T) -> T {
        self.path.push(key.to_owned());
        let out = f(self);
        self.path.pop();
        out
    }

    /// The object under `key`: `None` when absent or null; an error when anything else.
    fn optional_object<'a>(&mut self, obj: &'a Map<String, Value>, key: &str) -> Option<&'a Map<String, Value>> {
        match obj.get(key) {
            None | Some(Value::Null) => None,
            Some(Value::Object(o)) => Some(o),
            Some(_) => {
                self.at(key, |p| p.fail(ErrorKind::WrongType { expected: "an object" }));
                None
            }
        }
    }

    /// The object under `key`; missing is an error too.
    fn required_object<'a>(&mut self, obj: &'a Map<String, Value>, key: &str) -> Option<&'a Map<String, Value>> {
        if obj.get(key).is_none_or(Value::is_null) {
            self.at(key, |p| p.fail(ErrorKind::Missing));
            return None;
        }
        self.optional_object(obj, key)
    }

    fn colour_at(&mut self, obj: &Map<String, Value>, key: &str) -> Option<Rgba> {
        match obj.get(key) {
            None | Some(Value::Null) => {
                self.at(key, |p| p.fail(ErrorKind::Missing));
                None
            }
            Some(v) => self.at(key, |p| p.colour(v)),
        }
    }

    fn colour(&mut self, v: &Value) -> Option<Rgba> {
        let c = v.as_str().and_then(Rgba::from_hex);
        if c.is_none() {
            self.fail(ErrorKind::BadColour);
        }
        c
    }

    fn theme(&mut self, raw: &Map<String, Value>) -> Option<ThemeDef> {
        let id = self.at("id", |p| match raw.get("id") {
            None => {
                p.fail(ErrorKind::Missing);
                None
            }
            Some(Value::String(s)) => ThemeId::parse(s).map_err(|e| p.fail(ErrorKind::BadId(e))).ok(),
            Some(_) => {
                p.fail(ErrorKind::WrongType { expected: "a string" });
                None
            }
        });
        let name = self.at("name", |p| match raw.get("name") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(_) => {
                p.fail(ErrorKind::WrongType { expected: "a string" });
                None
            }
        });
        let version = self.at("version", |p| match raw.get("version") {
            None | Some(Value::Null) => 1,
            Some(v) => v.as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or_else(|| {
                p.fail(ErrorKind::BadValue { expected: "a non-negative integer" });
                1
            }),
        });
        let variants =
            self.choice(raw, "variants", Variants::Both, &[("both", Variants::Both), ("light", Variants::LightOnly), ("dark", Variants::DarkOnly)]);
        let contrast_intent =
            self.choice(raw, "contrast_intent", ContrastIntent::Standard, &[("standard", ContrastIntent::Standard), ("low", ContrastIntent::Low)]);
        let seeds = self.seeds(raw);
        let shape = self.shape(raw);
        let pins = self.mode_pins(raw, variants, true);
        let mode_seeds = self.mode_seeds(raw, variants);
        let high_contrast = match self.optional_object(raw, "high_contrast") {
            Some(hc) => self.at("high_contrast", |p| p.mode_pins(hc, variants, false)),
            None => Some(ModePins::default()),
        };
        let apps = self.apps(raw, variants);
        let id = id?;
        Some(ThemeDef {
            name: name.unwrap_or_else(|| id.as_str().to_owned()),
            id,
            version,
            seeds: seeds?,
            mode_seeds: mode_seeds?,
            shape: shape?,
            variants,
            pins: pins?,
            high_contrast: high_contrast?,
            contrast_intent,
            apps: apps?,
        })
    }

    /// A string-valued key restricted to `options`; `default` when absent.
    fn choice<T: Copy>(&mut self, obj: &Map<String, Value>, key: &str, default: T, options: &[(&str, T)]) -> T {
        match obj.get(key) {
            None | Some(Value::Null) => default,
            Some(Value::String(s)) => match options.iter().find(|(name, _)| name == s) {
                Some(&(_, v)) => v,
                None => {
                    self.at(key, |p| p.fail(ErrorKind::BadValue { expected: "one of the listed names" }));
                    default
                }
            },
            Some(_) => {
                self.at(key, |p| p.fail(ErrorKind::WrongType { expected: "a string" }));
                default
            }
        }
    }

    fn seeds(&mut self, raw: &Map<String, Value>) -> Option<Seeds> {
        let seeds = self.required_object(raw, "seeds")?;
        self.at("seeds", |p| p.seed_values(seeds))
    }

    fn seed_values(&mut self, seeds: &Map<String, Value>) -> Option<Seeds> {
        let neutral = self.colour_at(seeds, "neutral");
        let accent = self.colour_at(seeds, "accent");
        let status = self.required_object(seeds, "status").and_then(|st| {
            self.at("status", |p| {
                let good = p.colour_at(st, "good");
                let warn = p.colour_at(st, "warn");
                let bad = p.colour_at(st, "bad");
                let info = p.colour_at(st, "info");
                Some(Status { good: good?, warn: warn?, bad: bad?, info: info? })
            })
        });
        Some(Seeds { neutral: neutral?, accent: accent?, status: status? })
    }

    /// `Some(None)` is a theme without its own shape.
    fn shape(&mut self, raw: &Map<String, Value>) -> Option<Option<ShapeSeed>> {
        let Some(shape) = self.optional_object(raw, "shape") else { return Some(None) };
        self.at("shape", |p| {
            let radius = match p.optional_object(shape, "radius") {
                Some(r) => p.at("radius", |p| p.radius_set(r)).map(Some),
                None => Some(None),
            };
            let radius_offset = match p.optional_object(shape, "radius_offset") {
                Some(r) => p.at("radius_offset", |p| p.radius_offset(r)).map(Some),
                None => Some(None),
            };
            let button_radius = match shape.get("button_radius") {
                None | Some(Value::Null) => Some(None),
                Some(v) => p.at("button_radius", |p| p.length(v)).map(Some),
            };
            let edge = p.at("edge", |p| match shape.get("edge") {
                None | Some(Value::Null) => Some(None),
                Some(v) => p.edge(v).map(Some),
            });
            let rows = p.at("rows", |p| match shape.get("rows") {
                None | Some(Value::Null) => Some(None),
                Some(Value::String(s)) if s == "divided" => Some(Some(RowStyle::Divided)),
                Some(Value::String(s)) if s == "rounded" => Some(Some(RowStyle::Rounded)),
                Some(_) => {
                    p.fail(ErrorKind::BadValue { expected: ROWS });
                    None
                }
            });
            let knob = p.at("knob", |p| match shape.get("knob") {
                None | Some(Value::Null) => Some(None),
                Some(Value::String(s)) if s == "round" => Some(Some(KnobShape::Round)),
                Some(Value::String(s)) if s == "tab" => Some(Some(KnobShape::Tab)),
                Some(_) => {
                    p.fail(ErrorKind::BadValue { expected: KNOB });
                    None
                }
            });
            let wells = p.at("wells", |p| match shape.get("wells") {
                None | Some(Value::Null) => Some(None),
                Some(Value::String(s)) if s == "square" => Some(Some(WellShape::Square)),
                Some(Value::String(s)) if s == "round" => Some(Some(WellShape::Round)),
                Some(_) => {
                    p.fail(ErrorKind::BadValue { expected: WELLS });
                    None
                }
            });
            Some(Some(ShapeSeed {
                radius: radius?,
                radius_offset: radius_offset?,
                button_radius: button_radius?,
                edge: edge?,
                rows: rows?,
                knob: knob?,
                wells: wells?,
            }))
        })
    }

    /// The radii; all but `pill` are required, and `pill` defaults to fully round.
    fn radius_set(&mut self, r: &Map<String, Value>) -> Option<RadiusSet> {
        let mut get = |key: &str| {
            self.at(key, |p| match r.get(key) {
                None | Some(Value::Null) => {
                    p.fail(ErrorKind::Missing);
                    None
                }
                Some(v) => p.length(v),
            })
        };
        let (control, surface, window, panel, button) = (get("control"), get("surface"), get("window"), get("panel"), get("button"));
        let pill = match r.get("pill") {
            None | Some(Value::Null) => Radius::Full.px(),
            Some(v) => self.at("pill", |p| p.length(v))?,
        };
        Some(RadiusSet { sm: 0.0, control: control?, surface: surface?, window: window?, panel: panel?, button: button?, pill })
    }

    /// The radius offsets; each key is optional and defaults to 0.
    fn radius_offset(&mut self, r: &Map<String, Value>) -> Option<RadiusOffset> {
        let mut get = |key: &str| match r.get(key) {
            None | Some(Value::Null) => Some(0.0),
            Some(v) => self.at(key, |p| p.offset(v)),
        };
        let (control, surface, window) = (get("control"), get("surface"), get("window"));
        Some(RadiusOffset { control: control?, surface: surface?, window: window? })
    }

    /// A signed length in logical pixels.
    fn offset(&mut self, v: &Value) -> Option<f32> {
        let out = v.as_f64().filter(|x| x.is_finite()).map(|x| x as f32);
        if out.is_none() {
            self.fail(ErrorKind::BadValue { expected: OFFSET });
        }
        out
    }

    /// A length in logical pixels: a number of 0 or more, or `"full"`.
    fn length(&mut self, v: &Value) -> Option<f32> {
        let out = match v {
            Value::String(s) if s == "full" => Some(Radius::Full.px()),
            Value::Number(n) => n.as_f64().filter(|x| x.is_finite() && *x >= 0.0).map(|x| x as f32),
            _ => None,
        };
        if out.is_none() {
            self.fail(ErrorKind::BadValue { expected: LENGTH });
        }
        out
    }

    /// `"flat"` or `{ "bevel": { "width": n } }`.
    fn edge(&mut self, v: &Value) -> Option<EdgeSeed> {
        let width = match v {
            Value::String(s) if s == "flat" => return Some(EdgeSeed::Flat),
            Value::Object(o) => o.get("bevel").and_then(|b| b.get("width")).and_then(Value::as_f64).filter(|w| w.is_finite() && *w > 0.0),
            _ => None,
        };
        if width.is_none() {
            self.fail(ErrorKind::BadValue { expected: EDGE });
        }
        width.map(|w| EdgeSeed::Bevel { width: w as f32 })
    }

    /// The `light` and `dark` maps of `obj`. A map for a mode `variants` leaves out is an error
    /// unless it is empty.
    fn mode_pins(&mut self, obj: &Map<String, Value>, variants: Variants, allow_seeds: bool) -> Option<ModePins> {
        let map = |p: &mut Self, key: &str, present: bool| {
            let m = p.optional_object(obj, key);
            p.at(key, |p| match m {
                Some(m) if !present => {
                    if !m.is_empty() {
                        p.fail(ErrorKind::NoSuchMode);
                    }
                    Some(Pins::default())
                }
                Some(m) => p.pins(m, allow_seeds),
                None => Some(Pins::default()),
            })
        };
        let light = map(self, "light", variants != Variants::DarkOnly);
        let dark = map(self, "dark", variants != Variants::LightOnly);
        Some(ModePins { light: light?, dark: dark? })
    }

    /// The `seeds` object of the theme's `light` and `dark` maps: any seed field it names replaces
    /// the theme's value in that mode.
    fn mode_seeds(&mut self, obj: &Map<String, Value>, variants: Variants) -> Option<ModeSeeds> {
        let one = |p: &mut Self, key: &str, present: bool| -> Option<SeedOverrides> {
            let Some(Value::Object(map)) = obj.get(key) else { return Some(SeedOverrides::default()) };
            let Some(seeds) = p.at(key, |p| p.optional_object(map, "seeds")) else { return Some(SeedOverrides::default()) };
            p.at(key, |p| {
                p.at("seeds", |p| {
                    if !present && !seeds.is_empty() {
                        p.fail(ErrorKind::NoSuchMode);
                        return None;
                    }
                    let neutral = p.optional_colour(seeds, "neutral");
                    let accent = p.optional_colour(seeds, "accent");
                    let status = match p.optional_object(seeds, "status") {
                        None => Some(Status { good: None, warn: None, bad: None, info: None }),
                        Some(st) => p.at("status", |p| {
                            let good = p.optional_colour(st, "good");
                            let warn = p.optional_colour(st, "warn");
                            let bad = p.optional_colour(st, "bad");
                            let info = p.optional_colour(st, "info");
                            Some(Status { good: good?, warn: warn?, bad: bad?, info: info? })
                        }),
                    };
                    Some(SeedOverrides { neutral: neutral?, accent: accent?, status: status? })
                })
            })
        };
        let light = one(self, "light", variants != Variants::DarkOnly);
        let dark = one(self, "dark", variants != Variants::LightOnly);
        Some(ModeSeeds { light: light?, dark: dark? })
    }

    /// `Some(None)` when `key` is absent; `None` when it is present and not a colour.
    fn optional_colour(&mut self, obj: &Map<String, Value>, key: &str) -> Option<Option<Rgba>> {
        match obj.get(key) {
            None | Some(Value::Null) => Some(None),
            Some(v) => self.at(key, |p| p.colour(v)).map(Some),
        }
    }

    /// A map of colour paths to colours. Surfaces must be opaque.
    fn pins(&mut self, m: &Map<String, Value>, allow_seeds: bool) -> Option<Pins> {
        let mut pins = Pins::default();
        let mut ok = true;
        for (key, v) in m {
            if key == "seeds" {
                if !allow_seeds {
                    self.at(key, |p| p.fail(ErrorKind::SeedsNotAllowed));
                    ok = false;
                }
                continue;
            }
            ok &= self.at(key, |p| {
                let Some(token) = ColorToken::from_path(key) else {
                    p.fail(ErrorKind::UnknownToken);
                    return false;
                };
                if let Some(name) = v.as_str().and_then(reference_name) {
                    return match ColorToken::from_path(name) {
                        None => {
                            p.fail(ErrorKind::UnknownReference { name: name.to_owned() });
                            false
                        }
                        Some(target) => {
                            pins.refs.insert(token, target);
                            true
                        }
                    };
                }
                match p.colour(v) {
                    Some(c) if c.a() < 255 && token.is_opaque_surface() => {
                        p.fail(ErrorKind::NotOpaque);
                        false
                    }
                    Some(c) => {
                        pins.colors.insert(token, c);
                        true
                    }
                    None => false,
                }
            });
        }
        ok.then_some(pins)
    }

    fn apps(&mut self, raw: &Map<String, Value>, variants: Variants) -> Option<BTreeMap<AppId, AppOverrides>> {
        let Some(m) = self.optional_object(raw, "apps") else { return Some(BTreeMap::new()) };
        self.at("apps", |p| {
            let mut out = BTreeMap::new();
            let mut ok = true;
            for (key, v) in m {
                ok &= p.at(key, |p| {
                    let Ok(id) = AppId::parse(key) else {
                        p.fail(ErrorKind::BadAppId);
                        return false;
                    };
                    let Value::Object(app) = v else {
                        p.fail(ErrorKind::WrongType { expected: "an object" });
                        return false;
                    };
                    // Sizes belong to the app (`AppSizes`); a theme sets none.
                    if app.contains_key("sizes") {
                        p.at("sizes", |p| p.fail(ErrorKind::UnknownToken));
                        return false;
                    }
                    let pins = p.mode_pins(app, variants, false);
                    let high_contrast = match p.optional_object(app, "high_contrast") {
                        Some(hc) => p.at("high_contrast", |p| p.mode_pins(hc, variants, false)),
                        None => Some(ModePins::default()),
                    };
                    match (pins, high_contrast) {
                        (Some(pins), Some(high_contrast)) => {
                            out.insert(id, AppOverrides { pins, high_contrast });
                            true
                        }
                        _ => false,
                    }
                });
            }
            ok.then_some(out)
        })
    }
}
