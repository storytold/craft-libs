use super::*;
use crate::compression::Compression;
use crate::header::{ColorMode, Header, Version};
use crate::image_data::ImageData;
use crate::layer::{LayerInfo, LayerRecord};

fn pascal(names: &[&str]) -> Vec<u8> {
    names.iter().flat_map(|n| std::iter::once(n.len() as u8).chain(n.bytes())).collect()
}

fn unicode(names: &[&str]) -> Vec<u8> {
    names.iter().flat_map(|n| [(n.len() as u32).to_be_bytes().to_vec(), n.encode_utf16().flat_map(u16::to_be_bytes).collect()].concat()).collect()
}

fn display(kinds: &[u8]) -> Vec<u8> {
    let mut out = 1u32.to_be_bytes().to_vec();
    for k in kinds {
        out.extend_from_slice(&[0, 0, 0xff, 0xff, 0, 0, 0, 0, 0, 0, 0, 50, *k]);
    }
    out
}

fn get(resources: &[ImageResource], id: u16) -> Option<&[u8]> {
    resources.iter().find(|r| r.id == id).map(|r| r.data.as_slice())
}

/// The resources a PSD model with merged transparency and three spot or alpha channels has.
fn model_resources(quick_mask: u16) -> Vec<ImageResource> {
    vec![
        ImageResource::new(ids::ALPHA_NAMES, pascal(&["Transparency", "Spot", "Alpha", "Mask"])),
        ImageResource::new(ids::UNICODE_ALPHA_NAMES, unicode(&["Transparency", "Spot", "Alpha", "Mask"])),
        ImageResource::new(ids::DISPLAY_INFO, display(&[0, 2, 0, 0])),
        ImageResource::new(ids::ALPHA_IDENTIFIERS, alpha_identifiers(true, 3)),
        ImageResource::new(ids::QUICK_MASK_INFO, [quick_mask.to_be_bytes().as_slice(), &[0]].concat()),
    ]
}

#[test]
fn alpha_identifiers_are_numbered_as_photoshop_does() {
    let ids =
        |transparency, count| alpha_identifiers(transparency, count).as_chunks::<4>().0.iter().map(|c| u32::from_be_bytes(*c)).collect::<Vec<_>>();
    assert_eq!(ids(false, 3), [3, 4, 5]);
    assert_eq!(ids(true, 3), [0, 3, 4, 5]);
    assert!(ids(false, 0).is_empty());
    assert_eq!(ids(true, 0), [0]);
}

#[test]
fn without_the_transparency_in_the_ifd_its_entries_go() {
    // RGB with transparency and three channels; the IFD stores colour and the three channels.
    let mut r = model_resources(6);
    ifd_channel_resources(&mut r, IfdChannels { color: 3, merged_alpha: true, write_alpha: false, extra: 3, kept: 3 });
    assert_eq!(get(&r, ids::ALPHA_NAMES), Some(pascal(&["Spot", "Alpha", "Mask"]).as_slice()));
    assert_eq!(get(&r, ids::UNICODE_ALPHA_NAMES), Some(unicode(&["Spot", "Alpha", "Mask"]).as_slice()));
    assert_eq!(get(&r, ids::DISPLAY_INFO), Some(display(&[2, 0, 0]).as_slice()));
    assert_eq!(get(&r, ids::ALPHA_IDENTIFIERS), Some(alpha_identifiers(false, 3).as_slice()));
    // The Quick Mask was merged channel 6 (R, G, B, transparency, spot, alpha, mask): sample 5.
    assert_eq!(get(&r, ids::QUICK_MASK_INFO), Some(&[0, 5, 0][..]));
}

#[test]
fn with_the_transparency_in_the_ifd_nothing_moves() {
    let mut r = model_resources(6);
    ifd_channel_resources(&mut r, IfdChannels { color: 3, merged_alpha: true, write_alpha: true, extra: 3, kept: 3 });
    assert_eq!(r, model_resources(6));
}

