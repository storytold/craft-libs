//! A grid's ICC profile may sit on its tiles (a OnePlus 11 writes one Display P3 `prof` on each of
//! its 48 tiles and none on the grid). Ported from PhotoCraft's `photocraft-heif`. The containers
//! are built by hand and never decoded; each test first checks the fixture has the shape it claims.

use super::*;

/// Stands in for a profile: `colr` hands its payload back without parsing it.
const TILE_ICC: &[u8] = b"tile-profile";
const GRID_ICC: &[u8] = b"grid-profile";

const GRID_ID: u16 = 1;
const PICTURE_ID: u16 = 2;

fn bx(kind: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = u32::try_from(payload.len() + 8).unwrap().to_be_bytes().to_vec();
    v.extend_from_slice(kind);
    v.extend_from_slice(payload);
    v
}

/// A box whose first four payload bytes are a version and 24 bits of flags.
fn full(kind: &[u8; 4], version: u8, payload: &[u8]) -> Vec<u8> {
    let mut p = vec![version, 0, 0, 0];
    p.extend_from_slice(payload);
    bx(kind, &p)
}

fn hdlr() -> Vec<u8> {
    let mut p = vec![0; 4];
    p.extend_from_slice(b"pict");
    p.extend_from_slice(&[0; 12]);
    p.push(0);
    full(b"hdlr", 0, &p)
}

fn infe(id: u16, item_type: &[u8; 4]) -> Vec<u8> {
    let mut p = id.to_be_bytes().to_vec();
    p.extend_from_slice(&0u16.to_be_bytes());
    p.extend_from_slice(item_type);
    p.push(0);
    full(b"infe", 2, &p)
}

fn iref_dimg(from: u16, to: u16) -> Vec<u8> {
    let mut p = from.to_be_bytes().to_vec();
    p.extend_from_slice(&1u16.to_be_bytes());
    p.extend_from_slice(&to.to_be_bytes());
    full(b"iref", 0, &bx(b"dimg", &p))
}

fn colr_prof(icc: &[u8]) -> Vec<u8> {
    let mut p = b"prof".to_vec();
    p.extend_from_slice(icc);
    bx(b"colr", &p)
}

fn ispe(w: u32, h: u32) -> Vec<u8> {
    let mut p = w.to_be_bytes().to_vec();
    p.extend_from_slice(&h.to_be_bytes());
    full(b"ispe", 0, &p)
}

fn ipma(entries: &[(u16, &[u8])]) -> Vec<u8> {
    let mut p = u32::try_from(entries.len()).unwrap().to_be_bytes().to_vec();
    for (item, assoc) in entries {
        p.extend_from_slice(&item.to_be_bytes());
        p.push(u8::try_from(assoc.len()).unwrap());
        p.extend_from_slice(assoc);
    }
    full(b"ipma", 0, &p)
}

/// `item`'s bytes live in `idat` (construction method 1) at offset 0.
fn iloc_in_idat(item: u16, length: u32) -> Vec<u8> {
    let mut p = vec![0x44, 0x00];
    p.extend_from_slice(&1u16.to_be_bytes());
    p.extend_from_slice(&item.to_be_bytes());
    p.extend_from_slice(&1u16.to_be_bytes());
    p.extend_from_slice(&0u16.to_be_bytes());
    p.extend_from_slice(&1u16.to_be_bytes());
    p.extend_from_slice(&0u32.to_be_bytes());
    p.extend_from_slice(&length.to_be_bytes());
    full(b"iloc", 1, &p)
}

fn file(meta: &[u8]) -> Vec<u8> {
    let mut ftyp = b"heic".to_vec();
    ftyp.extend_from_slice(&0u32.to_be_bytes());
    ftyp.extend_from_slice(b"mif1heic");
    let mut out = bx(b"ftyp", &ftyp);
    out.extend_from_slice(&full(b"meta", 0, meta));
    out
}

