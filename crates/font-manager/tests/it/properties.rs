//! The font picker's Font Properties filter over synthetic faces built in memory: each property comes from the face's
//! own tables and outlines, the properties combine, and the rows show only when the host asks for them.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft_font_manager::craft_font_catalog::{Limits, Os, Source};
use craft_font_manager::picker::{
    FamilyInfos, FilterLayout, FontCase, FontFigures, FontFilters, FontPicker, FontPickerSettings, FontProperties, Grade, family_rows,
};
use craft_font_manager::{Catalog, FontEnv};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Fixture, Room, trial};
use egui::{Key, Modifiers};
use write_fonts::FontBuilder;
use write_fonts::read::tables::glyf::CurvePoint;
use write_fonts::tables::cmap::Cmap;
use write_fonts::tables::fvar::{AxisInstanceArrays, Fvar, InstanceRecord, VariationAxisRecord};
use write_fonts::tables::glyf::{Contour, GlyfLocaBuilder, Glyph, SimpleGlyph};
use write_fonts::tables::gvar::{GlyphDelta, GlyphDeltas, GlyphVariations, Gvar, Tent};
use write_fonts::tables::head::Head;
use write_fonts::tables::hhea::Hhea;
use write_fonts::tables::hmtx::{Hmtx, LongMetric};
use write_fonts::tables::maxp::Maxp;
use write_fonts::tables::name::{Name, NameRecord};
use write_fonts::tables::os2::{Os2, SelectionFlags};
use write_fonts::types::{F2Dot14, FWord, Fixed, GlyphId, NameId, Tag};

/// What a synthetic face states and draws, in units of a 1000-unit em.
#[derive(Clone)]
struct Face {
    family: &'static str,
    style: &'static str,
    weight: u16,
    /// `usWidthClass`: 5 is normal, 3 condensed, 7 expanded.
    width: u16,
    /// `sxHeight`; `None` leaves the x-height to the top of `x`.
    x_height: Option<i16>,
    /// The top of the small letters (`x` and `o` included).
    small_top: i16,
    /// The top of the capitals.
    cap_top: i16,
    /// The top of `0`.
    zero_top: i16,
    /// The stroke of `o` at its sides and at its top and bottom.
    o_strokes: (i16, i16),
    /// Each small letter maps to its capital's glyph.
    caps_only: bool,
    /// The top of `o`'s outer contour is a quadratic curve whose control point stands this far above it, so the
    /// curve rises half as far.
    o_bulge: i16,
    /// A `wght` axis from 100 (the default) to 900 with a Regular instance at 400; at 900 the top of `x` rises by
    /// this much.
    x_rise: Option<i16>,
}

fn face(family: &'static str) -> Face {
    Face {
        family,
        style: "Regular",
        weight: 400,
        width: 5,
        x_height: None,
        small_top: 480,
        cap_top: 700,
        zero_top: 700,
        o_strokes: (80, 80),
        caps_only: false,
        o_bulge: 0,
        x_rise: None,
    }
}

/// A closed rectangle contour from (`x0`, `y0`) to (`x1`, `y1`).
fn rect(x0: i16, y0: i16, x1: i16, y1: i16) -> Contour {
    let at = |x, y| CurvePoint { x, y, on_curve: true };
    Contour::from(vec![at(x0, y0), at(x0, y1), at(x1, y1), at(x1, y0)])
}

fn glyph(contours: Vec<Contour>) -> Glyph {
    let mut simple = SimpleGlyph { contours, ..SimpleGlyph::default() };
    simple.recompute_bounding_box();
    Glyph::Simple(simple)
}

