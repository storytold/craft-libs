//! The Curves dialog's Input and Output with no point selected: they read out the curve under the pointer as text, and
//! turn back into the selected point's fields when a point is selected (photocraft#14435).

use craft_imaging::ui::adjust::{AdjustContext, CurvesMode, CurvesPanel, ToneChannels, ToneCurves, ToneSpace};
use craft_imaging::ui::curve::{Curve, CurveEditor};
use craft_theme::{Contrast, Mode};
use craft_ui::contract::{Bind, Role};
use craft_ui::testing::access::name;
use craft_ui::testing::pointer::{gone, moved, press, release, secondary};
use craft_ui::testing::{Fixture, style, trial};
use egui::{Pos2, Vec2, pos2, vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// The curve the reference application was measured with: over input 64 it reads 20.
const MEASURED: [[f32; 2]; 3] = [[0.0, 0.0], [128.0, 64.0], [255.0, 255.0]];

/// The natural cubic spline through `points` at every level 0..=255, as PhotoCraft's Curves draws them.
fn natural(points: &[[f32; 2]]) -> Vec<f32> {
    let n = points.len();
    let x: Vec<f64> = points.iter().map(|p| f64::from(p[0])).collect();
    let y: Vec<f64> = points.iter().map(|p| f64::from(p[1])).collect();
    // Second derivatives, zero at both ends, by the tridiagonal sweep.
    let mut m = vec![0.0; n];
    if n > 2 {
        let h: Vec<f64> = x.windows(2).map(|w| w[1] - w[0]).collect();
        let mut diag = vec![0.0; n];
        let mut rhs = vec![0.0; n];
        for i in 1..n - 1 {
            diag[i] = 2.0 * (h[i - 1] + h[i]);
            rhs[i] = 6.0 * ((y[i + 1] - y[i]) / h[i] - (y[i] - y[i - 1]) / h[i - 1]);
        }
        for i in 2..n - 1 {
            let k = h[i - 1] / diag[i - 1];
            diag[i] -= k * h[i - 1];
            rhs[i] -= k * rhs[i - 1];
        }
        for i in (1..n - 1).rev() {
            m[i] = (rhs[i] - h[i] * m[i + 1]) / diag[i];
        }
    }
    (0..=255)
        .map(|level| {
            let t = f64::from(level);
            let i = (0..n - 1).find(|&i| t <= x[i + 1]).unwrap_or(n - 2);
            let h = x[i + 1] - x[i];
            let (a, b) = (x[i + 1] - t, t - x[i]);
            let v = m[i] * a.powi(3) / (6.0 * h)
                + m[i + 1] * b.powi(3) / (6.0 * h)
                + (y[i] / h - m[i] * h / 6.0) * a
                + (y[i + 1] / h - m[i + 1] * h / 6.0) * b;
            v.clamp(0.0, 255.0) as f32
        })
        .collect()
}

fn room() -> Vec2 {
    let unit = style::tokens(Mode::Light, Contrast::Standard).size.plot.width;
    Vec2::new(unit * 5.0, unit * 7.0)
}

struct Rig {
    fixture: Fixture,
    /// The bare editor's pencil mode.
    pencil: Rc<Cell<bool>>,
    /// The bare editor's curve.
    curve: Option<Rc<RefCell<Curve>>>,
    selected: Rc<Cell<Option<usize>>>,
    origin: Pos2,
    scale: Vec2,
}

impl Rig {
    /// The Curves panel over the RGB curve through `points`, in `context`.
    fn panel(points: &[[f32; 2]], context: AdjustContext) -> Rig {
        let curve = Curve::smooth(points.to_vec());
        let channels = ToneChannels::of(ToneSpace::Rgb, false);
        let mut curves = vec![Curve::identity(255.0); channels.len()];
        curves[0] = curve.clone();
        let value = Rc::new(RefCell::new(ToneCurves::new(curves, CurvesMode::Points)));
        let selected = Rc::new(Cell::new(None));
        let (v, s) = (value.clone(), selected.clone());
        let fixture = Fixture::new(move |ui| {
            let mut tone = v.borrow().clone();
            let out = CurvesPanel::new("Curves", Bind::bound(&mut tone), channels.clone()).context(context).sampler(natural).show(ui);
            s.set(out.selected);
            *v.borrow_mut() = tone;
        });
        Rig::calibrated(fixture, selected, &curve)
    }

    /// A bare editor of `points`, as the Contour Editor and the raw developer show it, with the readout when
    /// `readout`.
    fn editor(points: &[[f32; 2]], readout: bool) -> Rig {
        let curve = Curve::smooth(points.to_vec());
        let shared = Rc::new(RefCell::new(curve.clone()));
        let selected = Rc::new(Cell::new(None));
        let pencil = Rc::new(Cell::new(false));
        let (c, s, p) = (shared.clone(), selected.clone(), pencil.clone());
        let fixture = Fixture::new(move |ui| {
            let mut value = c.borrow().clone();
            let mut editor = CurveEditor::new("Curve", Bind::bound(&mut value)).selection(s.get());
            if readout {
                editor = editor.hover_readout();
            }
            if p.get() {
                editor = editor.pencil();
            }
            let out = editor.show(ui);
            s.set(out.selected);
            *c.borrow_mut() = value;
        });
        let mut rig = Rig::calibrated(fixture, selected, &curve);
        rig.pencil = pencil;
        rig.curve = Some(shared);
        rig
    }

    /// Maps levels to screen from the end points' nodes.
    fn calibrated(mut fixture: Fixture, selected: Rc<Cell<Option<usize>>>, curve: &Curve) -> Rig {
        fixture.room(room());
        fixture.settle();
        let points = curve.points().map(<[_]>::to_vec).unwrap_or_default();
        let n = points.len();
        let a = fixture.by_label(&format!("Point 1 of {n}")).expect("the first point").center();
        let b = fixture.by_label(&format!("Point {n} of {n}")).expect("the last point").center();
        let (p, q) = (points[0], points[n - 1]);
        let scale = vec2((b.x - a.x) / (q[0] - p[0]), (b.y - a.y) / (q[1] - p[1]));
        let origin = pos2(a.x - p[0] * scale.x, a.y - p[1] * scale.y);
        Rig { fixture, pencil: Rc::new(Cell::new(false)), curve: None, selected, origin, scale }
    }

    /// Switches the bare editor to pencil mode with the drawn `map` as its curve.
    fn draw(&mut self, map: Vec<f32>) {
        if let Some(curve) = &self.curve {
            *curve.borrow_mut() = Curve::Map(map);
        }
        self.pencil.set(true);
        self.fixture.settle();
    }

    fn at(&self, q: [f32; 2]) -> Pos2 {
        pos2(self.origin.x + q[0] * self.scale.x, self.origin.y + q[1] * self.scale.y)
    }

    fn hover(&mut self, q: [f32; 2]) {
        self.fixture.frame(vec![moved(self.at(q))]);
        self.fixture.idle(1);
    }

    fn click(&mut self, q: [f32; 2]) {
        let pos = self.at(q);
        self.fixture.frame(vec![moved(pos)]);
        self.fixture.frame(vec![press(pos)]);
        self.fixture.frame(vec![release(pos)]);
        self.fixture.idle(1);
    }

    fn right_click(&mut self, q: [f32; 2]) {
        let pos = self.at(q);
        self.fixture.frame(vec![moved(pos)]);
        self.fixture.frame(vec![secondary(pos, true)]);
        self.fixture.frame(vec![secondary(pos, false)]);
        self.fixture.idle(1);
    }

    /// The read-only text named `label`: the readout's value, `None` while it shows as a field.
    fn reading(&self, label: &str) -> Option<String> {
        let tree = self.fixture.tree();
        tree.keys()
            .into_iter()
            .filter(|&key| name(tree, key) == Some(label) && tree.role_of(key) == Some(Role::LABEL))
            .filter_map(|key| tree.value_of(key))
            // The field's own label node carries its name as its value.
            .find(|&value| value != label)
            .map(str::to_owned)
    }

    /// The field named `label`: its value and whether it is disabled, `None` while it reads out as text.
    fn field(&self, label: &str) -> Option<(String, bool)> {
        let tree = self.fixture.tree();
        tree.keys()
            .into_iter()
            .find(|&key| name(tree, key) == Some(label) && tree.role_of(key) != Some(Role::LABEL) && tree.value_of(key).is_some())
            .and_then(|key| Some((tree.value_of(key)?.to_owned(), tree.is_disabled_key(key))))
    }
}

/// With no point selected, the pointer over input 64 of the measured curve reads Input 64 and Output 20 as plain text,
/// and the readout follows the pointer.
#[trial]
fn with_no_point_selected_the_fields_read_the_curve_under_the_pointer() {
    let mut rig = Rig::panel(&MEASURED, AdjustContext::dialog(true));
    rig.hover([64.0, 200.0]);
    assert_eq!(rig.selected.get(), None);
    assert_eq!(rig.reading("Input").as_deref(), Some("64"));
    assert_eq!(rig.reading("Output").as_deref(), Some("20"));
    assert_eq!(rig.field("Input"), None, "no editable Input field while the readout shows");
    assert_eq!(rig.field("Output"), None, "no editable Output field while the readout shows");

    rig.hover([200.0, 30.0]);
    let expected = natural(&MEASURED)[200].round();
    assert_eq!(rig.reading("Input").as_deref(), Some("200"));
    assert_eq!(rig.reading("Output"), Some(format!("{expected}")));
}

/// A selected point shows its values in editable fields; deselecting it returns to the readout.
#[trial]
fn selecting_a_point_shows_its_fields_and_deselecting_returns_to_the_readout() {
    let mut rig = Rig::panel(&MEASURED, AdjustContext::dialog(true));
    rig.click([128.0, 64.0]);
    assert_eq!(rig.selected.get(), Some(1));
    rig.hover([64.0, 200.0]);
    assert_eq!(rig.field("Input"), Some(("128".to_owned(), false)), "the selected point's Input, editable");
    assert_eq!(rig.field("Output"), Some(("64".to_owned(), false)), "the selected point's Output, editable");
    assert_eq!(rig.reading("Input"), None);

    rig.right_click([64.0, 200.0]);
    assert_eq!(rig.selected.get(), None);
    assert_eq!(rig.reading("Input").as_deref(), Some("64"));
    assert_eq!(rig.reading("Output").as_deref(), Some("20"));
}

/// In pencil mode the Input and Output read out the drawn curve under the pointer, and with the pointer off the graph
/// there is no Input and Output row, as without the readout.
#[trial]
fn in_pencil_mode_the_fields_read_the_drawn_curve_under_the_pointer() {
    let mut rig = Rig::editor(&MEASURED, true);
    rig.draw((0..=255).map(|level| 255.0 - level as f32).collect());
    rig.hover([100.0, 30.0]);
    assert_eq!(rig.reading("Input").as_deref(), Some("100"));
    assert_eq!(rig.reading("Output").as_deref(), Some("155"));
    assert_eq!(rig.field("Input"), None);

    rig.fixture.frame(vec![gone()]);
    rig.fixture.idle(1);
    for label in ["Input", "Output"] {
        assert_eq!((rig.reading(label), rig.field(label)), (None, None), "no {label} once the pointer has left");
    }
}

/// With the pointer off the graph the fields keep today's disabled zeros (not measured).
#[trial]
fn off_the_graph_the_fields_keep_their_disabled_zeros() {
    let mut rig = Rig::panel(&MEASURED, AdjustContext::dialog(true));
    rig.hover([64.0, 200.0]);
    assert!(rig.reading("Input").is_some());
    rig.fixture.frame(vec![gone()]);
    rig.fixture.idle(1);
    assert_eq!(rig.field("Input"), Some(("0".to_owned(), true)));
    assert_eq!(rig.field("Output"), Some(("0".to_owned(), true)));
}

/// The readout is the Curves dialog's: the Properties panel and the bare editor keep their disabled fields.
#[trial]
fn the_properties_panel_and_the_bare_editor_keep_their_fields() {
    let mut rig = Rig::panel(&MEASURED, AdjustContext::properties(true));
    rig.hover([64.0, 200.0]);
    assert_eq!(rig.reading("Input"), None);
    assert_eq!(rig.field("Input"), Some(("0".to_owned(), true)));

    let mut rig = Rig::editor(&MEASURED, false);
    rig.hover([64.0, 200.0]);
    assert_eq!(rig.reading("Input"), None);
    assert_eq!(rig.field("Input"), Some(("0".to_owned(), true)));
}
