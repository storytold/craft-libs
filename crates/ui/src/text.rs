//! Position already shaped text within a fixed-width reading area.

use std::sync::Arc;

use egui::emath::GuiRounding as _;
use egui::{Align, Color32, Galley, Painter, Pos2, Rect, Vec2, epaint::TextShape, epaint::text::PlacedRow, pos2, vec2};

/// Already-shaped rows and their explicit placement relative to a reading area.
///
/// The normalized galley alone does not include its reading-area offset. Use [`Self::paint`]
/// or [`Self::text_shape`] to retain it; there is deliberately no implicit galley conversion.
#[derive(Clone, Debug)]
#[must_use]
pub enum AlignedGalley {
    /// A complete left-anchored logical bound, with the reading area kept separately.
    Aligned { galley: Arc<Galley>, paint_offset: Vec2, reading_rect: Rect },
    /// An invalid width leaves the original galley untouched, including its alignment.
    InvalidWidth { original: Arc<Galley> },
}

impl AlignedGalley {
    /// The requested reading area, in coordinates relative to the supplied reading origin.
    /// An invalid width has no reading rectangle.
    pub fn reading_rect(&self) -> Option<Rect> {
        match self {
            Self::Aligned { reading_rect, .. } => Some(*reading_rect),
            Self::InvalidWidth { .. } => None,
        }
    }

    /// The original text height, independent of horizontal overflow.
    pub fn height(&self) -> f32 {
        match self {
            Self::Aligned { reading_rect, .. } => reading_rect.height(),
            Self::InvalidWidth { original } => original.size().y,
        }
    }

    /// A shape placed at the reading area's origin, retaining the explicit galley offset.
    /// Like [`TextShape::new`], this leaves final pixel rounding to the renderer.
    pub fn text_shape(&self, reading_origin: Pos2, color: Color32) -> TextShape {
        match self {
            Self::Aligned { galley, paint_offset, .. } => TextShape::new(reading_origin + *paint_offset, galley.clone(), color),
            Self::InvalidWidth { original } => TextShape::new(reading_origin, original.clone(), color),
        }
    }

    /// Paint at the reading area's origin using the painter's existing clipping rectangle.
    ///
    /// When text rounding is enabled, snap the reading origin before adding the pixel-aligned
    /// offset. Otherwise translating an origin near half a pixel could change which pixel the
    /// renderer selects. An invalid width uses the original painting path unchanged.
    pub fn paint(self, painter: &Painter, reading_origin: Pos2, color: Color32) {
        match self {
            Self::Aligned { galley, paint_offset, .. } => {
                let origin = if painter.ctx().tessellation_options(|options| options.round_text_to_pixels) {
                    reading_origin.round_to_pixels(galley.pixels_per_point)
                } else {
                    reading_origin
                };
                painter.galley(origin + paint_offset, galley, color);
            }
            Self::InvalidWidth { original } => painter.galley(reading_origin, original, color),
        }
    }
}

