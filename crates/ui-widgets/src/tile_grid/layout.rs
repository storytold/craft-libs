//! Where tiles, group headers and tail rows go: `arrange` lays out justified rows and card grids under group headers,
//! the tail rows follow them, and `GridLayout` is the item layout the grid hands the item step. Equal tiles without
//! groups or tail rows are `craft_behavior::TileLayout`; everything else is an arrangement. Every tile is its picture
//! plus what the tile adds around it (`Around`), in every tiling.

use crate::runtime::env::Label;
use craft_behavior::{ItemLayout, ItemSource, NavShape, ScrollPos, SizeHint, TileLayout as EqualTiles};
use egui::{Pos2, Rect, Vec2, pos2, vec2};
use std::ops::Range;
use std::sync::Arc;

/// A run of tiles under one header (a date, a folder): `start..start + count` in the order of the tiles the grid is
/// given.
///
/// The header is a row of the grid's outline, as a tree's group row is: it holds the keyboard lead, opens and closes
/// with Left and Right, reports Enter and the context-menu key, and is never selected. Its `key` names that row, so
/// it differs from the key of every tile. Positions in the grid's events (`CollectionEvent::Reorder`,
/// `FilesDropped`, the `accepts` test) count the headers as rows: the header of a group sits just before its first
/// tile.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct TileGroup<K> {
    /// The key of the header row.
    pub key: K,
    /// The first tile's position among the tiles.
    pub start: usize,
    /// How many tiles.
    pub count: usize,
    /// The header's title ("October 2026").
    pub title: String,
    /// The header's detail at the trailing end ("128 photos").
    pub detail: String,
    /// Whether the group shows its tiles; a closed group shows its header only.
    pub open: bool,
}

impl<K> TileGroup<K> {
    /// An open group of `count` tiles from tile position `start`, titled `title`, whose header row is keyed `key`.
    pub fn new(key: K, start: usize, count: usize, title: impl Into<String>) -> Self {
        TileGroup { key, start, count, title: title.into(), detail: String::new(), open: true }
    }

    /// The detail at the header's trailing end.
    #[must_use]
    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    /// Whether the group shows its tiles.
    #[must_use]
    pub fn open(mut self, open: bool) -> Self {
        self.open = open;
        self
    }
}

/// A full-width row after the tiles and their groups, below a divider: an action such as "Create preset", listed
/// after the tiles as a menu lists its commands after a separator.
///
/// The row is a row of the grid's outline, as a group header is: it holds the keyboard lead, Down from the last line
/// of tiles reaches it, and a click, Enter or AccessKit's default action reports `CollectionEvent::Activate` with its
/// `key`. It is never selected, dragged, renamed or reached by type-ahead, and no drop lands on or after it. Its `key`
/// differs from the key of every tile and header. Positions count it after every tile and header.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TailRow<K> {
    /// The key of the row.
    pub key: K,
    /// The row's label, which also names it for a screen reader.
    pub label: Label,
}

impl<K> TailRow<K> {
    /// A row keyed `key` showing `label`.
    pub fn new(key: K, label: impl Into<Label>) -> Self {
        TailRow { key, label: label.into() }
    }
}

/// The tiles each of `groups` covers among `tiles` tiles: groups are in display order, a start before the end of the
/// group before it moves up to that end, and a range ends at the last tile.
pub(super) fn group_spans<K>(groups: &[TileGroup<K>], tiles: usize) -> Vec<Range<usize>> {
    let mut at = 0;
    groups
        .iter()
        .map(|group| {
            let first = group.start.clamp(at, tiles);
            let end = group.start.saturating_add(group.count).clamp(first, tiles);
            at = end;
            first..end
        })
        .collect()
}