/// The font file of `f`: `name`, `OS/2`, `head`, `hhea`, `hmtx`, `maxp`, `cmap` and outlines for A to Z, a to z and
/// `0`. Small letters are boxes up to `small_top`, `o` a box ring with `o_strokes`, capitals boxes up to `cap_top`.
fn font(f: &Face) -> Vec<u8> {
    let mut glyphs = vec![Glyph::Empty];
    let mut map: Vec<(char, GlyphId)> = Vec::new();
    for capital in 'A'..='Z' {
        map.push((capital, GlyphId::new(glyphs.len() as u32)));
        glyphs.push(glyph(vec![rect(50, 0, 550, f.cap_top)]));
    }
    for (capital, small) in ('A'..='Z').zip('a'..='z') {
        if f.caps_only {
            let id = map.iter().find(|(c, _)| *c == capital).map(|(_, id)| *id).unwrap_or(GlyphId::NOTDEF);
            map.push((small, id));
            continue;
        }
        map.push((small, GlyphId::new(glyphs.len() as u32)));
        let drawn = if small == 'o' {
            let (sides, ends) = f.o_strokes;
            let point = |x, y, on_curve| CurvePoint { x, y, on_curve };
            let top = f.small_top;
            let outer =
                vec![point(50, 0, true), point(50, top, true), point(275, top + f.o_bulge, false), point(500, top, true), point(500, 0, true)];
            vec![Contour::from(outer), rect(50 + sides, ends, 500 - sides, top - ends)]
        } else {
            vec![rect(50, 0, 450, f.small_top)]
        };
        glyphs.push(glyph(drawn));
    }
    map.push(('0', GlyphId::new(glyphs.len() as u32)));
    glyphs.push(glyph(vec![rect(50, 0, 450, f.zero_top)]));

    let mut outlines = GlyfLocaBuilder::new();
    for g in &glyphs {
        outlines.add_glyph(g).expect("a valid glyph");
    }
    let (glyf, loca, loca_format) = outlines.build();
    let count = glyphs.len() as u16;

    let mut records = Vec::new();
    let mut add = |id: u16, text: String| records.push(NameRecord::new(3, 1, 0x409, NameId::new(id), text.into()));
    add(1, f.family.into());
    add(2, f.style.into());
    add(4, format!("{} {}", f.family, f.style));
    add(6, format!("{}-{}", f.family.replace(' ', ""), f.style));
    if f.x_rise.is_some() {
        add(255, "Weight".into());
        add(256, "Regular".into());
    }
    records.sort_by_key(|r| r.name_id);

    let mut builder = FontBuilder::new();
    builder.add_table(&Name::new(records)).expect("name");
    let os2 = Os2 {
        us_weight_class: f.weight,
        us_width_class: f.width,
        fs_selection: SelectionFlags::REGULAR,
        ul_code_page_range_1: Some(1),
        ul_code_page_range_2: Some(0),
        sx_height: Some(f.x_height.unwrap_or(0)),
        s_cap_height: Some(f.cap_top),
        us_default_char: Some(0),
        us_break_char: Some(32),
        us_max_context: Some(0),
        ..Default::default()
    };
    builder.add_table(&os2).expect("OS/2");
    let head = Head { units_per_em: 1000, index_to_loc_format: loca_format as i16, ..Default::default() };
    builder.add_table(&head).expect("head");
    let hhea = Hhea { ascender: FWord::new(800), descender: FWord::new(-200), number_of_h_metrics: count, ..Default::default() };
    builder.add_table(&hhea).expect("hhea");
    builder.add_table(&Hmtx::new(vec![LongMetric::new(600, 50); usize::from(count)], Vec::new())).expect("hmtx");
    builder.add_table(&Maxp::new(count)).expect("maxp");
    builder.add_table(&Cmap::from_mappings(map.clone()).expect("a cmap")).expect("cmap");
    builder.add_table(&glyf).expect("glyf");
    builder.add_table(&loca).expect("loca");
    if let Some(rise) = f.x_rise {
        let axis = VariationAxisRecord {
            axis_tag: Tag::new(b"wght"),
            min_value: Fixed::from_f64(100.0),
            default_value: Fixed::from_f64(100.0),
            max_value: Fixed::from_f64(900.0),
            flags: 0,
            axis_name_id: NameId::new(255),
        };
        let regular =
            InstanceRecord { subfamily_name_id: NameId::new(256), flags: 0, coordinates: vec![Fixed::from_f64(400.0)], post_script_name_id: None };
        builder.add_table(&Fvar::new(AxisInstanceArrays::new(vec![axis], vec![regular]))).expect("fvar");
        let x = map.iter().find(|(c, _)| *c == 'x').map(|(_, id)| *id).expect("x is mapped");
        // The rectangle's corners in drawing order (its top is the second and third), then the four phantom points.
        let deltas: Vec<GlyphDelta> = [0, rise, rise, 0, 0, 0, 0, 0].into_iter().map(|dy| GlyphDelta::required(0, dy)).collect();
        let variations = (0..u32::from(count))
            .map(GlyphId::new)
            .map(|id| {
                let deltas = if id == x { vec![GlyphDeltas::new(vec![Tent::new(F2Dot14::ONE, None)], deltas.clone())] } else { Vec::new() };
                GlyphVariations::new(id, deltas)
            })
            .collect();
        builder.add_table(&Gvar::new(variations, 1).expect("a gvar")).expect("gvar");
    }
    builder.build()
}