/// Align each row inside `width`, without changing its glyphs, wrapping, or logical text.
///
/// This is useful for read-only comments, captions and messages whose text must reach the
/// container's edges. `LayoutJob` alignment ordinarily uses the widest text row instead.
/// The caller supplies the row alignment, for example from paragraph direction in its shaper.
/// This helper uses only stock egui APIs and does not implement bidirectional shaping.
///
/// The result keeps the requested reading rectangle separately from the complete logical bound.
/// Oversized right-aligned rows overflow to the left, and centered rows overflow on both sides.
/// The galley is normalized to a left-edge anchor (`job.halign = Align::Min`, `rect.left() = 0`);
/// its explicit paint offset preserves every row's position relative to the reading area.
/// `mesh_bounds` describes the complete normalized ink. Apply clipping separately when required.
/// Positions snap down to physical pixels so right-aligned rows do not cross the right edge
/// through nearest-pixel rounding.
///
/// Use the result's paint method, supplying the reading area's origin. Per-row alignment is not
/// represented by its single `LayoutJob` alignment, so editable text should retain its original
/// galley for hit testing.
/// Zero width is valid and may place ink entirely outside the reading rectangle.
/// Invalid widths explicitly return the untouched original, without calling `align`.
/// Row positions are copy-on-write; the original shared glyph meshes remain shared, and a cached
/// galley is never modified.
pub fn align_galley_rows(mut galley: Arc<Galley>, width: f32, mut align: impl FnMut(&PlacedRow) -> Align) -> AlignedGalley {
    if !width.is_finite() || width < 0.0 {
        return AlignedGalley::InvalidWidth { original: galley };
    }
    let positioned = Arc::make_mut(&mut galley);
    if positioned.job.halign != Align::Min {
        Arc::make_mut(&mut positioned.job).halign = Align::Min;
    }
    let reading_rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(width, positioned.rect.height()));
    let mut bounds = reading_rect;
    for row in &mut positioned.rows {
        let spare = f64::from(width) - f64::from(row.size.x);
        let ppp = f64::from(positioned.pixels_per_point);
        row.pos.x = ((f64::from(align(row).to_factor()) * spare * ppp).floor() / ppp) as f32;
        bounds |= row.rect();
    }
    let paint_offset = vec2(bounds.left(), 0.0);
    let mut rect = reading_rect.translate(-paint_offset);
    let mut mesh_bounds = Rect::NOTHING;
    for row in &mut positioned.rows {
        row.pos.x -= paint_offset.x;
        rect |= row.rect();
        if !row.visuals.mesh.vertices.is_empty() {
            let ink = row.visuals.mesh_bounds;
            let offset = row.pos.to_vec2();
            // Keep the public translated rectangle as well as exact translated endpoints:
            // min + offset + (max - min) can round differently from max + offset.
            mesh_bounds |= ink.translate(offset);
            mesh_bounds |= Rect::from_min_max(ink.min + offset, ink.max + offset);
        }
    }
    positioned.rect = rect;
    positioned.mesh_bounds = mesh_bounds;
    AlignedGalley::Aligned { galley, paint_offset, reading_rect }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three-row fixture (`Short` / `A longer line` / `End`) laid out at 1.0 ppp and left aligned.
    fn layout() -> Arc<Galley> {
        layout_text("Short\nA longer line\nEnd", 1.0, Align::Min)
    }

    /// A fresh context at `ppp` with one settling pass, then a simple job of `text` with the requested
    /// halign and infinite width.
    fn layout_with_context(text: &str, ppp: f32, halign: Align) -> (egui::Context, Arc<Galley>) {
        let ctx = egui::Context::default();
        ctx.set_pixels_per_point(ppp);
        ctx.run_ui(egui::RawInput::default(), |_| {}).drop_without_applying_deltas();
        let mut job = egui::text::LayoutJob::simple(text.into(), egui::FontId::proportional(14.0), Color32::BLACK, f32::INFINITY);
        job.halign = halign;
        let galley = ctx.fonts_mut(|fonts| fonts.layout_job(job));
        (ctx, galley)
    }

    /// Convenience wrapper returning only the galley from [`layout_with_context`].
    fn layout_text(text: &str, ppp: f32, halign: Align) -> Arc<Galley> {
        layout_with_context(text, ppp, halign).1
    }

    /// Destructure an aligned result into galley, paint offset and reading rect; a valid fixture must
    /// not become `InvalidWidth`.
    fn parts(aligned: &AlignedGalley) -> (&Arc<Galley>, Vec2, Rect) {
        match aligned {
            AlignedGalley::Aligned { galley, paint_offset, reading_rect } => (galley, *paint_offset, *reading_rect),
            AlignedGalley::InvalidWidth { .. } => panic!("a valid fixture width must produce aligned rows"),
        }
    }

    fn fractional_ink_fixture() -> (Arc<Galley>, f32, Rect, Galley) {
        let shift = f32::from_bits(0x3f2a_aaab);
        let mut original = layout_text(" \nI", 1.5, Align::Min);
        let fixture = Arc::make_mut(&mut original);
        assert_eq!(fixture.rows.len(), 2);
        fixture.rect = Rect::from_min_size(Pos2::ZERO, vec2(1.0, 64.0));
        fixture.rows[0].pos = Pos2::ZERO;
        let empty = Arc::make_mut(&mut fixture.rows[0].row);
        empty.size.x = 0.5; // Max at width 0 snaps to -one pixel, not -two pixels.
        empty.visuals = Default::default();
        fixture.rows[1].pos = pos2(0.0, shift);
        let ink = Arc::make_mut(&mut fixture.rows[1].row);
        ink.size.x = 1.0;
        let mut mesh = egui::epaint::Mesh::default();
        mesh.add_rect_with_uv(
            Rect::from_min_max(pos2(-2.0, -2.0), pos2(f32::from_bits(0x4055_5555), shift)),
            Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)),
            Color32::BLACK,
        );
        ink.visuals = egui::epaint::text::RowVisuals {
            mesh_bounds: mesh.calc_bounds(),
            glyph_index_start: 0,
            glyph_vertex_range: 0..mesh.vertices.len(),
            mesh,
        };
        let local = ink.visuals.mesh_bounds;
        assert!(ink.visuals.mesh.is_valid());
        assert_eq!(ink.visuals.mesh.vertices.len(), 4);
        assert_eq!(ink.visuals.mesh.indices.len(), 6);
        for vertex in &ink.visuals.mesh.vertices {
            assert!(local.contains(vertex.pos));
        }
        fixture.mesh_bounds = Rect::from_min_max(local.min + vec2(0.0, shift), local.max + vec2(0.0, shift));
        fixture.num_vertices = 4;
        fixture.num_indices = 6;
        let snapshot = (*original).clone();
        assert!(snapshot.rows[0].visuals.mesh.vertices.is_empty());
        assert!(snapshot.rows[0].visuals.mesh.indices.is_empty());
        assert_eq!(snapshot.rows[0].visuals.mesh_bounds, Rect::NOTHING);
        assert_eq!(snapshot.rows.iter().filter(|row| !row.visuals.mesh.vertices.is_empty()).count(), 1);
        (original, shift, local, snapshot)
    }

    fn align_fixture(galley: Arc<Galley>) -> AlignedGalley {
        let mut index = 0;
        let aligned = align_galley_rows(galley, 0.0, |_| {
            index += 1;
            if index == 1 { Align::Max } else { Align::Min }
        });
        assert_eq!(index, 2);
        aligned
    }

    fn assert_shared_rows_and_bounds(aligned: &Galley, snapshot: &Galley, offset: Vec2, shift: f32) {
        assert_eq!(aligned.num_vertices, snapshot.num_vertices);
        assert_eq!(aligned.num_indices, snapshot.num_indices);
        for (index, (row, old)) in aligned.rows.iter().zip(&snapshot.rows).enumerate() {
            assert!(Arc::ptr_eq(&row.row, &old.row), "cached glyphs and meshes stay shared");
            assert_eq!(row.pos.y, old.pos.y);
            let reading_position = if index == 0 { pos2(-shift, 0.0) } else { old.pos };
            assert_eq!(row.pos + offset, reading_position);
            assert!(aligned.rect.contains_rect(row.rect()));
        }
        assert_eq!(aligned.rows[0].visuals.mesh_bounds, Rect::NOTHING);
    }

    fn assert_translated_ink_is_contained(aligned: &Galley, local: Rect) {
        let row = &aligned.rows[1];
        let legacy = local.translate(row.pos.to_vec2());
        let endpoints = Rect::from_min_max(local.min + row.pos.to_vec2(), local.max + row.pos.to_vec2());
        assert_eq!(legacy.max.x.to_bits(), 0x407f_ffff, "the legacy X maximum rounds inward");
        assert_eq!(endpoints.max.x.to_bits(), 0x4080_0000);
        assert_eq!(legacy.max.y.to_bits(), 0x3faa_aaac, "the legacy Y maximum rounds outward");
        assert_eq!(endpoints.max.y.to_bits(), 0x3faa_aaab);
        assert!(endpoints.max.x > legacy.max.x && endpoints.max.y < legacy.max.y);
        assert!(aligned.mesh_bounds.is_finite());
        assert!(aligned.mesh_bounds.contains_rect(legacy), "retain the complete public translated row rectangle");
        for (index, vertex) in row.visuals.mesh.vertices.iter().enumerate() {
            let position = row.pos + vertex.pos.to_vec2();
            assert!(
                aligned.mesh_bounds.contains(position),
                "actual translated vertex {index} must be contained: position={:?} bits={:x?}, bounds_max={:?} bits={:x?}",
                [position.x, position.y],
                [position.x.to_bits(), position.y.to_bits()],
                [aligned.mesh_bounds.max.x, aligned.mesh_bounds.max.y],
                [aligned.mesh_bounds.max.x.to_bits(), aligned.mesh_bounds.max.y.to_bits()]
            );
        }
        assert_eq!(aligned.mesh_bounds, legacy.union(endpoints));
    }

    fn assert_empty_ink_stays_empty(original: &Galley) {
        let mut all_empty = original.clone();
        for row in &mut all_empty.rows {
            Arc::make_mut(&mut row.row).visuals = Default::default();
        }
        all_empty.mesh_bounds = Rect::NOTHING;
        all_empty.num_vertices = 0;
        all_empty.num_indices = 0;
        let all_empty = align_fixture(Arc::new(all_empty));
        assert_eq!(parts(&all_empty).0.mesh_bounds, Rect::NOTHING);
    }

    /// Compare two rects componentwise within `epsilon`.
    fn close_rect(actual: Rect, expected: Rect, epsilon: f32) {
        assert!((actual.min - expected.min).length() <= epsilon, "{actual:?} != {expected:?}");
        assert!((actual.max - expected.max).length() <= epsilon, "{actual:?} != {expected:?}");
    }

    // The pre-normalization row placement, independent of the new wrapper/offset code. This is
    // the old physical-pixel floor formula, applied to the original shared glyph rows.
    /// Reconstruct the previous physical-pixel floor placement as an independent oracle. The
    /// reference starts from a rect of the requested width, recomputes each row's x from the alignment
    /// factor, and unions each row rect and translated ink bound into its `rect` and `mesh_bounds`;
    /// overwide rows therefore extend the rect past the requested width.
    fn reading_space_reference(original: &Galley, width: f32, alignments: &[Align]) -> Arc<Galley> {
        let mut reference = original.clone();
        reference.rect = Rect::from_min_size(Pos2::ZERO, vec2(width, original.rect.height()));
        reference.mesh_bounds = Rect::NOTHING;
        assert_eq!(reference.rows.len(), alignments.len());
        for (row, alignment) in reference.rows.iter_mut().zip(alignments) {
            let pixels = (f64::from(width) - f64::from(row.size.x)) * f64::from(alignment.to_factor()) * f64::from(original.pixels_per_point);
            row.pos.x = (pixels.floor() / f64::from(original.pixels_per_point)) as f32;
            reference.rect |= row.rect();
            reference.mesh_bounds |= row.visuals.mesh_bounds.translate(row.pos.to_vec2());
        }
        Arc::new(reference)
    }

    #[test]
    /// Per-row alignment at width 400 keeps row Y positions, the shared job and row arcs, and vertex
    /// and index counts; the returned logical width is 400 and every translated ink bound is contained.
    fn independent_row_alignment_preserves_shaping_and_cached_geometry() {
        let original = layout();
        let positions: Vec<_> = original.rows.iter().map(|row| row.pos).collect();
        let mut i = 0;
        let result = align_galley_rows(original.clone(), 400.0, |_| {
            let align = match i {
                0 => Align::Max,
                1 => Align::Min,
                _ => Align::Center,
            };
            i += 1;
            align
        });
        let (aligned, offset, reading) = parts(&result);
        assert_eq!(i, 3);
        assert_eq!(offset, Vec2::ZERO);
        assert_eq!(result.reading_rect(), Some(reading));
        assert_eq!(original.rows.iter().map(|row| row.pos).collect::<Vec<_>>(), positions);
        assert!(Arc::ptr_eq(&original.job, &aligned.job));
        assert_eq!(aligned.num_vertices, original.num_vertices);
        assert_eq!(aligned.num_indices, original.num_indices);
        for (index, (old, new)) in original.rows.iter().zip(&aligned.rows).enumerate() {
            assert!(Arc::ptr_eq(&old.row, &new.row), "glyphs and meshes must remain shared");
            assert_eq!(old.pos.y, new.pos.y);
            let factor = match index {
                0 => 1.0,
                1 => 0.0,
                _ => 0.5,
            };
            assert!((new.pos.x - (400.0 - new.size.x) * factor).abs() < 1.0);
            let bounds = new.visuals.mesh_bounds.translate(new.pos.to_vec2());
            assert!(aligned.mesh_bounds.contains_rect(bounds));
        }
        assert_eq!(aligned.size().x, 400.0);
        assert_eq!(aligned.size().y, original.size().y);
        assert_eq!(result.height(), original.size().y);
    }

    #[test]
    /// NaN, infinite and negative widths return `InvalidWidth` without calling the alignment callback
    /// or changing the galley and halign; there is no reading rect, and painting still uses the original
    /// galley at the given origin.
    fn invalid_widths_preserve_the_input() {
        for halign in [Align::Min, Align::Center, Align::Max] {
            let original = layout_text("Short", 1.0, halign);
            for width in [f32::NAN, f32::INFINITY, -1.0] {
                let result = align_galley_rows(original.clone(), width, |_| panic!("an invalid width must not call the alignment callback"));
                let AlignedGalley::InvalidWidth { original: unchanged } = &result else { panic!("invalid width must be explicit") };
                assert!(Arc::ptr_eq(&original, unchanged));
                assert_eq!(unchanged.job.halign, halign);
                assert_eq!(result.reading_rect(), None);
                assert_eq!(result.height(), original.size().y);
                let shape = result.text_shape(pos2(17.25, 31.5), Color32::BLACK);
                assert_eq!(shape.pos, pos2(17.25, 31.5));
                assert!(Arc::ptr_eq(&shape.galley, &original));
            }
        }
    }

    #[test]
    /// At 2.0 ppp, an unbreakably wide row at width 23.3 keeps the requested reading edge through the
    /// paint offset for Min/Center/Max: physical x is pixel-aligned and within half a pixel of the
    /// expected alignment, and all logical and ink bounds contain the row.
    fn overwide_rows_keep_the_requested_reading_edge() {
        let original = layout_text("unbreakable-long-token", 2.0, Align::Min);
        let width = 23.3;
        assert!(original.rows[0].size.x > width);
        for alignment in [Align::Min, Align::Center, Align::Max] {
            let result = align_galley_rows(original.clone(), width, |_| alignment);
            let (aligned, offset, reading) = parts(&result);
            let row = &aligned.rows[0];
            let x = offset.x + row.pos.x;
            let expected = (width - row.size.x) * alignment.to_factor();
            assert!(x <= expected + 0.0001);
            assert!(expected - x < 0.5);
            assert_eq!(x * 2.0, (x * 2.0).floor());
            if alignment == Align::Max {
                assert!(x < 0.0);
                assert!(x + row.size.x <= width + 0.0001);
            }
            assert_eq!(aligned.rect.left(), 0.0);
            assert_eq!(aligned.job.halign, Align::Min);
            assert_eq!(reading.width(), width);
            assert!(aligned.rect.contains_rect(row.rect()));
            assert!(aligned.rect.contains_rect(reading.translate(-offset)));
            assert!(aligned.mesh_bounds.contains_rect(row.visuals.mesh_bounds.translate(row.pos.to_vec2())));
        }
    }

    #[test]
    /// A right-aligned overwide row extends left of zero while its right edge stays at the requested
    /// reading edge, and the public logical rect contains the actual row.
    fn overwide_right_aligned_rows_have_complete_logical_bounds() {
        let original = layout_text("unbreakable-long-token", 2.0, Align::Min);
        let width = 23.3;
        assert!(original.rows[0].size.x > width, "the fixture must overflow the reading area");
        let result = align_galley_rows(original, width, |_| Align::Max);
        let (aligned, offset, _) = parts(&result);
        let row = &aligned.rows[0];
        let reading_row = row.rect().translate(offset);
        assert!(reading_row.left() < 0.0, "the right-aligned overwide row must extend to the left");
        assert!(reading_row.right() <= width, "the requested right reading edge stays fixed");
        assert!(aligned.rect.contains_rect(row.rect()), "the public logical bound {:?} must contain the actual row {:?}", aligned.rect, row.rect());
    }

    #[test]
    /// Fractional width 163.3 with any original halign normalizes the result to a left anchor: halign
    /// becomes Min, offset is zero, the logical rect is 0..163.3, and rows keep pixel-snapped absolute
    /// positions.
    fn fractional_width_and_nonleft_input_normalize_the_anchor() {
        for original_align in [Align::Min, Align::Center, Align::Max] {
            let original = layout_text("Short", 2.0, original_align);
            let result = align_galley_rows(original.clone(), 163.3, |_| Align::Max);
            let (aligned, offset, _) = parts(&result);
            assert_eq!(original.job.halign, original_align);
            assert_eq!(aligned.job.halign, Align::Min);
            assert_eq!(offset, Vec2::ZERO);
            assert_eq!(aligned.rect.left(), 0.0);
            assert_eq!(aligned.rect.right(), 163.3);
            for row in &aligned.rows {
                assert_eq!(row.pos.x * 2.0, (row.pos.x * 2.0).floor());
                assert!(row.rect().right() <= 163.3);
                assert!(163.3 - row.rect().right() < 0.5);
            }
        }
    }

    #[test]
    /// Empty, whitespace-only and trailing-newline text produce a valid container rect of the requested
    /// width and original height; a cleared row list gives zero height and `Rect::NOTHING` ink.
    fn empty_and_whitespace_rows_have_valid_logical_bounds() {
        for text in ["", "   ", "Short\n"] {
            let original = layout_text(text, 1.0, Align::Min);
            let height = original.rect.height();
            let result = align_galley_rows(original, 80.25, |_| Align::Max);
            let (aligned, offset, reading) = parts(&result);
            assert_eq!(offset, Vec2::ZERO);
            assert_eq!(aligned.rect, Rect::from_min_size(pos2(0.0, 0.0), vec2(80.25, height)));
            assert_eq!(reading, aligned.rect);
            assert_eq!(result.height(), height);
        }
        let original = layout();
        let mut empty = (*original).clone();
        empty.rows.clear();
        empty.rect = Rect::ZERO;
        empty.mesh_bounds = Rect::NOTHING;
        let result = align_galley_rows(Arc::new(empty), 80.25, |_| Align::Max);
        let (aligned, offset, _) = parts(&result);
        assert_eq!(offset, Vec2::ZERO);
        assert_eq!(aligned.rect, Rect::from_min_size(pos2(0.0, 0.0), vec2(80.25, 0.0)));
        assert_eq!(aligned.mesh_bounds, Rect::NOTHING);
    }

    #[test]
    /// At fractional scales, zero and fractional widths, aligning directly and re-aligning after a
    /// different first width produce identical reading rects, offsets, logical/ink bounds and row
    /// positions; row x positions are pixel-snapped in physical space.
    fn fractional_scale_zero_width_and_repeated_alignment_keep_absolute_positions() {
        for ppp in [1.25, 1.5] {
            let original = layout_text("Short", ppp, Align::Center);
            for width in [0.0, 163.3] {
                for alignment in [Align::Min, Align::Center, Align::Max] {
                    let direct = align_galley_rows(original.clone(), width, |_| alignment);
                    for first_width in [3.5, 230.5] {
                        let first = align_galley_rows(original.clone(), first_width, |_| Align::Max);
                        let repeated = align_galley_rows(parts(&first).0.clone(), width, |_| alignment);
                        let (a, a_offset, a_reading) = parts(&direct);
                        let (b, b_offset, b_reading) = parts(&repeated);
                        assert_eq!(a_reading.width(), width);
                        assert_eq!(a_reading, b_reading);
                        assert_eq!(a_offset, b_offset);
                        assert_eq!(a.rect, b.rect);
                        assert_eq!(a.mesh_bounds, b.mesh_bounds);
                        for (a, b) in a.rows.iter().zip(&b.rows) {
                            assert_eq!(a.pos, b.pos);
                            let pixel = (a.pos.x + a_offset.x) * ppp;
                            assert!((pixel - pixel.round()).abs() < 1e-4);
                        }
                    }
                }
            }
        }
    }

    #[test]
    /// A synthetic cached quad with binary32 rounding differences keeps its glyph arcs shared. The
    /// normalized offset is exactly the whitespace shift; `mesh_bounds` contains every translated vertex
    /// and the complete legacy rect, equals the union of both rounding directions, survives repetition,
    /// and remains `Rect::NOTHING` when all ink is empty.
    fn fractional_ink_bounds_contain_translated_rectangles_and_vertices() {
        let (original, shift, local, snapshot) = fractional_ink_fixture();
        let result = align_fixture(original.clone());
        let (aligned, offset, reading) = parts(&result);
        assert_eq!(offset, vec2(-shift, 0.0));
        assert_eq!(aligned.rows[1].pos, pos2(shift, shift));
        assert_eq!(aligned.rect.left_top(), Pos2::ZERO);
        assert_eq!(aligned.job.halign, Align::Min);
        assert_eq!(result.height(), 64.0);
        assert_eq!(reading, Rect::from_min_size(Pos2::ZERO, vec2(0.0, 64.0)));
        assert!(aligned.rect.contains_rect(reading.translate(-offset)));
        assert_eq!(*original, snapshot, "alignment must leave the complete source unchanged");
        assert_shared_rows_and_bounds(aligned, &snapshot, offset, shift);
        let repeated = align_fixture(aligned.clone());
        let (again, again_offset, again_reading) = parts(&repeated);
        assert_eq!(again_offset, offset);
        assert_eq!(again_reading, reading);
        assert_eq!(**again, **aligned, "repeated alignment is exact, including cached ink bounds");

        assert_empty_ink_stays_empty(&original);
        assert_translated_ink_is_contained(aligned, local);
    }

    #[test]
    /// The logical container width follows the reading width while ink bounds stay the glyph extent;
    /// whitespace has a container but no ink bounds.
    fn logical_container_and_ink_bounds_remain_distinct() {
        let result = align_galley_rows(layout_text("Short", 1.0, Align::Min), 400.0, |_| Align::Min);
        let (aligned, _, _) = parts(&result);
        assert_eq!(aligned.rect.width(), 400.0);
        assert!(aligned.mesh_bounds.right() < aligned.rect.right());
        let result = align_galley_rows(layout_text("   ", 1.0, Align::Min), 400.0, |_| Align::Max);
        let (whitespace, _, _) = parts(&result);
        assert_eq!(whitespace.rect.width(), 400.0);
        assert_eq!(whitespace.mesh_bounds, Rect::NOTHING);
    }

    #[test]
    /// A uniquely owned galley is reused at the same allocation instead of being cloned.
    fn uniquely_owned_galley_reuses_its_allocation() {
        let original = Arc::new((*layout()).clone());
        assert_eq!(Arc::strong_count(&original), 1);
        let before = Arc::as_ptr(&original);
        let result = align_galley_rows(original, 400.0, |_| Align::Max);
        assert_eq!(before, Arc::as_ptr(parts(&result).0));
    }

    const MIXED: &str = "First extraordinarily long reading row\nSecond independently aligned reading row\nThird right-aligned overwide reading row";
    const ALIGNMENTS: [Align; 3] = [Align::Min, Align::Center, Align::Max];

    /// Assert the mixed fixture's three rows each overflow `width`, then apply Min/Center/Max in that
    /// order through `align_galley_rows`.
    fn mixed(original: Arc<Galley>, width: f32) -> AlignedGalley {
        assert_eq!(original.rows.len(), ALIGNMENTS.len());
        assert!(original.rows.iter().all(|row| row.size.x > width), "each fixture row must overflow");
        let mut index = 0;
        let result = align_galley_rows(original, width, |_| {
            let alignment = ALIGNMENTS[index];
            index += 1;
            alignment
        });
        assert_eq!(index, ALIGNMENTS.len());
        result
    }

    #[test]
    /// Across fractional scales and widths 0/23.3, mixed overflow matches the independent oracle after
    /// translating by the offset, shares row arcs, preserves absolute Y positions, and keeps the
    /// reading rect at the requested width.
    fn mixed_overflow_normalizes_bounds_without_changing_absolute_rows_or_ink() {
        for ppp in [1.25, 1.5, 2.0] {
            let original = layout_text(MIXED, ppp, Align::Center);
            for width in [0.0, 23.3] {
                let expected = reading_space_reference(&original, width, &ALIGNMENTS);
                assert!(expected.rect.left() < 0.0 && expected.rect.right() > width, "fixture must overflow both reading edges");
                let result = mixed(original.clone(), width);
                let (aligned, offset, reading) = parts(&result);
                assert_eq!(aligned.job.halign, Align::Min);
                assert_eq!(aligned.rect.left_top(), Pos2::ZERO);
                assert_eq!(reading, Rect::from_min_size(Pos2::ZERO, vec2(width, original.rect.height())));
                assert_eq!(offset, vec2(expected.rect.left(), 0.0));
                close_rect(aligned.rect.translate(offset), expected.rect, 1e-4 / ppp);
                close_rect(aligned.mesh_bounds.translate(offset), expected.mesh_bounds, 1e-4 / ppp);
                assert_eq!(aligned.num_vertices, original.num_vertices);
                assert_eq!(aligned.num_indices, original.num_indices);
                for ((row, expected), original) in aligned.rows.iter().zip(&expected.rows).zip(&original.rows) {
                    assert!(Arc::ptr_eq(&row.row, &original.row));
                    assert_eq!(row.pos.y, original.pos.y);
                    assert!(aligned.rect.contains_rect(row.rect()));
                    assert!(aligned.mesh_bounds.contains_rect(row.visuals.mesh_bounds.translate(row.pos.to_vec2())));
                    assert!(((row.pos + offset) - expected.pos).length() * ppp < 1e-4);
                    let pixel = (row.pos.x + offset.x) * ppp;
                    assert!((pixel - pixel.round()).abs() < 1e-4);
                }
            }
        }
    }

    #[test]
    /// `text_shape` anchors rotation at the complete logical bound rather than the reading rect; row
    /// corners and the visual ink bounds match an independently rotated oracle for several anchors.
    fn public_text_shape_uses_the_complete_logical_bound_for_rotation_anchors() {
        let original = layout_text(MIXED, 1.5, Align::Min);
        let expected = reading_space_reference(&original, 23.3, &ALIGNMENTS);
        let result = mixed(original, 23.3);
        let (galley, _, _) = parts(&result);
        let origin = pos2(51.25, 37.5);
        let angle = 0.73;
        let rotation = egui::emath::Rot2::from_angle(angle);
        for anchor in [egui::Align2::LEFT_TOP, egui::Align2::CENTER_CENTER, egui::Align2::RIGHT_BOTTOM] {
            let pivot = origin + anchor.pos_in_rect(&expected.rect).to_vec2();
            let shape = result.text_shape(origin, Color32::BLACK).with_angle_and_anchor(angle, anchor);
            for (row, expected) in galley.rows.iter().zip(&expected.rows) {
                for (corner, expected) in
                    [(row.rect().left_top(), expected.rect().left_top()), (row.rect().right_bottom(), expected.rect().right_bottom())]
                {
                    let actual = shape.pos + rotation * corner.to_vec2();
                    let expected = pivot + rotation * (origin + expected.to_vec2() - pivot);
                    assert!((actual - expected).length() < 1e-4, "rotation must retain the independently computed logical anchor");
                }
            }
            let corners = [
                expected.mesh_bounds.left_top(),
                expected.mesh_bounds.right_top(),
                expected.mesh_bounds.right_bottom(),
                expected.mesh_bounds.left_bottom(),
            ];
            let expected_ink = Rect::from_points(&corners.map(|p| pivot + rotation * (origin + p.to_vec2() - pivot)));
            close_rect(shape.visual_bounding_rect(), expected_ink, 1e-4);
        }
    }

    #[test]
    /// At fractional DPI and origins, including points straddling a physical pixel, `paint` emits one
    /// text shape whose tessellated vertices, indices, UVs and colors match the reading-space reference
    /// at the same physical pixels, with text rounding both on and off.
    fn painter_preserves_visible_vertices_at_fractional_origins_and_dpi() {
        for ppp in [1.25, 1.5, 2.0] {
            let (ctx, original) = layout_with_context(MIXED, ppp, Align::Min);
            for width in [0.0, 23.3] {
                let reference = reading_space_reference(&original, width, &ALIGNMENTS);
                let aligned = mixed(original.clone(), width);
                for rounding in [false, true] {
                    ctx.tessellation_options_mut(|options| options.round_text_to_pixels = rounding);
                    let options = ctx.tessellation_options(|options| *options);
                    for origin in [pos2(37.25, 19.375), pos2(-11.5, 9.5), pos2(0.5 / ppp - 1e-5, 0.5 / ppp), pos2(0.5 / ppp + 1e-5, 0.5 / ppp)] {
                        let output = ctx.run_ui(egui::RawInput::default(), |ui| aligned.clone().paint(ui.painter(), origin, Color32::BLACK));
                        let shapes: Vec<_> = output
                            .shapes
                            .iter()
                            .filter_map(|clipped| match &clipped.shape {
                                egui::Shape::Text(shape) => Some(shape),
                                _ => None,
                            })
                            .collect();
                        assert_eq!(shapes.len(), 1, "the actual paint method must emit one text shape");
                        let atlas_size = ctx.fonts(|fonts| fonts.image().size);
                        let tessellate = |shape: &TextShape| {
                            let mut tessellator = egui::epaint::Tessellator::new(ppp, options, atlas_size, Vec::new());
                            let mut mesh = egui::epaint::Mesh::default();
                            tessellator.tessellate_text(shape, &mut mesh);
                            mesh
                        };
                        let actual = tessellate(shapes[0]);
                        let expected = tessellate(&TextShape::new(origin, reference.clone(), Color32::BLACK));
                        assert!(!expected.vertices.is_empty(), "real glyph ink must be tessellated");
                        assert_eq!(actual.indices, expected.indices);
                        assert_eq!(actual.vertices.len(), expected.vertices.len());
                        for (actual, expected) in actual.vertices.iter().zip(&expected.vertices) {
                            assert_eq!(actual.uv, expected.uv);
                            assert_eq!(actual.color, expected.color);
                            assert!(
                                (actual.pos - expected.pos).length() * ppp < 1e-4,
                                "the wrapper must not move ink to another physical pixel: {:?} != {:?}",
                                actual.pos,
                                expected.pos
                            );
                        }
                        drop(shapes);
                        output.drop_without_applying_deltas();
                    }
                }
            }
        }
    }
}