#[test]
fn past_the_cap_only_the_stored_channels_are_described() {
    // Two of three channels kept, transparency not written: a Quick Mask on the third goes.
    let mut r = model_resources(6);
    ifd_channel_resources(&mut r, IfdChannels { color: 3, merged_alpha: true, write_alpha: false, extra: 3, kept: 2 });
    assert_eq!(get(&r, ids::ALPHA_NAMES), Some(pascal(&["Spot", "Alpha"]).as_slice()));
    assert_eq!(get(&r, ids::UNICODE_ALPHA_NAMES), Some(unicode(&["Spot", "Alpha"]).as_slice()));
    assert_eq!(get(&r, ids::DISPLAY_INFO), Some(display(&[2, 0]).as_slice()));
    assert_eq!(get(&r, ids::ALPHA_IDENTIFIERS), Some(alpha_identifiers(false, 2).as_slice()));
    assert_eq!(get(&r, ids::QUICK_MASK_INFO), None, "the Quick Mask was the dropped channel");
    // One on a kept channel stays.
    let mut r = model_resources(5);
    ifd_channel_resources(&mut r, IfdChannels { color: 3, merged_alpha: true, write_alpha: false, extra: 3, kept: 2 });
    assert_eq!(get(&r, ids::QUICK_MASK_INFO), Some(&[0, 4, 0][..]));
    // Transparency written: its entry stays ahead of the kept channels.
    let mut r = model_resources(4);
    ifd_channel_resources(&mut r, IfdChannels { color: 3, merged_alpha: true, write_alpha: true, extra: 3, kept: 1 });
    assert_eq!(get(&r, ids::ALPHA_NAMES), Some(pascal(&["Transparency", "Spot"]).as_slice()));
    assert_eq!(get(&r, ids::DISPLAY_INFO), Some(display(&[0, 2]).as_slice()));
    assert_eq!(get(&r, ids::QUICK_MASK_INFO), Some(&[0, 4, 0][..]));
}

#[test]
fn resources_too_short_for_their_entries_are_left_alone() {
    let short = vec![
        ImageResource::new(ids::ALPHA_NAMES, vec![9, b'a']),
        ImageResource::new(ids::UNICODE_ALPHA_NAMES, vec![0, 0, 0, 40, 0, b'a']),
        ImageResource::new(ids::DISPLAY_INFO, vec![0, 0, 0, 1, 7]),
        ImageResource::new(ids::QUICK_MASK_INFO, vec![3]),
    ];
    for c in [
        IfdChannels { color: 3, merged_alpha: true, write_alpha: false, extra: 3, kept: 3 },
        IfdChannels { color: 3, merged_alpha: false, write_alpha: false, extra: 5, kept: 1 },
    ] {
        let mut r = short.clone();
        ifd_channel_resources(&mut r, c);
        let unchanged: Vec<_> = r.iter().filter(|r| r.id != ids::ALPHA_IDENTIFIERS).cloned().collect();
        assert_eq!(unchanged, short, "{c:?}");
    }
    // Lengths that overflow are refused, not wrapped.
    assert_eq!(unicode_end(&[0xff, 0xff, 0xff, 0xff], 1), None);
    assert_eq!(pascal_end(&[], 0), Some(0));
}

fn file_with(channels: u16, merged_alpha: bool, blocks: Vec<TaggedBlock>, mask: Option<GlobalLayerMask>) -> PsdFile {
    let blocks_of_layer =
        vec![TaggedBlock::new(*b"luni", vec![0; 4]), TaggedBlock::new(*b"lyid", vec![0, 0, 0, 7]), TaggedBlock::new(*b"lsct", vec![0; 4])];
    let layer = LayerRecord { blocks: blocks_of_layer, ..LayerRecord::default() };
    let isd = ImageSourceData {
        version: Version::Psd,
        byte_order: crate::tiff::ByteOrder::Big,
        layer_info: Some(LayerInfo { merged_alpha, layers: vec![layer], ..LayerInfo::default() }),
        layer_info_key: *b"Layr",
        global_layer_mask: mask,
        global_blocks: blocks,
        trailing: Vec::new(),
    };
    isd.into_psd(Header::new(Version::Psd, 2, 2, channels, 8, ColorMode::Rgb), Vec::new(), ImageData::new(Compression::Raw, Vec::new()))
}