/// How many headers sit at or before outline position `at`, given the first tile of every group: the header of group
/// `g` sits at `starts[g] + g`.
pub(super) fn headers_through(starts: &[usize], at: usize) -> usize {
    let (mut low, mut high) = (0, starts.len());
    while low < high {
        let mid = low + (high - low) / 2;
        if starts[mid] + mid <= at {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    low
}

/// The outline position of tile `tile`: the tiles' own position plus the headers before it.
pub(super) fn outline_position(starts: &[usize], tile: usize) -> usize {
    tile + starts.partition_point(|&start| start <= tile)
}

/// What stands at an outline position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Slot {
    /// The header of the group.
    Header(usize),
    /// The tile at this position among the tiles.
    Tile(usize),
    /// The tail row at this position among the tail rows.
    Tail(usize),
}

/// What stands at outline position `at`, given the first tile of every group and the count of `tiles`: the tail rows
/// follow every tile and header.
pub(super) fn slot_at(starts: &[usize], tiles: usize, at: usize) -> Slot {
    if let Some(tail) = at.checked_sub(tiles + starts.len()) {
        return Slot::Tail(tail);
    }
    let before = headers_through(starts, at);
    match before.checked_sub(1) {
        Some(group) if starts[group] + group == at => Slot::Header(group),
        _ => Slot::Tile(at - before),
    }
}

/// What a tile adds around its picture: the padding, the caption column at the reading start and the caption band
/// under the picture. A tile is its picture plus this, whatever the tiling.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct Around {
    /// Before the picture, at the reading start: the padding and the caption column.
    pub start: f32,
    /// After the picture, at the reading end.
    pub end: f32,
    /// Above the picture.
    pub top: f32,
    /// Under the picture: the caption band and the padding.
    pub bottom: f32,
}

impl Around {
    /// `start` before the picture, `end` after it, `top` above it and `bottom` under it.
    pub fn new(start: f32, end: f32, top: f32, bottom: f32) -> Self {
        Around { start, end, top, bottom }
    }

    /// The width added to the picture's.
    pub fn width(&self) -> f32 {
        self.start + self.end
    }

    /// The height added to the picture's.
    pub fn height(&self) -> f32 {
        self.top + self.bottom
    }

    /// The width and height added.
    pub fn size(&self) -> Vec2 {
        vec2(self.width(), self.height())
    }
}

/// How the pictures of an arrangement are sized.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum Tiling {
    /// Rows of pictures at their own aspect, `row_height` high, each row but a group's last scaled to fill the
    /// width.
    #[non_exhaustive]
    Justified {
        /// The picture height of an unscaled row.
        row_height: f32,
    },
    /// Equal cards: pictures `side` wide and `side / aspect` high.
    #[non_exhaustive]
    Cards {
        /// Picture width.
        side: f32,
        /// Picture width divided by height.
        aspect: f32,
    },
}

impl Tiling {
    /// Justified rows with pictures `row_height` high.
    pub fn justified(row_height: f32) -> Self {
        Tiling::Justified { row_height }
    }

    /// Cards with pictures `side` wide.
    pub fn cards(side: f32, aspect: f32) -> Self {
        Tiling::Cards { side, aspect }
    }
}

/// One laid-out row: its tiles and its band.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct TileRow {
    /// The tiles in it, in display order.
    pub tiles: Range<usize>,
    /// The row's top, from the arrangement's top.
    pub top: f32,
    /// Its height: the pictures and what the tiles add around them.
    pub height: f32,
    /// The group it belongs to, when groups are given.
    pub group: Option<usize>,
}

/// Where every tile and header goes for one width, tiling and set of aspects.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct TileArrangement {
    /// Rows, top to bottom.
    pub rows: Vec<TileRow>,
    /// Each tile's picture rect from the arrangement's top left; `Rect::NOTHING` for a tile of a closed group.
    pub tiles: Vec<Rect>,
    /// Each group header's rect, in group order.
    pub headers: Vec<Rect>,
    /// Each group's first tile, in group order.
    pub starts: Vec<usize>,
    /// Each tail row's rect, in order, after every tile and header.
    pub tails: Vec<Rect>,
    /// The whole height.
    pub height: f32,
}

impl TileArrangement {
    /// The rows that cross `top..bottom` (from the arrangement's top), by binary search over the row tops.
    pub fn rows_between(&self, top: f32, bottom: f32) -> Range<usize> {
        let first = self.rows.partition_point(|r| r.top + r.height < top);
        let rest = self.rows.get(first..).unwrap_or(&[]);
        let count = rest.partition_point(|r| r.top <= bottom);
        first..first + count
    }

