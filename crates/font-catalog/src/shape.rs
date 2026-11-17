//! Shape a run with its source clusters and surrounding text context.

use std::sync::Arc;

use harfrust::{BufferClusterLevel, Direction, Feature, Language, Script, ShapeOptions, ShapePlan, Shaper, UnicodeBuffer};

/// Parameters for a single directional run. Text is in logical order and `source_start` is its
/// byte offset in the complete source. Context influences joining but emits no glyphs.
pub struct RunInput<'a> {
    /// Text in this run.
    pub text: &'a str,
    /// Its byte offset in the complete source.
    pub source_start: usize,
    /// Text before the run, within the same shaping context.
    pub pre_context: &'a str,
    /// Text after the run, within the same shaping context.
    pub post_context: &'a str,
    /// Direction of shaping.
    pub direction: Direction,
    /// Language used for language-specific forms.
    pub language: Option<&'a Language>,
    /// OpenType features with ranges in source byte coordinates.
    pub features: &'a [Feature],
    /// Use character-level instead of monotone grapheme clusters.
    pub character_clusters: bool,
    /// Uppercase each mapped character without losing its source cluster.
    pub uppercase: bool,
}

/// A glyph in design units, with its source-byte cluster and directional advances and offsets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunGlyph {
    pub glyph_id: u32,
    pub cluster: u32,
    pub x_advance: i32,
    pub y_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

/// A run's source offsets cannot fit HarfRust's 32-bit clusters.
#[derive(Debug, thiserror::Error)]
#[error("shaping cluster exceeds the 32-bit source offset range")]
pub struct ShapeRunError;

/// Shape a run using a caller's face-specific plan cache. `map_character` applies digit or other
/// character mapping before case conversion; expansions retain the original byte cluster.
/// The plan callback receives the guessed script, direction, language and feature set so it can
/// reuse plans for the particular shaper and its variable-font instance.
pub fn shape_run(
    shaper: &Shaper<'_>,
    input: &RunInput<'_>,
    mut map_character: impl FnMut(char) -> char,
    plan: impl FnOnce(&Shaper<'_>, Direction, Option<Script>, Option<&Language>, &[Feature]) -> Arc<ShapePlan>,
) -> Result<Vec<RunGlyph>, ShapeRunError> {
    let mut buffer = UnicodeBuffer::new();
    for (offset, character) in input.text.char_indices() {
        let cluster = input.source_start.checked_add(offset).and_then(|n| u32::try_from(n).ok()).ok_or(ShapeRunError)?;
        let character = map_character(character);
        if input.uppercase {
            for upper in character.to_uppercase() {
                buffer.add(upper, cluster);
            }
        } else {
            buffer.add(character, cluster);
        }
    }
    buffer.set_direction(input.direction);
    buffer.guess_segment_properties();
    if input.character_clusters {
        buffer.set_cluster_level(BufferClusterLevel::Characters);
    }
    if let Some(language) = input.language {
        buffer.set_language(language.clone());
    }
    buffer.set_pre_context(input.pre_context);
    buffer.set_post_context(input.post_context);
    let script = Some(buffer.script()).filter(|s| *s != harfrust::script::UNKNOWN);
    let plan = plan(shaper, buffer.direction(), script, input.language, input.features);
    let glyphs = shaper.shape(buffer, ShapeOptions::new().features(input.features).plan(Some(&plan)));
    Ok(glyphs
        .glyph_infos()
        .iter()
        .zip(glyphs.glyph_positions())
        .map(|(info, pos)| RunGlyph {
            glyph_id: info.glyph_id,
            cluster: info.cluster,
            x_advance: pos.x_advance,
            y_advance: pos.y_advance,
            x_offset: pos.x_offset,
            y_offset: pos.y_offset,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use harfrust::{FontRef, ShaperData};

    #[test]
    fn synthetic_face_runs_without_installed_fonts() {
        // A tiny face with a single .notdef glyph. The shaper still processes source
        // clusters, direction, mapping, features and context without external font files.
        let mut builder = write_fonts::FontBuilder::new();
        let mut head = vec![0; 54];
        head[18..20].copy_from_slice(&1000u16.to_be_bytes());
        let mut hhea = vec![0; 36];
        hhea[34..36].copy_from_slice(&1u16.to_be_bytes());
        for (tag, bytes) in [(b"head", head), (b"hhea", hhea), (b"hmtx", vec![0, 100, 0, 0]), (b"maxp", vec![0, 0, 0x50, 0, 0, 1])] {
            builder.add_raw(write_fonts::types::Tag::new(tag), bytes);
        }
        check_run(&builder.build());
    }

    #[test]
    fn run_matches_direct_shaping_with_source_clusters_and_context() {
        let Ok(dir) = std::env::var("CRAFT_FONTS_DIR") else { return };
        let bytes = std::fs::read(std::path::Path::new(&dir).join("fonts/noto-sans-arabic/NotoSansArabic.ttf")).unwrap();
        check_run(&bytes);
    }

    fn check_run(bytes: &[u8]) {
        let font = FontRef::from_index(bytes, 0).unwrap();
        let data = ShaperData::new(&font);
        let shaper = data.shaper(&font).build();
        let overflow = RunInput {
            text: "ab",
            source_start: usize::MAX,
            pre_context: "",
            post_context: "",
            direction: Direction::LeftToRight,
            language: None,
            features: &[],
            character_clusters: false,
            uppercase: false,
        };
        assert!(shape_run(&shaper, &overflow, |c| c, |s, d, script, lang, f| Arc::new(ShapePlan::new(s, d, script, lang, f))).is_err());
        let language: Language = "ar".parse().unwrap();
        let features = [Feature::new(harfrust::Tag::new(b"kern"), 0, 19..25)];
        for direction in [Direction::LeftToRight, Direction::RightToLeft, Direction::TopToBottom] {
            let input = RunInput {
                text: "بaß1",
                source_start: 19,
                pre_context: "س",
                post_context: "م",
                direction,
                language: Some(&language),
                features: &features,
                character_clusters: true,
                uppercase: true,
            };
            let map = |c| if c == '1' { '٢' } else { c };
            let actual =
                shape_run(&shaper, &input, map, |s, d, script, lang, features| Arc::new(ShapePlan::new(s, d, script, lang, features))).unwrap();
            let mut buffer = UnicodeBuffer::new();
            for (i, c) in input.text.char_indices() {
                for upper in map(c).to_uppercase() {
                    buffer.add(upper, 19 + i as u32);
                }
            }
            buffer.set_direction(direction);
            buffer.guess_segment_properties();
            buffer.set_cluster_level(BufferClusterLevel::Characters);
            buffer.set_language(language.clone());
            buffer.set_pre_context(input.pre_context);
            buffer.set_post_context(input.post_context);
            let script = Some(buffer.script()).filter(|s| *s != harfrust::script::UNKNOWN);
            let plan = ShapePlan::new(&shaper, direction, script, Some(&language), &features);
            let expected = shaper.shape(buffer, ShapeOptions::new().features(&features).plan(Some(&plan)));
            let tuples: Vec<_> = expected
                .glyph_infos()
                .iter()
                .zip(expected.glyph_positions())
                .map(|(i, p)| (i.glyph_id, i.cluster, p.x_advance, p.y_advance, p.x_offset, p.y_offset))
                .collect();
            assert_eq!(actual.iter().map(|g| (g.glyph_id, g.cluster, g.x_advance, g.y_advance, g.x_offset, g.y_offset)).collect::<Vec<_>>(), tuples);
            assert!(!actual.is_empty());
        }
    }
}