fn catalog_of(faces: &[Face]) -> Arc<Catalog> {
    let catalog = Arc::new(Catalog::new(FontEnv { os: Os::None, ..FontEnv::default() }).with_font_service(false));
    for f in faces {
        catalog.add_bytes(font(f), Source::Bundled, &Limits::default()).expect("a font file");
    }
    catalog.rescan();
    catalog
}

/// The families of `catalog` the list keeps under `properties`.
fn kept(catalog: &Arc<Catalog>, properties: FontProperties) -> Vec<String> {
    let mut filters = FontFilters::default();
    filters.properties = properties;
    family_rows(catalog, &FamilyInfos::of(catalog), "", None, &filters, &[], "en").into_iter().map(|row| row.family).collect()
}

fn with(edit: impl FnOnce(&mut FontProperties)) -> FontProperties {
    let mut properties = FontProperties::default();
    edit(&mut properties);
    properties
}

/// Weight and width come from every face of a family: a family with a bold face is heavy as well as regular, and a
/// condensed face makes its family narrow.
#[trial]
fn weight_and_width_keep_the_families_with_a_face_of_that_grade() {
    let bold = Face { style: "Bold", weight: 700, ..face("Pair") };
    let condensed = Face { width: 3, ..face("Narrow Set") };
    let catalog = catalog_of(&[face("Pair"), bold, face("Solo"), condensed, Face { weight: 200, ..face("Hairline") }]);
    assert_eq!(kept(&catalog, with(|p| p.weight = Some(Grade::High))), ["Pair"], "only the family with a bold face is heavy");
    let regular = kept(&catalog, with(|p| p.weight = Some(Grade::Medium)));
    assert!(regular.contains(&"Pair".to_owned()) && regular.contains(&"Solo".to_owned()), "regular: {regular:?}");
    assert!(!regular.contains(&"Hairline".to_owned()), "a family of only a light face is not regular");
    assert_eq!(kept(&catalog, with(|p| p.weight = Some(Grade::Low))), ["Hairline"]);
    assert_eq!(kept(&catalog, with(|p| p.width = Some(Grade::Low))), ["Narrow Set"], "the condensed face makes its family narrow");
    assert!(kept(&catalog, with(|p| p.width = Some(Grade::High))).is_empty(), "no face is expanded");
}

/// The x-height is `OS/2`'s `sxHeight` when the face states one, else the top of `x`; a face that states a tall
/// x-height is high even though its outlines are short.
#[trial]
fn height_reads_the_stated_x_height_else_the_top_of_x() {
    let catalog = catalog_of(&[
        Face { small_top: 400, ..face("Short") },
        Face { small_top: 560, ..face("Tall") },
        Face { small_top: 400, x_height: Some(560), ..face("Stated") },
    ]);
    assert_eq!(kept(&catalog, with(|p| p.height = Some(Grade::Low))), ["Short"]);
    assert_eq!(kept(&catalog, with(|p| p.height = Some(Grade::High))), ["Stated", "Tall"]);
}

/// A family whose regular face is a named instance is measured at that instance's location, not at the font's
/// default: the default's `x` is short and the Regular instance's is tall.
#[trial]
fn a_named_instance_is_measured_at_its_own_location() {
    let catalog = catalog_of(&[Face { style: "Thin", weight: 100, small_top: 400, x_rise: Some(400), ..face("Flex") }]);
    assert_eq!(kept(&catalog, with(|p| p.height = Some(Grade::High))), ["Flex"]);
    assert!(kept(&catalog, with(|p| p.height = Some(Grade::Low))).is_empty(), "the default's short x does not count");
}

/// Contrast compares the sides of `o` with its top and bottom, whichever is thicker. A curved edge counts as far as the
/// curve reaches, not as far as its control point stands.
#[trial]
fn contrast_compares_the_strokes_of_o() {
    let catalog = catalog_of(&[
        face("Even"),
        Face { o_strokes: (100, 20), ..face("Sharp") },
        Face { o_strokes: (20, 40), ..face("Reversed") },
        Face { o_strokes: (40, 40), o_bulge: 160, ..face("Domed") },
    ]);
    assert_eq!(kept(&catalog, with(|p| p.contrast = Some(Grade::Low))), ["Even"]);
    let medium = kept(&catalog, with(|p| p.contrast = Some(Grade::Medium)));
    assert_eq!(medium, ["Domed", "Reversed"], "strokes twice as thick on the ends, the domed top's by the curve's height");
    assert_eq!(kept(&catalog, with(|p| p.contrast = Some(Grade::High))), ["Sharp"]);
}