fn keys(blocks: &[TaggedBlock]) -> Vec<String> {
    blocks.iter().map(TaggedBlock::key_str).collect()
}

#[test]
fn layer_data_is_shaped_as_photoshop_writes_it() {
    // Colour, transparency and a spot; Save Transparency on.
    let mut file = file_with(5, true, vec![TaggedBlock::new(*b"Txt2", vec![1])], Some(GlobalLayerMask { data: Vec::new() }));
    let isd = ImageSourceData::for_tiff(&mut file, true);
    assert_eq!(keys(&isd.global_blocks), ["MTrn", "Patt", "Txt2", "FMsk"]);
    assert_eq!(isd.global_blocks[3].data, FILTER_MASK_DEFAULT);
    assert_eq!(isd.global_layer_mask, Some(GlobalLayerMask::photoshop_default()), "a short LMsk is replaced");
    let info = isd.layer_info.as_ref().unwrap();
    assert!(!info.merged_alpha, "a positive layer count with MTrn marks the transparency");
    let layer = &info.layers[0];
    assert!(layer.flags.0 & LayerFlags::BIT4_USEFUL != 0);
    assert_eq!(keys(&layer.blocks), ["luni", "lyid", "lnsr", "clbl", "infx", "knko", "lspf", "lclr", "lsct"]);
    assert!(file.layer_info.is_none() && file.global_blocks.is_empty(), "the layers moved out");

    // Colour and transparency only, Save Transparency on: no marker, the count stays negative.
    let mut file = file_with(4, true, vec![TaggedBlock::new(*b"Pat2", vec![1, 2])], Some(GlobalLayerMask::photoshop_default()));
    let isd = ImageSourceData::for_tiff(&mut file, true);
    assert_eq!(keys(&isd.global_blocks), ["Pat2", "FMsk"], "an existing pattern block stays first");
    assert!(isd.layer_info.as_ref().unwrap().merged_alpha);

    // Save Transparency off: a kept marker would make the first spot read as transparency.
    let mut file = file_with(5, true, vec![TaggedBlock::new(*b"Mtrn", Vec::new()), TaggedBlock::new(*b"FMsk", vec![9])], None);
    let isd = ImageSourceData::for_tiff(&mut file, false);
    assert_eq!(keys(&isd.global_blocks), ["Patt", "FMsk"]);
    assert_eq!(isd.global_blocks[1].data, [9], "an existing filter mask is kept");
    assert!(!isd.layer_info.as_ref().unwrap().merged_alpha);
    assert_eq!(isd.global_layer_mask, None);
}

#[test]
fn layer_blocks_already_present_are_not_added_twice() {
    let mut blocks = vec![TaggedBlock::new(*b"luni", vec![0; 4]), TaggedBlock::knockout(1), TaggedBlock::sheet_color(3)];
    photoshop_layer_blocks(&mut blocks);
    assert_eq!(keys(&blocks), ["luni", "lnsr", "clbl", "infx", "lspf", "knko", "lclr"]);
    let mut none = Vec::new();
    photoshop_layer_blocks(&mut none);
    assert_eq!(keys(&none), ["lnsr", "clbl", "infx", "knko", "lspf", "lclr"], "appended when there is no id");
}

#[test]
fn padding_is_dropped_on_open() {
    let mut isd = ImageSourceData::for_tiff(&mut file_with(3, false, vec![TaggedBlock::new(*b"Pat3", vec![1])], None), false);
    isd.global_blocks.push(TaggedBlock::new(*b"Pat2", Vec::new()));
    isd.drop_photoshop_padding();
    assert_eq!(keys(&isd.global_blocks), ["Pat3"], "empty patterns and the default filter mask go; content stays");
}