/// A 1x1 64x64 grid, with `grid_icc` on the grid item and `tile_icc` on its single tile.
///
/// `ipco` children are numbered from 1 in file order; an association byte's top bit marks the
/// property essential, as macOS writes it.
fn grid_file(grid_icc: Option<&[u8]>, tile_icc: Option<&[u8]>) -> Vec<u8> {
    let mut ipco = ispe(64, 64);
    let (mut grid_assoc, mut tile_assoc, mut index) = (vec![0x81], vec![0x81], 1u8);
    if let Some(icc) = grid_icc {
        index += 1;
        ipco.extend_from_slice(&colr_prof(icc));
        grid_assoc.push(0x80 | index);
    }
    if let Some(icc) = tile_icc {
        index += 1;
        ipco.extend_from_slice(&colr_prof(icc));
        tile_assoc.push(0x80 | index);
    }
    let mut iprp = bx(b"ipco", &ipco);
    iprp.extend_from_slice(&ipma(&[(GRID_ID, grid_assoc.as_slice()), (PICTURE_ID, tile_assoc.as_slice())]));

    let mut items = 2u16.to_be_bytes().to_vec();
    items.extend_from_slice(&infe(GRID_ID, b"grid"));
    items.extend_from_slice(&infe(PICTURE_ID, b"hvc1"));

    // Version 0, flags 0, one row, one column, 64x64 output.
    const DESCRIPTOR: &[u8] = &[0, 0, 0, 0, 0, 64, 0, 64];

    let mut meta = hdlr();
    meta.extend_from_slice(&full(b"pitm", 0, &GRID_ID.to_be_bytes()));
    meta.extend_from_slice(&full(b"iinf", 0, &items));
    meta.extend_from_slice(&iref_dimg(GRID_ID, PICTURE_ID));
    meta.extend_from_slice(&bx(b"iprp", &iprp));
    meta.extend_from_slice(&bx(b"idat", DESCRIPTOR));
    meta.extend_from_slice(&iloc_in_idat(GRID_ID, u32::try_from(DESCRIPTOR.len()).unwrap()));
    file(&meta)
}

/// One coded picture rather than a derivation, with `icc` on the item itself.
fn picture_file(icc: Option<&[u8]>) -> Vec<u8> {
    let mut ipco = ispe(64, 64);
    let mut assoc = vec![0x81];
    if let Some(icc) = icc {
        ipco.extend_from_slice(&colr_prof(icc));
        assoc.push(0x82);
    }
    let mut iprp = bx(b"ipco", &ipco);
    iprp.extend_from_slice(&ipma(&[(PICTURE_ID, assoc.as_slice())]));

    let mut items = 1u16.to_be_bytes().to_vec();
    items.extend_from_slice(&infe(PICTURE_ID, b"hvc1"));

    let mut meta = hdlr();
    meta.extend_from_slice(&full(b"pitm", 0, &PICTURE_ID.to_be_bytes()));
    meta.extend_from_slice(&full(b"iinf", 0, &items));
    meta.extend_from_slice(&bx(b"iprp", &iprp));
    file(&meta)
}

fn ctx_of(bytes: &[u8]) -> Context<'_> {
    Context::open(bytes).expect("the synthetic container must parse")
}

#[test]
fn a_grid_inherits_its_tiles_icc_profile() {
    let bytes = grid_file(None, Some(TILE_ICC));
    let ctx = ctx_of(&bytes);
    let primary = ctx.meta.primary;
    assert_eq!(primary, u32::from(GRID_ID));
    let (grid, tiles) = ctx.grid(primary).unwrap().expect("a grid derivation");
    assert_eq!((grid.rows, grid.columns, tiles.len()), (1, 1, 1));
    assert!(ctx.props(primary).unwrap().icc.is_none(), "the grid item carries no colr of its own");
    assert_eq!(ctx.props(tiles[0]).unwrap().icc, Some(TILE_ICC), "the tile carries the profile");

    assert_eq!(item_icc(&ctx, primary).as_deref(), Some(TILE_ICC));
}

#[test]
fn a_grids_own_profile_wins_over_its_tiles() {
    let bytes = grid_file(Some(GRID_ICC), Some(TILE_ICC));
    let ctx = ctx_of(&bytes);
    assert_eq!(item_icc(&ctx, ctx.meta.primary).as_deref(), Some(GRID_ICC));
}

#[test]
fn an_unlabelled_grid_has_no_profile() {
    let bytes = grid_file(None, None);
    let ctx = ctx_of(&bytes);
    assert_eq!(item_icc(&ctx, ctx.meta.primary), None);
}

#[test]
fn a_coded_picture_resolves_its_own_profile() {
    let labelled = picture_file(Some(GRID_ICC));
    let ctx = ctx_of(&labelled);
    assert!(ctx.grid(ctx.meta.primary).unwrap().is_none(), "not a derivation");
    assert_eq!(item_icc(&ctx, ctx.meta.primary).as_deref(), Some(GRID_ICC));

    let bare = picture_file(None);
    let ctx = ctx_of(&bare);
    assert_eq!(item_icc(&ctx, ctx.meta.primary), None);
}