/// A face whose small letters draw as its capitals is caps only; oldstyle figures stand near the x-height.
#[trial]
fn case_and_figures_read_the_outlines() {
    let catalog = catalog_of(&[face("Plain"), Face { caps_only: true, ..face("Titling") }, Face { zero_top: 500, ..face("Text Figures") }]);
    assert_eq!(kept(&catalog, with(|p| p.case = Some(FontCase::CapsOnly))), ["Titling"]);
    let standard = kept(&catalog, with(|p| p.case = Some(FontCase::Standard)));
    assert_eq!(standard, ["Plain", "Text Figures"]);
    assert_eq!(kept(&catalog, with(|p| p.figures = Some(FontFigures::Oldstyle))), ["Text Figures"]);
    let lining = kept(&catalog, with(|p| p.figures = Some(FontFigures::Lining)));
    assert_eq!(lining, ["Plain"], "a caps-only face has no x-height below its capitals to place its figures by");
}

/// Properties combine: only the families with every chosen property stay, and none chosen keeps them all.
#[trial]
fn chosen_properties_combine() {
    let catalog = catalog_of(&[
        Face { style: "Bold", weight: 700, zero_top: 500, ..face("Both") },
        Face { style: "Bold", weight: 700, ..face("Heavy Only") },
        Face { zero_top: 500, ..face("Oldstyle Only") },
    ]);
    let both = with(|p| {
        p.weight = Some(Grade::High);
        p.figures = Some(FontFigures::Oldstyle);
    });
    assert_eq!(kept(&catalog, both), ["Both"]);
    assert_eq!(kept(&catalog, FontProperties::default()).len(), 3, "nothing chosen keeps every family");
}

/// An open library list over two families, in `layout`, with the Font Properties rows when `properties` is set.
fn properties_rig(filters: Rc<RefCell<FontFilters>>, layout: FilterLayout, properties: bool) -> Fixture {
    properties_rig_on(filters, layout, properties, None)
}

fn properties_rig_on(filters: Rc<RefCell<FontFilters>>, layout: FilterLayout, properties: bool, jobs: Option<Arc<dyn craft_host::Jobs>>) -> Fixture {
    let catalog = catalog_of(&[face("Book"), Face { style: "Black", weight: 900, ..face("Poster") }]);
    let mut family = "Poster".to_owned();
    let mut settings = FontPickerSettings::default();
    let mut fx = Fixture::new(move |ui| {
        let mut filters = filters.borrow_mut();
        FontPicker::new("Font family", Bind::bound(&mut family), &catalog)
            .settings(Bind::bound(&mut settings))
            .filters(Bind::bound(&mut filters))
            .filter_layout(layout)
            .properties(properties)
            .show(ui);
    });
    if let Some(jobs) = jobs {
        let host = craft_host::Host::builder(craft_host::AppInfo { id: "trial", name: "Trial", version: "0" }).jobs(jobs).build();
        fx.host(Arc::new(host));
    }
    fx.in_room(Room::Window);
    fx.settle();
    fx.key(Key::Tab, Modifiers::NONE);
    fx.key(Key::ArrowDown, Modifiers::ALT);
    fx.settle();
    fx
}

/// Whether `family` is named or painted in the open list.
fn listed(fx: &Fixture, family: &str) -> bool {
    fx.by_label(family).is_some() || fx.texts().iter().any(|painted| painted.text == family)
}

#[trial]
fn outline_properties_load_through_the_hosts_jobs() {
    let jobs = Arc::new(craft_host::ManualJobs::new());
    let filters = Rc::new(RefCell::new(FontFilters::default()));
    filters.borrow_mut().properties.height = Some(Grade::Medium);
    let mut fx = properties_rig_on(filters, FilterLayout::Toolbar, true, Some(jobs.clone()));
    assert!(!listed(&fx, "Book"), "an unread outline does not count as a matching family");
    assert!(fx.texts().iter().any(|text| text.text == "Font information is loading."));
    assert!(jobs.waiting() > 0, "the host has not run the outline reads yet");
    jobs.run_all();
    fx.settle();
    jobs.run_all();
    fx.settle();
    assert!(listed(&fx, "Book"), "the matching family appears after its outline is read");
    assert!(!fx.texts().iter().any(|text| text.text == "Font information is loading."));
}

