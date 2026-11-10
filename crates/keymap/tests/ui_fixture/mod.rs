//! The catalog the dialog tests and screenshots use.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeSet;
use std::rc::Rc;

use craft_keymap::ui::{PLURALS, STRINGS, Text};
use craft_keymap::{Bindable, Binding, Key, Keymap, Kind, Platform, Scope, Scopes};

fn binding(text: &str) -> Binding {
    Binding::parse(text).unwrap_or_else(|e| panic!("{text}: {e}"))
}

pub fn fixture(platform: Platform) -> Keymap {
    let tool = Kind::Tool { group: Some("Marquee".into()) };
    let path = |p: &[&str]| p.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let timeline = Scope::new("Timeline").unwrap_or_else(Scope::app);
    let catalog = vec![
        Bindable::new("file.save", "Save", Scope::app()).with_path(path(&["File", "Save"])).with_default(binding("Cmd+S")),
        Bindable::new("file.saveAs", "Save As…", Scope::app()).with_path(path(&["File", "Save As…"])).with_default(binding("Cmd+Shift+S")),
        Bindable::new("file.export", "Export…", Scope::app()).with_path(path(&["File", "Export…"])).with_default(binding("Cmd+E")),
        Bindable::new("object.front", "Bring to Front", Scope::app()).with_path(path(&["Object", "Arrange", "Bring to Front"])),
        Bindable::new("tool.marquee", "Marquee Tool", Scope::app()).with_kind(tool.clone()).with_default(binding("M")),
        Bindable::new("tool.ellipse", "Elliptical Marquee Tool", Scope::app()).with_kind(tool).with_default(binding("M")),
        Bindable::new("timeline.loop", "Loop", timeline).with_default(binding("L")),
        Bindable::new("type.bold", "Bold", Scope::app()).with_default(binding("Text: Cmd+B")),
    ];
    Keymap::new(catalog, Scopes::new().with_panel("Timeline").with_context("Text"), platform)
}

/// `km` with a command whose label and menu path are far too long for a row.
#[allow(dead_code)]
pub fn with_long_label(km: Keymap) -> Keymap {
    let long = "Distribute Selected Objects Evenly Along the Active Guide Path and Keep Their Spacing".to_string();
    let mut catalog = km.catalog().to_vec();
    catalog.insert(
        3,
        Bindable::new("object.distribute", long.as_str(), Scope::app())
            .with_path(vec!["Object".into(), "Arrange and Distribute Along a Path".into(), long.clone()])
            .with_default(binding("Cmd+Alt+Shift+D"))
            .with_default(binding("Ctrl+Alt+Shift+F12")),
    );
    Keymap::new(catalog, Scopes::new().with_panel("Timeline").with_context("Text"), km.platform())
}

/// A pseudo translation: every string about 40 % longer, accented, between right-to-left marks and
/// in `⟦…⟧`, so text drawn without the marks never went through [`Text`]. Placeholders stay as
/// they are. Labels and path parts are marked too. Looked-up dialog strings missing from
/// `STRINGS` or `PLURALS` are collected in `unlisted`.
#[allow(dead_code)]
#[derive(Default)]
pub struct Pseudo {
    pub unlisted: Rc<RefCell<BTreeSet<String>>>,
    /// Lay the dialog out right to left.
    pub rtl: bool,
}

#[allow(dead_code)]
impl Pseudo {
    pub fn mark(source: &str) -> String {
        let mut out = String::from("⟦\u{200F}");
        let mut in_placeholder = false;
        let mut letters = 0;
        for c in source.chars() {
            match c {
                '{' => in_placeholder = true,
                '}' => in_placeholder = false,
                _ => {}
            }
            let accented = match c {
                _ if in_placeholder => c,
                'a' => 'á',
                'e' => 'ë',
                'i' => 'ï',
                'o' => 'ö',
                'u' => 'ü',
                'A' => 'Å',
                'E' => 'É',
                'O' => 'Ø',
                _ => c,
            };
            if !in_placeholder && c.is_alphabetic() {
                letters += 1;
            }
            out.push(accented);
        }
        out.extend(std::iter::repeat_n('~', letters * 2 / 5));
        out.push_str("\u{200F}⟧");
        out
    }
}

impl Text for Pseudo {
    fn ui<'a>(&'a self, source: &'a str) -> Cow<'a, str> {
        if !STRINGS.contains(&source) {
            self.unlisted.borrow_mut().insert(source.to_string());
        }
        Pseudo::mark(source).into()
    }

    fn plural(&self, n: u64, one: &str, other: &str) -> String {
        if !PLURALS.contains(&(one, other)) {
            self.unlisted.borrow_mut().insert(format!("{one}|{other}"));
        }
        Pseudo::mark(if n == 1 { one } else { other }).replace("{n}", &n.to_string())
    }

    fn command<'a>(&'a self, b: &'a Bindable) -> Cow<'a, str> {
        Pseudo::mark(&b.label).into()
    }

    fn segment<'a>(&'a self, s: &'a str) -> Cow<'a, str> {
        Pseudo::mark(s).into()
    }

    /// Key names spelled out in words (Escape, Space), on every platform.
    fn key(&self, k: Key, _p: Platform) -> Option<String> {
        let name = k.to_string();
        name.chars().any(|c| c.is_ascii_lowercase()).then(|| Pseudo::mark(&name))
    }

    fn rtl(&self) -> bool {
        self.rtl
    }
}

/// English laid out right to left, to look at the mirrored layout.
#[allow(dead_code)]
pub struct EnglishRtl;

impl Text for EnglishRtl {
    fn ui<'a>(&'a self, source: &'a str) -> Cow<'a, str> {
        source.into()
    }

    fn plural(&self, n: u64, one: &str, other: &str) -> String {
        (if n == 1 { one } else { other }).replace("{n}", &n.to_string())
    }

    fn rtl(&self) -> bool {
        true
    }

    /// The words stay in English order; spaces at either end change sides, as in a right-to-left
    /// paragraph.
    fn visual(&self, s: &str) -> String {
        let core = s.trim();
        if core.is_empty() {
            return s.to_string();
        }
        let start = s.len() - s.trim_start().len();
        let end = s.trim_end().len();
        format!("{}{core}{}", &s[end..], &s[..start])
    }
}