    /// The rows in order as ranges of tiles, with each run of tiles of a closed group as a range of its own.
    fn runs(&self) -> Vec<Range<usize>> {
        let mut runs = Vec::with_capacity(self.rows.len());
        let mut next = 0;
        for row in &self.rows {
            if row.tiles.start > next {
                runs.push(next..row.tiles.start);
            }
            runs.push(row.tiles.clone());
            next = row.tiles.end;
        }
        if next < self.tiles.len() {
            runs.push(next..self.tiles.len());
        }
        runs
    }

    /// The row lengths of the grid's outline: a group's header is a row of one position before its tiles' rows.
    fn outline_rows(&self) -> Vec<usize> {
        let mut lengths = Vec::with_capacity(self.rows.len() + self.starts.len());
        let mut group = 0;
        for run in self.runs() {
            let mut from = run.start;
            while from < run.end {
                while group < self.starts.len() && self.starts[group] <= from {
                    lengths.push(1);
                    group += 1;
                }
                let cut = self.starts.get(group).copied().filter(|&start| start < run.end).unwrap_or(run.end);
                lengths.push(cut - from);
                from = cut;
            }
        }
        lengths.extend(std::iter::repeat_n(1, self.starts.len() - group + self.tails.len()));
        lengths
    }

    /// Puts `count` tail rows `height` high after the tiles and headers, across the content of `width`, one gap below
    /// the last of them; the arrangement grows by them.
    pub(super) fn add_tail(&mut self, count: usize, height: f32, width: f32, spacing: ArrangeSpacing) {
        if count == 0 {
            return;
        }
        let ArrangeSpacing { gap, inset, .. } = spacing;
        let content = (width - 2.0 * inset).max(1.0);
        let mut y = self.height - inset + gap;
        for _ in 0..count {
            self.tails.push(Rect::from_min_size(pos2(inset, y), vec2(content, height)));
            y += height;
        }
        self.height = y + inset;
    }

    /// The outline position of the first tail row: every tile and header comes before it.
    pub(super) fn tail_from(&self) -> usize {
        self.tiles.len() + self.starts.len()
    }

    /// The row holding tile `at`.
    pub fn row_of(&self, at: usize) -> Option<usize> {
        let row = self.rows.partition_point(|r| r.tiles.end <= at);
        self.rows.get(row).filter(|r| r.tiles.contains(&at)).map(|_| row)
    }

    /// The header pinned at the top of a view scrolled to `top`, and where it stands: the header of the group whose
    /// tiles cross the top edge, lifted out by the next header.
    pub fn pinned(&self, top: f32) -> Option<(usize, Rect)> {
        let group = self.group_at(top)?;
        let header = *self.headers.get(group)?;
        if header.top() >= top {
            return None;
        }
        let lift = self.headers.get(group + 1).map_or(top, |next| top.min(next.top() - header.height()));
        Some((group, Rect::from_min_size(pos2(header.left(), lift), header.size())))
    }

    /// The group whose tiles cross the top edge `top`, for the pinned header.
    pub fn group_at(&self, top: f32) -> Option<usize> {
        let row = self.rows.get(self.rows_between(top, top).start)?;
        row.group
    }
}

/// The spacing `arrange` reads.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ArrangeSpacing {
    /// Between tiles.
    pub gap: f32,
    /// Around the content.
    pub inset: f32,
    /// A group header's height.
    pub group_header: f32,
}

impl ArrangeSpacing {
    /// Spacing of `gap` between tiles, `inset` around the content and headers `group_header` high.
    pub fn new(gap: f32, inset: f32, group_header: f32) -> Self {
        ArrangeSpacing { gap, inset, group_header }
    }
}

/// The aspect of every tile of `source`: `SizeHint::Aspect`, or a `SizeHint::Rect`'s width over height; a tile that
/// gives neither, or a non-finite or non-positive one, counts as 1. Section rows (group headers) are not tiles.
pub fn aspects_of<K>(source: &dyn ItemSource<K>) -> Vec<f32> {
    let sane = |aspect: f32| if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
    (0..source.len())
        .filter(|&at| !source.is_section(at))
        .map(|at| match source.size_hint(at) {
            SizeHint::Aspect(aspect) => sane(aspect),
            SizeHint::Rect(size) if size.y > 0.0 => sane(size.x / size.y),
            _ => 1.0,
        })
        .collect()
}