#[trial]
fn outline_properties_resume_after_the_host_drops_a_lane() {
    use std::sync::atomic::{AtomicBool, Ordering};
    struct Refusing {
        ready: AtomicBool,
        jobs: craft_host::ManualJobs,
    }
    impl craft_host::Jobs for Refusing {
        fn spawn(&self, name: &str, job: Box<dyn FnOnce() + Send>) {
            if self.ready.load(Ordering::Relaxed) {
                craft_host::Jobs::spawn(&self.jobs, name, job);
            }
        }
    }
    let jobs = Arc::new(Refusing { ready: AtomicBool::new(false), jobs: craft_host::ManualJobs::new() });
    let filters = Rc::new(RefCell::new(FontFilters::default()));
    filters.borrow_mut().properties.height = Some(Grade::Medium);
    let mut fx = properties_rig_on(filters, FilterLayout::Toolbar, true, Some(jobs.clone()));
    assert!(!listed(&fx, "Book"));
    jobs.ready.store(true, Ordering::Relaxed);
    fx.settle();
    assert!(jobs.jobs.waiting() > 0, "a later frame retries the dropped lane");
    jobs.jobs.run_all();
    fx.settle();
    assert!(listed(&fx, "Book"));
}

#[trial]
fn outline_properties_follow_replacement_host_jobs() {
    let first = Arc::new(craft_host::ManualJobs::new());
    let second = Arc::new(craft_host::ManualJobs::new());
    let filters = Rc::new(RefCell::new(FontFilters::default()));
    filters.borrow_mut().properties.height = Some(Grade::Medium);
    let mut fx = properties_rig_on(filters, FilterLayout::Toolbar, true, Some(first.clone()));
    assert!(first.waiting() > 0);
    let host = craft_host::Host::builder(craft_host::AppInfo { id: "replacement", name: "Replacement", version: "0" }).jobs(second.clone()).build();
    fx.host(Arc::new(host));
    fx.settle();
    assert!(second.waiting() > 0, "pending reads use the replacement host");
    second.run_all();
    fx.settle();
    assert!(listed(&fx, "Book"));
    first.run_all();
    fx.settle();
    assert!(listed(&fx, "Book"));
}

fn closed_property_field() -> (Fixture, Rc<RefCell<String>>, Arc<craft_host::ManualJobs>) {
    closed_property_field_from(&["Book", "Poster"])
}

