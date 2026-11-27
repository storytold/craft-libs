use craft_zip::{Compression, Writer};
use std::io::{Cursor, Read, Write};

#[test]
fn aligned_stored_matches_previous_usdz_writer() {
    for tail in 0..128 {
        let payload = vec![37u8; tail];
        let entries: [(&str, &[u8]); 3] = [("scene.usdc", b"PXR-USDC"), ("textures/synthetic.png", &payload), ("empty", b"")];
        let mut shared = Writer::new();
        let mut previous = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored).with_alignment(64);
        for (name, data) in entries {
            shared.add_stored_aligned(name, data, 64).unwrap();
            previous.start_file(name, options).unwrap();
            previous.write_all(data).unwrap();
        }
        let bytes = shared.finish().unwrap();
        assert_eq!(bytes, previous.finish().unwrap().into_inner());
        let mut archive = zip::ZipArchive::new(Cursor::new(&bytes)).unwrap();
        for (i, (name, expected)) in entries.iter().enumerate() {
            let mut file = archive.by_index(i).unwrap();
            assert_eq!(file.name().unwrap(), *name);
            assert_eq!(file.compression(), zip::CompressionMethod::Stored);
            assert_eq!(file.data_start().unwrap() % 64, 0);
            let mut actual = Vec::new();
            file.read_to_end(&mut actual).unwrap();
            assert_eq!(actual, *expected);
        }
    }
}

#[test]
fn invalid_alignment_does_not_start_entry_and_duplicates_fail() {
    let mut writer = Writer::new();
    for alignment in [0, 3, 65, 8192, u16::MAX] {
        assert!(writer.add_stored_aligned("layer", b"data", alignment).is_err());
    }
    writer.add_stored_aligned("layer", b"data", 64).unwrap();
    assert!(writer.add_stored_aligned("layer", b"again", 64).is_err());
}

#[test]
fn alignment_one_retains_ordinary_stored_bytes() {
    let mut ordinary = Writer::new();
    ordinary.add("entry", b"data", Compression::Stored).unwrap();
    let mut aligned = Writer::new();
    aligned.add_stored_aligned("entry", b"data", 1).unwrap();
    assert_eq!(ordinary.finish().unwrap(), aligned.finish().unwrap());
}