/// Lays out `aspects` (width over height, one per tile in display order) in `width`. A tile is its picture plus
/// `around`; the pictures are sized by `tiling`.
///
/// - `Justified`: tiles fill a row with pictures `row_height` high until the next would pass the width; the row's
///   pictures are then scaled so its tiles and gaps fill the width exactly; a group's last row keeps `row_height`.
/// - `Cards`: as many columns as fit; each group starts a new row below its header.
/// - `groups` are in display order and cover only the tiles they name; tiles outside every group sit in rows
///   without a header. A closed group has its header only.
pub fn arrange<K>(aspects: &[f32], groups: &[TileGroup<K>], width: f32, tiling: Tiling, spacing: ArrangeSpacing, around: Around) -> TileArrangement {
    let sanitized: Vec<f32> = aspects.iter().map(|a| if a.is_finite() && *a > 0.0 { *a } else { 1.0 }).collect();
    let n = sanitized.len();
    let ArrangeSpacing { gap, inset, group_header } = spacing;
    let content = (width - 2.0 * inset).max(1.0);
    let mut out = TileArrangement { tiles: vec![Rect::NOTHING; n], ..TileArrangement::default() };

    let spans = group_spans(groups, n);
    let mut segments: Vec<(Option<usize>, Range<usize>)> = Vec::new();
    let mut at = 0;
    for (index, span) in spans.iter().enumerate() {
        if span.start > at {
            segments.push((None, at..span.start));
        }
        segments.push((Some(index), span.clone()));
        at = span.end;
    }
    if at < n {
        segments.push((None, at..n));
    }
    out.starts = spans.iter().map(|span| span.start).collect();

    let mut y = inset;
    let mut started = false;
    for (group, range) in segments {
        if started {
            y += gap;
        }
        started = true;
        if let Some(index) = group {
            out.headers.push(Rect::from_min_size(pos2(inset, y), vec2(content, group_header)));
            y += group_header;
            if !groups[index].open {
                continue;
            }
        }
        let mut first_row = true;
        let mut place = |out: &mut TileArrangement, y: &mut f32, tiles: Range<usize>, height: f32| {
            if !first_row {
                *y += gap;
            }
            first_row = false;
            out.rows.push(TileRow { tiles, top: *y, height, group });
            let top = *y;
            *y += height;
            top
        };
        match tiling {
            Tiling::Justified { row_height } => {
                let mut i = range.start;
                while i < range.end {
                    // The width the pictures of tiles `i..i + count` have when the tiles and gaps fill the content.
                    let room = |count: usize| content - around.width() * count as f32 - gap * (count - 1) as f32;
                    let mut j = i + 1;
                    let mut sum = sanitized[i];
                    while j < range.end && (sum + sanitized[j]) * row_height <= room(j - i + 1) {
                        sum += sanitized[j];
                        j += 1;
                    }
                    // A picture is never scaled away: a tile wider than the content keeps a line to draw.
                    let picture = if j < range.end { (room(j - i) / sum).max(MIN_PICTURE) } else { row_height };
                    let top = place(&mut out, &mut y, i..j, picture + around.height());
                    let mut x = inset + around.start;
                    for (tile, aspect) in out.tiles[i..j].iter_mut().zip(&sanitized[i..j]) {
                        let w = aspect * picture;
                        *tile = Rect::from_min_size(pos2(x, top + around.top), vec2(w, picture));
                        x += w + around.end + gap + around.start;
                    }
                    i = j;
                }
            }
            Tiling::Cards { side, aspect } => {
                let aspect = if aspect.is_finite() && aspect > 0.0 { aspect } else { 1.0 };
                let card = side + around.width();
                let columns = columns_in(content, card, gap);
                let picture = vec2(side, side / aspect);
                let height = picture.y + around.height();
                let mut i = range.start;
                while i < range.end {
                    let j = (i + columns).min(range.end);
                    let top = place(&mut out, &mut y, i..j, height);
                    for (c, k) in (i..j).enumerate() {
                        out.tiles[k] = Rect::from_min_size(pos2(inset + around.start + c as f32 * (card + gap), top + around.top), picture);
                    }
                    i = j;
                }
            }
        }
    }
    out.height = y + inset;
    out
}

/// The least height a scaled picture of a justified row has, in points.
const MIN_PICTURE: f32 = 1.0;

/// Cards of `card` with `gap` between them that fit in `width`, at least one: the count `EqualTiles::columns` gives.
fn columns_in(width: f32, card: f32, gap: f32) -> usize {
    (((width + gap) / (card + gap)).floor() as usize).max(1)
}