fn closed_property_field_from(names: &[&'static str]) -> (Fixture, Rc<RefCell<String>>, Arc<craft_host::ManualJobs>) {
    let jobs = Arc::new(craft_host::ManualJobs::new());
    let family = Rc::new(RefCell::new("Book".to_owned()));
    let shown = Rc::clone(&family);
    let catalog = catalog_of(&names.iter().map(|name| face(name)).collect::<Vec<_>>());
    let mut filters = FontFilters::default();
    filters.properties.height = Some(Grade::Medium);
    let mut fx = Fixture::new(move |ui| {
        FontPicker::new("Font family", Bind::bound(&mut *shown.borrow_mut()), &catalog).filters(Bind::bound(&mut filters)).show(ui);
        craft_ui::buttons::Button::new("Next").show(ui);
    });
    let host = craft_host::Host::builder(craft_host::AppInfo { id: "trial", name: "Trial", version: "0" }).jobs(jobs.clone()).build();
    fx.host(Arc::new(host));
    fx.in_room(Room::Window);
    fx.settle();
    fx.key(Key::Tab, Modifiers::NONE);
    (fx, family, jobs)
}

#[trial]
fn closed_field_steps_wait_for_outline_properties() {
    let (mut fx, family, jobs) = closed_property_field();
    fx.key(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(family.borrow().as_str(), "Book");
    assert!(jobs.waiting() > 0);
    jobs.run_all();
    fx.settle();
    assert_eq!(family.borrow().as_str(), "Poster", "the deferred arrow uses the complete filtered order");
}

#[trial]
fn closed_field_wheel_waits_for_outline_properties() {
    use craft_ui::testing::pointer::{moved, wheel_lines};
    let (mut fx, family, jobs) = closed_property_field();
    let field = fx.by_label("Font family").expect("the family field");
    fx.frame(vec![moved(field.center()), wheel_lines(-1.0)]);
    assert_eq!(family.borrow().as_str(), "Book");
    assert!(jobs.waiting() > 0);
    jobs.run_all();
    fx.settle();
    assert_eq!(family.borrow().as_str(), "Poster", "the deferred wheel uses the complete filtered order");
}

#[trial]
fn closed_field_preserves_multiple_steps_while_properties_load() {
    let (mut fx, family, jobs) = closed_property_field_from(&["Book", "Middle", "Poster"]);
    fx.key(Key::ArrowDown, Modifiers::NONE);
    fx.key(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(family.borrow().as_str(), "Book");
    jobs.run_all();
    fx.settle();
    assert_eq!(family.borrow().as_str(), "Poster", "both queued arrows advance the filtered order");
}

#[trial]
fn closed_field_discards_pending_steps_after_focus_leaves() {
    let (mut fx, family, jobs) = closed_property_field();
    fx.key(Key::ArrowDown, Modifiers::NONE);
    let next = fx.by_label("Next").expect("another control");
    fx.click(&next);
    fx.settle();
    jobs.run_all();
    fx.settle();
    assert_eq!(family.borrow().as_str(), "Book", "leaving the field cancels the pending arrow");
}

#[trial]
fn closed_field_preserves_multiple_host_steps_while_properties_load() {
    use craft_font_manager::picker::{HostFontGroup, HostFontRow};
    let jobs = Arc::new(craft_host::ManualJobs::new());
    let current = Rc::new(RefCell::new("a".to_owned()));
    let shown = Rc::clone(&current);
    let catalog = catalog_of(&[face("Book")]);
    let rows = [HostFontRow::face("a", "A", None), HostFontRow::face("b", "B", None), HostFontRow::face("c", "C", None)];
    let mut family = String::new();
    let mut filters = FontFilters::default();
    filters.properties.height = Some(Grade::Medium);
    let mut fx = Fixture::new(move |ui| {
        let out = FontPicker::new("Font family", Bind::bound(&mut family), &catalog)
            .filters(Bind::bound(&mut filters))
            .host(&[HostFontGroup::new(None, &rows)])
            .current_host(shown.borrow().clone())
            .show(ui);
        if let Some(key) = out.host_picked {
            *shown.borrow_mut() = key;
        }
    });
    let host = craft_host::Host::builder(craft_host::AppInfo { id: "trial", name: "Trial", version: "0" }).jobs(jobs.clone()).build();
    fx.host(Arc::new(host));
    fx.in_room(Room::Window);
    fx.settle();
    fx.key(Key::Tab, Modifiers::NONE);
    fx.key(Key::ArrowDown, Modifiers::NONE);
    fx.key(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(current.borrow().as_str(), "a");
    jobs.run_all();
    fx.settle();
    assert_eq!(current.borrow().as_str(), "c", "both queued arrows advance through host rows");
}

/// Queued closed-field steps that pass a host row and stop on a family pick that family: the last stop decides, so
/// no host key is reported, and the family is the pick. `wheel` takes the second step with the wheel.
fn host_then_family_steps(wheel: bool) {
    use craft_font_manager::picker::{HostFontGroup, HostFontRow};
    use craft_ui::testing::pointer::{moved, wheel_lines};
    #[derive(Default)]
    struct Out {
        host: Vec<String>,
        picks: Vec<String>,
    }
    let jobs = Arc::new(craft_host::ManualJobs::new());
    let seen = Rc::new(RefCell::new(Out::default()));
    let shown = Rc::clone(&seen);
    let catalog = catalog_of(&[face("Book")]);
    let rows = [HostFontRow::face("a", "A", None), HostFontRow::face("b", "B", None)];
    let mut family = String::new();
    let mut filters = FontFilters::default();
    filters.properties.height = Some(Grade::Medium);
    let mut fx = Fixture::new(move |ui| {
        let out = FontPicker::new("Font family", Bind::bound(&mut family), &catalog)
            .filters(Bind::bound(&mut filters))
            .host(&[HostFontGroup::new(None, &rows)])
            .current_host("a".to_owned())
            .show(ui);
        let mut seen = shown.borrow_mut();
        seen.host.extend(out.host_picked);
        seen.picks.extend(out.picked);
    });
    let host = craft_host::Host::builder(craft_host::AppInfo { id: "trial", name: "Trial", version: "0" }).jobs(jobs.clone()).build();
    fx.host(Arc::new(host));
    fx.in_room(Room::Window);
    fx.settle();
    fx.key(Key::Tab, Modifiers::NONE);
    fx.key(Key::ArrowDown, Modifiers::NONE);
    if wheel {
        let field = fx.by_label("Font family").expect("the family field");
        fx.frame(vec![moved(field.center()), wheel_lines(-1.0)]);
    } else {
        fx.key(Key::ArrowDown, Modifiers::NONE);
    }
    assert!(seen.borrow().host.is_empty() && seen.borrow().picks.is_empty(), "the steps wait for the outline reads");
    jobs.run_all();
    fx.settle();
    if wheel {
        // The wheel's run picks its family when the gesture ends.
        fx.advance(craft_behavior::GESTURE_GAP * 2.0).idle(3);
    }
    let seen = seen.borrow();
    assert!(seen.host.is_empty(), "a later family stop reports no host key, got {:?}", seen.host);
    assert_eq!(seen.picks, ["Book"], "the family the steps stopped on is the pick");
}

/// How many host jobs an open list over Book and Poster runs for its height reads, with the Document set (Book only)
/// on when `set_on`.
fn outline_reads_with_set(set_on: bool) -> usize {
    use craft_font_manager::picker::FamilySet;
    let jobs = Arc::new(craft_host::ManualJobs::new());
    let catalog = catalog_of(&[face("Book"), Face { style: "Black", weight: 900, ..face("Poster") }]);
    let members = vec!["book".to_owned()];
    let mut family = "Book".to_owned();
    let mut filters = FontFilters::default();
    filters.properties.height = Some(Grade::Medium);
    if set_on {
        filters.sets = vec!["document".to_owned()];
    }
    let mut fx = Fixture::new(move |ui| {
        let sets = [FamilySet::new("document", "Document", &members)];
        FontPicker::new("Font family", Bind::bound(&mut family), &catalog).filters(Bind::bound(&mut filters)).sets(&sets).show(ui);
    });
    let host = craft_host::Host::builder(craft_host::AppInfo { id: "trial", name: "Trial", version: "0" }).jobs(jobs.clone()).build();
    fx.host(Arc::new(host));
    fx.in_room(Room::Window);
    fx.settle();
    fx.key(Key::Tab, Modifiers::NONE);
    fx.key(Key::ArrowDown, Modifiers::ALT);
    fx.settle();
    let mut ran = 0;
    for _ in 0..4 {
        ran += jobs.run_all();
        fx.settle();
    }
    assert!(listed(&fx, "Book"), "Book is in the set and matches the height");
    ran
}

/// An enabled family set narrows the list before the property filters, so families outside it queue no outline reads.
#[trial]
fn an_enabled_set_keeps_families_outside_it_from_queuing_outline_reads() {
    assert!(outline_reads_with_set(true) < outline_reads_with_set(false), "the set's other family is never read");
}

/// An outline read that finishes while the list is open keeps the keyboard lead where the arrows left it: the next
/// arrow moves on from that row.
#[trial]
fn finished_outline_reads_keep_the_open_lists_lead() {
    use craft_behavior::Preview;
    let jobs = Arc::new(craft_host::ManualJobs::new());
    let preview = Rc::new(RefCell::new(None::<String>));
    let shown = Rc::clone(&preview);
    let catalog = catalog_of(&[face("Book"), face("Middle"), face("Poster")]);
    let mut family = "Book".to_owned();
    let mut filters = FontFilters::default();
    filters.properties.height = Some(Grade::Medium);
    let mut fx = Fixture::new(move |ui| {
        let out = FontPicker::new("Font family", Bind::bound(&mut family), &catalog).filters(Bind::bound(&mut filters)).show(ui);
        match out.preview {
            Preview::Shows(family) => *shown.borrow_mut() = Some(family),
            Preview::Ended => *shown.borrow_mut() = None,
            _ => {}
        }
    });
    let host = craft_host::Host::builder(craft_host::AppInfo { id: "trial", name: "Trial", version: "0" }).jobs(jobs.clone()).build();
    fx.host(Arc::new(host));
    fx.in_room(Room::Window);
    fx.settle();
    fx.key(Key::Tab, Modifiers::NONE);
    fx.key(Key::ArrowDown, Modifiers::ALT);
    fx.settle();
    // Book is the field's text, so only Middle and Poster show whether their rows are read.
    while !(listed(&fx, "Middle") && listed(&fx, "Poster")) && jobs.run_next() {
        fx.settle();
    }
    assert!(fx.texts().iter().any(|text| text.text == "Font information is loading."), "a read is still pending");
    // The lead starts on the first row read (Poster); the arrow moves it up to Middle.
    fx.key(Key::ArrowUp, Modifiers::NONE);
    fx.settle();
    assert_eq!(preview.borrow().as_deref(), Some("Middle"), "the arrow leads Middle");
    jobs.run_all();
    fx.settle();
    jobs.run_all();
    fx.settle();
    assert!(!fx.texts().iter().any(|text| text.text == "Font information is loading."), "every read finished");
    fx.key(Key::ArrowUp, Modifiers::NONE);
    fx.settle();
    assert_eq!(preview.borrow().as_deref(), Some("Book"), "the next arrow moves on from Middle to the row read last");
}

#[trial]
fn closed_field_arrows_from_a_host_row_to_a_family_pick_the_family() {
    host_then_family_steps(false);
}

#[trial]
fn closed_field_wheel_from_a_host_row_to_a_family_picks_the_family() {
    host_then_family_steps(true);
}

/// Queued closed-field arrows that pass a family and stop on a host row report the host row and leave the family
/// binding as it was: the family stepped past is not written.
#[trial]
fn closed_field_arrows_past_a_family_to_a_host_row_leave_the_family() {
    use craft_font_manager::picker::{HostFontGroup, HostFontRow};
    let jobs = Arc::new(craft_host::ManualJobs::new());
    let hosts = Rc::new(RefCell::new(Vec::<String>::new()));
    let family = Rc::new(RefCell::new("Poster".to_owned()));
    let (seen_hosts, seen_family) = (Rc::clone(&hosts), Rc::clone(&family));
    let catalog = catalog_of(&[face("Book"), face("Poster")]);
    let rows = [HostFontRow::face("a", "A", None)];
    let mut filters = FontFilters::default();
    filters.properties.height = Some(Grade::Medium);
    let mut fx = Fixture::new(move |ui| {
        let out = FontPicker::new("Font family", Bind::bound(&mut *seen_family.borrow_mut()), &catalog)
            .filters(Bind::bound(&mut filters))
            .host(&[HostFontGroup::new(None, &rows)])
            .show(ui);
        seen_hosts.borrow_mut().extend(out.host_picked);
    });
    let host = craft_host::Host::builder(craft_host::AppInfo { id: "trial", name: "Trial", version: "0" }).jobs(jobs.clone()).build();
    fx.host(Arc::new(host));
    fx.in_room(Room::Window);
    fx.settle();
    fx.key(Key::Tab, Modifiers::NONE);
    fx.key(Key::ArrowUp, Modifiers::NONE);
    fx.key(Key::ArrowUp, Modifiers::NONE);
    assert!(hosts.borrow().is_empty(), "the arrows wait for the outline reads");
    jobs.run_all();
    fx.settle();
    assert_eq!(*hosts.borrow(), ["a"], "the host row the arrows stopped on is reported");
    assert_eq!(family.borrow().as_str(), "Poster", "Book, stepped past, is not written");
}

/// Picks Heavy in the open list's Weight dropdown.
fn pick_heavy(fx: &mut Fixture) {
    let weight = fx.by_label("Weight").expect("the weight dropdown");
    fx.click(&weight);
    fx.settle();
    let heavy = fx.by_label("Heavy").expect("the weight dropdown's Heavy choice");
    fx.click(&heavy);
    fx.settle();
}

/// The dropdowns show only when the host asks for them; under the toggle row, a choice narrows the list.
#[trial]
fn the_property_dropdowns_show_when_asked_and_a_choice_narrows_the_list() {
    let filters = Rc::new(RefCell::new(FontFilters::default()));
    let fx = properties_rig(Rc::clone(&filters), FilterLayout::Toolbar, false);
    assert!(fx.by_label("Weight").is_none(), "no property dropdowns unless asked");

    let mut fx = properties_rig(Rc::clone(&filters), FilterLayout::Toolbar, true);
    assert!(listed(&fx, "Book"), "every family lists before a choice");
    pick_heavy(&mut fx);
    assert_eq!(filters.borrow().properties.weight, Some(Grade::High), "the choice is in the filters");
    assert!(!listed(&fx, "Book"), "the regular family leaves the list");
}

/// In the chips layout the dropdowns open behind the Font Properties chip, so the closed chip adds no row, and turning
/// the chip off clears the choices it holds.
#[trial]
fn the_font_properties_chip_opens_the_dropdowns_and_turned_off_clears_them() {
    let filters = Rc::new(RefCell::new(FontFilters::default()));
    let fx = properties_rig(Rc::clone(&filters), FilterLayout::Chips, false);
    assert!(fx.by_label("Font Properties").is_none(), "no chip unless asked");

    let mut fx = properties_rig(Rc::clone(&filters), FilterLayout::Chips, true);
    assert!(fx.by_label("Weight").is_none(), "the dropdowns wait for the chip");
    let chip = fx.by_label("Font Properties").expect("the Font Properties chip");
    fx.click(&chip);
    fx.settle();
    pick_heavy(&mut fx);
    assert_eq!(filters.borrow().properties.weight, Some(Grade::High), "the choice is in the filters");
    let chip = fx.by_label("Font Properties").expect("the Font Properties chip");
    fx.click(&chip);
    fx.settle();
    assert!(!filters.borrow().properties.any(), "turning the chip off clears its choices");
    assert!(fx.by_label("Weight").is_none(), "and closes the dropdowns");
}