#[test]
fn foreign_order_blocks_are_stripped_and_reported() {
    let foreign = |key: [u8; 4], len: usize| TaggedBlock { signature: *b"MIB8", ..TaggedBlock::new(key, vec![0; len]) };
    let mut file = file_with(3, false, vec![foreign(*b"zFoo", 5), TaggedBlock::new(*b"Txt2", vec![1])], None);
    if let Some(info) = &mut file.layer_info {
        info.layers[0].blocks.push(foreign(*b"zBar", 3));
    }
    let stripped = strip_foreign_order_blocks(&mut file);
    assert_eq!(stripped, [StrippedBlock { layer: Some(0), key: *b"zBar", len: 3 }, StrippedBlock { layer: None, key: *b"zFoo", len: 5 }]);
    assert_eq!(keys(&file.global_blocks), ["Txt2"]);
    assert_eq!(file.layer_info.as_ref().map(|i| i.layers[0].blocks.len()), Some(3));
}

#[test]
fn samples_round_trip_between_interleaved_and_planar() {
    // Two pixels of CMYK + one spot, 16-bit: ink inverted in the planes, the spot not.
    let px: [[u16; 5]; 2] = [[0, 1000, 65535, 32768, 7], [65535, 0, 1, 2, 9]];
    let interleaved: Vec<u8> = px.iter().flatten().flat_map(|v| v.to_ne_bytes()).collect();
    let planes = interleaved_to_planar(&interleaved, 2, 5, 2, 4).unwrap();
    let be: Vec<u16> = planes.as_chunks::<2>().0.iter().map(|b| u16::from_be_bytes(*b)).collect();
    assert_eq!(be, [65535, 0, 64535, 65535, 0, 65534, 32767, 65533, 7, 9]);
    assert_eq!(planar_to_interleaved(&planes, 2, 5, 2, 4).unwrap(), interleaved);
    // Fewer channels than the planes hold: the colour of an image whose extra planes follow.
    let rgb = planar_to_interleaved(&planes, 2, 3, 2, 0).unwrap();
    assert_eq!(rgb.len(), 2 * 3 * 2);
    // 8-bit and float.
    assert_eq!(interleaved_to_planar(&[1, 2, 3, 4, 5, 6], 3, 2, 1, 1).unwrap(), [254, 252, 250, 2, 4, 6]);
    let f: Vec<u8> = [0.25f32, 2.0].iter().flat_map(|v| v.to_ne_bytes()).collect();
    let fp = interleaved_to_planar(&f, 1, 2, 4, 1).unwrap();
    assert_eq!(fp, [0.75f32.to_be_bytes(), 2.0f32.to_be_bytes()].concat());
    assert_eq!(planar_to_interleaved(&fp, 1, 2, 4, 1).unwrap(), f);
    assert_eq!(swap_plane(&swap_plane(&interleaved, 2).unwrap(), 2).unwrap(), interleaved);
    // Big-endian 1 becomes a native 1.
    assert_eq!(swap_plane(&[0, 1], 2).unwrap(), 1u16.to_ne_bytes());
}

#[test]
fn sample_layouts_that_do_not_fit_are_errors() {
    assert!(interleaved_to_planar(&[0; 5], 2, 3, 1, 0).is_err(), "short");
    assert!(interleaved_to_planar(&[0; 6], 2, 1, 3, 0).is_err(), "3-byte samples");
    assert!(interleaved_to_planar(&[], 0, 0, 1, 0).is_err(), "no channels");
    assert!(interleaved_to_planar(&[], usize::MAX, 2, 4, 0).is_err(), "overflow");
    assert!(planar_to_interleaved(&[0; 5], 2, 3, 1, 0).is_err(), "planes too short");
    assert!(swap_plane(&[0; 3], 2).is_err() && swap_plane(&[0; 4], 8).is_err());
    assert_eq!(interleaved_to_planar(&[], 0, 3, 2, 0).unwrap(), Vec::<u8>::new(), "an empty image");
}