/// The item layout of a tile grid.
#[derive(Clone, Debug)]
pub struct GridLayout {
    kind: Kind,
    recipe: Option<Recipe>,
}

/// What an arrangement was made from. A pass whose recipe equals the last one's reuses the layout.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Recipe {
    /// The rows stamp: the set of tiles, as the app names it.
    pub(super) stamp: u64,
    pub(super) width: f32,
    pub(super) tiles: usize,
    pub(super) side: f32,
    pub(super) aspect: f32,
    pub(super) justified: bool,
    pub(super) around: Around,
    /// Each group's first tile, count and open state.
    pub(super) groups: Vec<(usize, usize, bool)>,
    /// How many tail rows follow, and their height.
    pub(super) tail: (usize, f32),
}

#[derive(Clone, Debug)]
enum Kind {
    /// Equal cards in no group, `inset` from every edge.
    Equal { tiles: EqualTiles, inset: f32 },
    /// An arrangement; each card is its picture and `around`.
    Arranged { arrangement: Arc<TileArrangement>, rows: Arc<[usize]>, starts: Arc<[usize]>, width: f32, around: Around },
}

impl GridLayout {
    /// Equal cards of `card` in no group: `craft_behavior::TileLayout` inside `inset` on every side.
    pub fn equal<K>(source: &dyn ItemSource<K>, width: f32, inset: f32, card: Vec2, gap: f32) -> Self {
        let tiles = EqualTiles::new(source, (width - 2.0 * inset).max(1.0), card, gap);
        GridLayout { kind: Kind::Equal { tiles, inset }, recipe: None }
    }

    /// The layout `arrangement` makes in `width`; each card is its picture and `around`. The positions of the layout are
    /// those of the grid's outline: the header of every group stands just before the group's first tile, and the tail
    /// rows follow every tile and header.
    pub fn arranged(arrangement: TileArrangement, width: f32, around: Around) -> Self {
        let rows = arrangement.outline_rows().into();
        let starts = arrangement.starts.as_slice().into();
        GridLayout { kind: Kind::Arranged { arrangement: Arc::new(arrangement), rows, starts, width, around }, recipe: None }
    }

    /// The layout remembers what it was made from.
    pub(super) fn made_from(mut self, recipe: Recipe) -> Self {
        self.recipe = Some(recipe);
        self
    }

    /// Whether the layout was made from `recipe`.
    pub(super) fn is_made_from(&self, recipe: &Recipe) -> bool {
        self.recipe.as_ref() == Some(recipe)
    }

    /// The arrangement behind the layout, when it is not a plain equal grid.
    pub fn arrangement(&self) -> Option<&Arc<TileArrangement>> {
        match &self.kind {
            Kind::Arranged { arrangement, .. } => Some(arrangement),
            Kind::Equal { .. } => None,
        }
    }

    /// The card of tile `at` in the arrangement: its picture and what the tile adds around it.
    fn card(arrangement: &TileArrangement, around: Around, at: usize) -> Rect {
        let picture = arrangement.tiles.get(at).copied().unwrap_or(Rect::NOTHING);
        if picture == Rect::NOTHING {
            return Rect::NOTHING;
        }
        Rect::from_min_max(
            pos2(picture.left() - around.start, picture.top() - around.top),
            pos2(picture.right() + around.end, picture.bottom() + around.bottom),
        )
    }
}

fn shifted(offset: ScrollPos, inset: f32) -> ScrollPos {
    ScrollPos { x: offset.x - f64::from(inset), y: offset.y - f64::from(inset) }
}

impl ItemLayout for GridLayout {
    fn content_size(&self) -> ScrollPos {
        match &self.kind {
            Kind::Equal { tiles, inset } => {
                let inner = tiles.content_size();
                let pad = 2.0 * f64::from(*inset);
                ScrollPos { x: inner.x + pad, y: if tiles.count == 0 { 0.0 } else { inner.y + pad } }
            }
            Kind::Arranged { arrangement, width, .. } => ScrollPos { x: f64::from(*width), y: f64::from(arrangement.height) },
        }
    }

    fn visible(&self, offset: ScrollPos, view: Vec2, out: &mut Vec<usize>) {
        match &self.kind {
            Kind::Equal { tiles, inset } => tiles.visible(shifted(offset, *inset), view, out),
            Kind::Arranged { arrangement, starts, .. } => {
                let top = offset.y as f32;
                let bottom = top + view.y;
                let mut found: Vec<usize> = Vec::new();
                for row in &arrangement.rows[arrangement.rows_between(top, bottom)] {
                    found.extend(row.tiles.clone().map(|tile| outline_position(starts, tile)));
                }
                let pinned = arrangement.pinned(top).map(|(group, _)| group);
                let shown = arrangement.headers.iter().enumerate().filter(|(_, header)| header.bottom() >= top && header.top() <= bottom);
                found.extend(shown.map(|(group, _)| group).chain(pinned).map(|group| starts[group] + group));
                let tails = arrangement.tails.iter().enumerate().filter(|(_, tail)| tail.bottom() >= top && tail.top() <= bottom);
                found.extend(tails.map(|(tail, _)| arrangement.tail_from() + tail));
                found.sort_unstable();
                found.dedup();
                out.extend(found);
            }
        }
    }

    fn rect(&self, at: usize, offset: ScrollPos) -> Rect {
        let behind = vec2(-(offset.x as f32), -(offset.y as f32));
        match &self.kind {
            Kind::Equal { tiles, inset } => tiles.rect(at, shifted(offset, *inset)),
            Kind::Arranged { arrangement, starts, around, .. } => match slot_at(starts, arrangement.tiles.len(), at) {
                Slot::Tail(tail) => arrangement.tails.get(tail).map_or(Rect::NOTHING, |rect| rect.translate(behind)),
                Slot::Header(group) => {
                    let pinned = arrangement.pinned(offset.y as f32).filter(|(held, _)| *held == group);
                    pinned
                        .map(|(_, rect)| rect)
                        .or_else(|| arrangement.headers.get(group).copied())
                        .map_or(Rect::NOTHING, |rect| rect.translate(behind))
                }
                Slot::Tile(tile) => {
                    let card = Self::card(arrangement, *around, tile);
                    if card == Rect::NOTHING { card } else { card.translate(behind) }
                }
            },
        }
    }

    fn hit(&self, point: Pos2, offset: ScrollPos) -> Option<usize> {
        match &self.kind {
            Kind::Equal { tiles, inset } => tiles.hit(point, shifted(offset, *inset)),
            Kind::Arranged { arrangement, starts, around, .. } => {
                let at = pos2(point.x + offset.x as f32, point.y + offset.y as f32);
                let header = |group: usize| starts[group] + group;
                if let Some((group, rect)) = arrangement.pinned(offset.y as f32)
                    && rect.contains(at)
                {
                    return Some(header(group));
                }
                if let Some(tail) = arrangement.tails.iter().position(|tail| tail.contains(at)) {
                    return Some(arrangement.tail_from() + tail);
                }
                let below = arrangement.headers.partition_point(|header| header.top() <= at.y);
                if let Some(group) = below.checked_sub(1).filter(|&group| arrangement.headers[group].contains(at)) {
                    return Some(header(group));
                }
                let row = arrangement.rows.get(arrangement.rows_between(at.y, at.y).start)?;
                row.tiles.clone().find(|&tile| Self::card(arrangement, *around, tile).contains(at)).map(|tile| outline_position(starts, tile))
            }
        }
    }

    fn nav(&self) -> NavShape {
        match &self.kind {
            Kind::Equal { tiles, .. } => tiles.nav(),
            Kind::Arranged { .. } => NavShape::RaggedGrid,
        }
    }

    fn row_len(&self, row: usize) -> Option<usize> {
        match &self.kind {
            Kind::Arranged { rows, .. } => rows.get(row).copied(),
            Kind::Equal { .. } => None,
        }
    }

    fn shown(&self, at: usize) -> bool {
        match &self.kind {
            Kind::Equal { .. } => true,
            Kind::Arranged { arrangement, starts, .. } => match slot_at(starts, arrangement.tiles.len(), at) {
                Slot::Header(group) => group < arrangement.headers.len(),
                Slot::Tail(tail) => tail < arrangement.tails.len(),
                Slot::Tile(tile) => arrangement.tiles.get(tile).is_some_and(|rect| *rect != Rect::NOTHING),
            },
        }
    }
}
