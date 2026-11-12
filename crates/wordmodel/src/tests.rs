use super::*;
use std::sync::Arc;

#[test]
fn new_document_is_valid() {
    let d = Document::new();
    assert_eq!(d.body.len(), 1);
    assert_eq!(d.para_paths(StoryRef::Body).len(), 1);
    assert_eq!(d.plain_text(StoryRef::Body), "");
    assert_eq!(d.sections().len(), 1);
}

#[test]
fn word_count() {
    let d = Document::from_text("Hello, world!\nThis is — a test 42.\n\n");
    assert_eq!(d.word_count(), 7);
    assert_eq!(count_words(" -- "), 0);
}

/// #2797: the status bar's character counts, worded like the Word Count dialog's: characters
/// with spaces leave the paragraph breaks out, characters without spaces leave every
/// whitespace character out.
#[test]
fn character_counts() {
    assert_eq!(count_chars(""), 0);
    assert_eq!(count_chars("Hello"), 5);
    // Paragraph breaks join the paragraphs but aren't characters.
    assert_eq!(count_chars("Hello\nworld\r\n"), 10);
    assert_eq!(count_chars("a\tb"), 3, "a tab is a character");
    // Every kind of whitespace goes, spaces, tabs and (non-ASCII) spaces between words.
    assert_eq!(count_chars_no_spaces("Hello world"), 10);
    assert_eq!(count_chars_no_spaces("a\tb c\u{a0}d"), 4, "including a non-breaking space");
    assert_eq!(count_chars_no_spaces(" \n\r\t "), 0);
    // Accented letters and emoji count as one character each, as they are.
    assert_eq!(count_chars("café 🚀"), 6);
    assert_eq!(count_chars_no_spaces("café 🚀"), 5);
}

/// #4547: each Han ideograph and kana counts as a word; punctuation and Hangul spacing don't change.
#[test]
fn word_count_counts_each_chinese_and_japanese_character() {
    // #4547: unspaced Chinese counted as one word per run.
    assert_eq!(count_words("我们今天去公园。"), 7);
    assert_eq!(count_words("WordCraft是一个文字处理器，很好用！"), 1 + 11);
    assert_eq!(count_words("ひらがなとカタカナ"), 9);
    assert_eq!(count_words("ｶﾀｶﾅ"), 4);
    assert_eq!(count_words("ｶﾞｷﾞ"), 2, "voiced sound marks belong to their kana");
    assert_eq!(count_words("。、・「」"), 0);
    // Korean is spaced: Hangul counts by tokens.
    assert_eq!(count_words("안녕하세요 세계"), 2);
}

#[test]
fn word_count_includes_numbered_list_labels_but_not_bullets() {
    let mut d = Document::new();
    let num_id = Arc::make_mut(&mut d.numbering).add_list(ListKind::Numbered);
    let bul_id = Arc::make_mut(&mut d.numbering).add_list(ListKind::Bullet);

    let mut p1 = Paragraph::with_text("Apple", CharProps::default());
    p1.props.numbering = Some(props::NumRef { num: num_id, level: 0 });
    let mut p2 = Paragraph::with_text("Banana", CharProps::default());
    p2.props.numbering = Some(props::NumRef { num: num_id, level: 0 });

    let mut p3 = Paragraph::with_text("Cherry", CharProps::default());
    p3.props.numbering = Some(props::NumRef { num: bul_id, level: 0 });
    let mut p4 = Paragraph::with_text("Date", CharProps::default());
    p4.props.numbering = Some(props::NumRef { num: bul_id, level: 0 });

    d.body = vec![Arc::new(Block::Para(p1)), Arc::new(Block::Para(p2)), Arc::new(Block::Para(p3)), Arc::new(Block::Para(p4))];

    // Numbered items contribute 2 words each (label + text) = 4 words.
    // Bullet items contribute 1 word each (bullet marker is not a word + text) = 2 words.
    assert_eq!(d.word_count(), 6);
    let plain = d.plain_text(StoryRef::Body);
    assert_eq!(plain, "Apple\nBanana\nCherry\nDate");
}

#[test]
fn word_count_includes_numbered_labels_with_nothing_suffix() {
    let mut d = Document::new();
    let num_id = Arc::make_mut(&mut d.numbering).add_list(ListKind::Numbered);
    let aid = d.numbering.num(num_id).expect("list num exists").abstract_id;
    let a = Arc::make_mut(&mut d.numbering).abstracts.iter_mut().find(|a| a.id == aid).expect("abstract num exists");
    a.levels[0].suffix = numbering::LevelSuffix::Nothing;

    let mut p1 = Paragraph::with_text("Apple", CharProps::default());
    p1.props.numbering = Some(props::NumRef { num: num_id, level: 0 });
    let mut p2 = Paragraph::with_text("Banana Cherry", CharProps::default());
    p2.props.numbering = Some(props::NumRef { num: num_id, level: 0 });
    d.body = vec![Arc::new(Block::Para(p1)), Arc::new(Block::Para(p2))];

    // Label "1." and text "Apple" must each be counted as a separate word (2 words),
    // and label "2." and text "Banana Cherry" as 3 words (total: 5 words),
    // even though LevelSuffix::Nothing produces no visible separator between label and text.
    assert_eq!(d.word_count(), 5);
}

/// #4547: a generated CJK list label such as 第十二章 still counts as one word.
#[test]
fn word_count_keeps_a_cjk_list_label_one_word() {
    let mut d = Document::new();
    let num_id = Arc::make_mut(&mut d.numbering).add_list(ListKind::Numbered);
    let aid = d.numbering.num(num_id).expect("list num exists").abstract_id;
    let a = Arc::make_mut(&mut d.numbering).abstracts.iter_mut().find(|a| a.id == aid).expect("abstract num exists");
    a.levels[0].format = section::NumFormat::ChineseCounting;
    a.levels[0].text = "第%1章".into();
    a.levels[0].start = 12;
    let mut p = Paragraph::with_text("总结", CharProps::default());
    p.props.numbering = Some(props::NumRef { num: num_id, level: 0 });
    d.body = vec![Arc::new(Block::Para(p))];
    // The label 第十二章 stays one word (#4547 changed only document text); 总结 is two.
    assert_eq!(d.word_count(), 1 + 2);
}

#[test]
fn sections_and_section_mut() {
    let mut d = Document::from_text("a\nb\nc");
    if let Some(Block::Para(p)) = d.body.get_mut(0).map(Arc::make_mut) {
        p.section = Some(Box::new(SectionProps { landscape: true, ..Default::default() }));
    }
    let s = d.sections();
    assert_eq!(s.len(), 2);
    assert_eq!(s[0].0, 0);
    assert_eq!(d.section_index_of(0), 0);
    assert_eq!(d.section_index_of(2), 1);
    d.section_mut(0).margin_left = 10.0;
    d.section_mut(2).margin_left = 20.0;
    assert_eq!(d.sections()[0].1.margin_left, 10.0);
    assert_eq!(d.last_section.margin_left, 20.0);
}

#[test]
fn parts_and_media() {
    let mut d = Document::new();
    let id = d.add_part(PartKind::Header, Vec::new());
    assert_eq!(d.story(StoryRef::Part(id)).map(|b| b.len()), Some(1));
    let k1 = d.add_media(vec![1, 2, 3], "png");
    let k2 = d.add_media(vec![1, 2, 3], "png");
    let k3 = d.add_media(vec![4], "png");
    assert_eq!(k1, k2);
    assert_ne!(k1, k3);
}

#[test]
fn many_distinct_media_add_in_linear_time() {
    // An import with tens of thousands of small distinct images (same length, differing only at
    // the end) used to compare each against every earlier one.
    let mut d = Document::new();
    let t = std::time::Instant::now();
    let n = 40_000u32;
    for i in 0..n {
        let mut bytes = vec![7u8; 256];
        bytes.extend(i.to_le_bytes());
        d.add_media(bytes, "png");
    }
    assert_eq!(d.media.len(), n as usize);
    // Duplicates still find their first copy, including media inserted directly.
    let mut first = vec![7u8; 256];
    first.extend(0u32.to_le_bytes());
    assert_eq!(d.add_media(first, "png"), "image1.png");
    d.media.insert("direct.png".into(), Arc::new(vec![9, 9]));
    assert_eq!(d.add_media(vec![9, 9], "png"), "direct.png");
    // About half a second on a loaded machine; the old scan took over two minutes.
    assert!(t.elapsed().as_secs_f32() < 10.0, "{:?}", t.elapsed());
    // The lookup cache isn't part of the document.
    let mut e = d.clone();
    e.media_index = MediaIndex::default();
    assert_eq!(d, e);
}

#[test]
fn media_keys_skip_taken_names_once() {
    let mut d = Document::new();
    // A file whose media already use the names add_media would pick next.
    for i in 2..2002u32 {
        d.media.insert(format!("image{i}.png"), Arc::new(i.to_le_bytes().to_vec()));
    }
    assert_eq!(d.add_media(vec![0], "png"), "image2002.png");
    assert_eq!(d.add_media(vec![1], "png"), "image2003.png");
    assert_eq!(d.add_media(vec![2], "png"), "image2004.png");
}

#[test]
fn free_ids_skip_taken_ones_instead_of_overflowing() {
    assert_eq!(free_id(None, std::iter::empty(), 1), Some(1));
    assert_eq!(free_id(Some(7), [3, 7].into_iter(), 1), Some(8));
    assert_eq!(free_id(Some(0), [0].into_iter(), 1), Some(1));
    // A file used the largest id: the lowest free one, not a wrapped 0 or a taken id.
    assert_eq!(free_id(Some(u32::MAX), [1, 2, 4, u32::MAX].into_iter(), 1), Some(3));
    assert_eq!(free_id(Some(u32::MAX), [u32::MAX, 1, 0, 1].into_iter(), 0), Some(2));
    // Every id from `min` up taken: none, rather than a duplicate.
    assert_eq!(free_id(Some(u32::MAX), [u32::MAX, u32::MAX - 1, u32::MAX - 2].into_iter(), u32::MAX - 2), None);
}

#[test]
fn parts_and_lists_after_the_largest_ids_take_free_ones() {
    // A file can carry any ids, 0 and u32::MAX included; new ones must not overflow or replace them.
    let mut d = Document::new();
    for id in [0, 1, u32::MAX] {
        let blocks = vec![para_block(Paragraph::with_text(&id.to_string(), CharProps::default()))];
        d.parts.insert(id, Part { kind: PartKind::Header, blocks, ..Default::default() });
    }
    let parts = d.parts.clone();
    assert_eq!(d.try_add_part(PartKind::Comment, Vec::new()), Ok(2));
    assert_eq!(d.add_part(PartKind::Footer, Vec::new()), 3);
    assert!(parts.iter().all(|(k, p)| d.parts.get(k) == Some(p)));

    let n = Arc::make_mut(&mut d.numbering);
    n.add_list(ListKind::Bullet);
    n.add_list(ListKind::Bullet);
    n.abstracts[1].id = u32::MAX;
    n.nums[1].id = u32::MAX;
    n.nums[1].abstract_id = u32::MAX;
    let lists = n.clone();
    let num = n.try_add_list(ListKind::Numbered).unwrap();
    assert_eq!((num, n.num(num).map(|x| x.abstract_id)), (2, Some(1)));
    assert_eq!(n.abstract_of(num).map(|a| &a.levels), Some(&numbering::levels_for(ListKind::Numbered)));
    assert_eq!(n.restart(num), Some(3));
    assert_eq!(n.num(3).map(|x| x.abstract_id), Some(1));
    assert_eq!((&n.abstracts[..2], &n.nums[..2]), (&lists.abstracts[..], &lists.nums[..]));
}

#[test]
fn ensure_nonempty_repairs() {
    let mut d = Document::new();
    d.body.clear();
    d.body.push(Arc::new(Block::Table(Table::new(1, 1, 100.0))));
    d.ensure_nonempty();
    assert!(matches!(d.body.last().map(|b| &**b), Some(Block::Para(_))));
}

/// `blocks` in the only cell of `levels` tables, each nested in the next.
fn nested_tables(levels: usize, blocks: Blocks) -> Blocks {
    (0..levels).fold(blocks, |inner, _| {
        let mut t = Table::new(1, 1, 100.0);
        t.rows[0].cells[0].blocks = inner;
        vec![Arc::new(Block::Table(t))]
    })
}

/// A paragraph showing text box story `story` (drawing id `id`).
fn box_para(story: u32, id: u32) -> Blocks {
    let shape = InlineObject::Shape {
        kind: para::ShapeKind::TextBox,
        w: 50.0,
        h: 20.0,
        fill: None,
        stroke: None,
        stroke_width: 0.0,
        float: Default::default(),
        story: Some(story),
        freeform: None,
        effects: Default::default(),
        extra: connector::ShapeExtra { id, ..Default::default() },
    };
    let mut p = Paragraph::with_text("cell", CharProps::default());
    p.insert_object(0, shape, &CharProps::default()).unwrap();
    vec![para_block(p)]
}

#[test]
fn text_boxes_shown_from_deeply_nested_tables_are_kept() {
    // #1339: the paragraph walk stopped 16 tables down, so a box shown only from deeper was taken
    // for unshown and the next edit pruned its story.
    let para_of = |t: &str| vec![para_block(Paragraph::with_text(t, CharProps::default()))];
    let mut d = Document::from_text("body");
    let at_17 = d.add_part(PartKind::TextBox, para_of("17 tables down"));
    let at_40 = d.add_part(PartKind::TextBox, para_of("40 tables down"));
    let inner = d.add_part(PartKind::TextBox, para_of("deep in a box"));
    let outer = d.add_part(PartKind::TextBox, nested_tables(18, box_para(inner, 0)));
    let in_header = d.add_part(PartKind::TextBox, para_of("deep in a header"));
    let hdr = d.add_part(PartKind::Header, nested_tables(20, box_para(in_header, 0)));
    let unshown = d.add_part(PartKind::TextBox, para_of("shown nowhere"));
    d.body.extend(nested_tables(17, box_para(at_17, 0)));
    d.body.extend(nested_tables(40, box_para(at_40, 7)));
    d.body.extend(box_para(outer, 0));
    assert_eq!(d.prune_text_boxes(), 1);
    assert!(!d.parts.contains_key(&unshown));
    let kept: Vec<u32> = d.parts.keys().copied().collect();
    assert_eq!(kept, vec![at_17, at_40, inner, outer, in_header, hdr]);
    assert_eq!(d.prune_text_boxes(), 0);
    // Counted in reading order, the nested box after its own; new drawing ids above the deep one.
    let part = StoryRef::Part;
    assert_eq!(d.counted_stories(), vec![StoryRef::Body, part(at_17), part(at_40), part(outer), part(inner)]);
    assert_eq!(d.next_shape_id(), 8);
}

fn fs(instr: &str) -> InlineObject {
    InlineObject::FieldStart { instr: instr.into(), locked: false, dirty: false }
}

#[test]
fn field_ranges_pair_across_paragraphs_and_nest() {
    let mut d = Document::from_text("See .\nBib one\nBib two");
    let c = CharProps::default();
    // Citation in paragraph 0: "See ␣[cite](Smith 2020)␣." with a nested field inside.
    let p0 = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p0.insert_object(4, fs("ADDIN ZOTERO_ITEM CSL_CITATION {}"), &c).unwrap();
    let n = p0.insert_text(7, "(Smith 2020)", &c).unwrap();
    p0.insert_object(7 + n, InlineObject::FieldEnd, &c).unwrap();
    p0.insert_object(7, fs("ADDIN inner"), &c).unwrap();
    p0.insert_object(10, InlineObject::FieldEnd, &c).unwrap();
    // Bibliography from paragraph 1 to the end of paragraph 2.
    d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().insert_object(0, fs("ADDIN ZOTERO_BIBL {} CSL_BIBLIOGRAPHY"), &c).unwrap();
    let p2 = d.para_mut(StoryRef::Body, &Path::top(2)).unwrap();
    let end = p2.len();
    p2.insert_object(end, InlineObject::FieldEnd, &c).unwrap();

    let r = d.field_ranges(StoryRef::Body);
    assert_eq!(r.len(), 3);
    assert!(r[0].instr.starts_with("ADDIN ZOTERO_ITEM"));
    assert_eq!((r[0].start.off, r[0].depth), (4, 0));
    assert_eq!((r[1].instr.as_str(), r[1].depth), ("ADDIN inner", 1));
    assert_eq!(r[2].start.path, Path::top(1));
    assert_eq!(r[2].end.path, Path::top(2));
    assert!(!d.has_unbalanced_field_ranges());
    assert_eq!(d.balance_field_ranges(), 0);
    // Markers contribute no text.
    assert_eq!(d.plain_text(StoryRef::Body), "See (Smith 2020).\nBib one\nBib two");
}

#[test]
fn orphan_field_markers_are_removed() {
    let mut d = Document::from_text("ab\ncd");
    let c = CharProps::default();
    let p0 = d.para_mut(StoryRef::Body, &Path::top(0)).unwrap();
    p0.insert_object(0, InlineObject::FieldEnd, &c).unwrap();
    // OBJ is three bytes: "␣a[x]b]" → start after "␣a", end after "b".
    p0.insert_object(4, fs("ADDIN x"), &c).unwrap();
    p0.insert_object(8, InlineObject::FieldEnd, &c).unwrap();
    d.para_mut(StoryRef::Body, &Path::top(1)).unwrap().insert_object(1, fs("ADDIN open"), &c).unwrap();
    assert!(d.has_unbalanced_field_ranges());
    assert_eq!(d.balance_field_ranges(), 2);
    assert!(!d.has_unbalanced_field_ranges());
    let r = d.field_ranges(StoryRef::Body);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].instr, "ADDIN x");
    assert_eq!(d.plain_text(StoryRef::Body), "ab\ncd");
}

#[test]
fn custom_props_set_get_remove() {
    let mut d = Document::new();
    assert_eq!(d.custom_prop("ZOTERO_PREF_1"), None);
    d.set_custom_prop("ZOTERO_PREF_1", "<data/>");
    d.set_custom_prop("Other", "x");
    d.set_custom_prop("zotero_pref_1", "<data2/>");
    assert_eq!(d.custom_props.len(), 2);
    assert_eq!(d.custom_props[0].name, "ZOTERO_PREF_1");
    assert_eq!(d.custom_prop("Zotero_Pref_1"), Some("<data2/>"));
    assert!(d.remove_custom_prop("other"));
    assert!(!d.remove_custom_prop("other"));
    assert_eq!(d.custom_props.len(), 1);
}

/// A paragraph that is nothing but `objs` (built directly: inserting one at a time is slow too).
fn para_of_objects(objs: Vec<InlineObject>) -> Paragraph {
    let mut p = Paragraph::new();
    p.text = para::OBJ.to_string().repeat(objs.len());
    p.runs = vec![Run { len: p.text.len(), props: CharProps::default() }];
    p.objects = objs;
    p
}

#[test]
fn object_walks_are_linear_in_the_paragraph() {
    // #3614: loops of object_offsets + object_at recounted the objects before each one, so a
    // paragraph of 40,000 markers took seconds per walk.
    let n = 40_000u32;
    let w = para::OBJ.len_utf8();
    let mut objs: Vec<InlineObject> = Vec::new();
    for i in 0..n {
        objs.push(InlineObject::BookmarkStart { name: format!("b{i}") });
        objs.push(fs("ADDIN x"));
        objs.push(InlineObject::PermStart { id: i, group: "everyone".into(), editor: String::new() });
    }
    // Ranges close in the order they opened (the worst case for a backwards search).
    for i in 0..n {
        objs.push(InlineObject::FieldEnd);
        objs.push(InlineObject::PermEnd { id: i });
    }
    let mut d = Document::new();
    d.body = vec![Arc::new(Block::Para(para_of_objects(objs)))];
    let t = std::time::Instant::now();
    assert_eq!(d.bookmarks().len(), n as usize);
    assert_eq!(d.field_ranges(StoryRef::Body).len(), n as usize);
    let perms = d.perm_ranges();
    assert_eq!(perms.len(), n as usize);
    assert!(perms.iter().all(|r| r.end.off > r.start.off), "every range closed by its own end");
    assert_eq!(d.text_box_anchor(7), None);
    // A caret before a long row of field ends moves past all of them.
    let first_end = 3 * n as usize * w;
    let p = d.past_field_ends(&Pos::body(0, first_end));
    assert_eq!(p.off, first_end + w);
    // Under 0.3 s on a loaded machine; the old walks took minutes (360 s for the first test).
    assert!(t.elapsed().as_secs_f32() < 10.0, "{:?}", t.elapsed());

    let ends: Vec<InlineObject> = (0..n).map(|_| InlineObject::FieldEnd).collect();
    d.body = vec![Arc::new(Block::Para(para_of_objects(ends)))];
    let t = std::time::Instant::now();
    assert_eq!(d.past_field_ends(&Pos::body(0, 0)).off, n as usize * w);
    // Typing at the end of a row of block-level control starts.
    let block = ContentControl { block: true, ..Default::default() };
    let starts: Vec<InlineObject> = (0..n).map(|_| InlineObject::ControlStart { control: Box::new(block.clone()) }).collect();
    d.body = vec![Arc::new(Block::Para(para_of_objects(starts)))];
    let end = Pos::body(0, n as usize * w);
    assert_eq!(d.typing_pos(&end), end);
    assert!(t.elapsed().as_secs_f32() < 10.0, "{:?}", t.elapsed());
}

#[test]
fn control_markers_kept_are_sorted() {
    // `delete_range` groups the kept markers by paragraph and binary-searches each group, so
    // `kept_control_markers` must hand them over sorted and without repeats (review of #3874).
    let start = || InlineObject::ControlStart { control: Box::default() };
    let mut d = Document::from_text("before");
    // Controls open and close across paragraphs: 1 closes the one 0 opens, 3 the one 2 opens…
    for i in 0..12 {
        let objs = if i % 2 == 0 { vec![start(), start()] } else { vec![InlineObject::ControlEnd, InlineObject::ControlEnd] };
        d.body.push(Arc::new(Block::Para(para_of_objects(objs))));
    }
    d.body.push(Arc::new(Block::Para(Paragraph::with_text("after", CharProps::default()))));
    let w = para::OBJ.len_utf8();
    for (a, b) in [(Pos::body(2, w), Pos::body(11, w)), (Pos::body(1, w), Pos::body(12, w)), (Pos::body(3, w), Pos::body(8, 0))] {
        let kept = d.kept_control_markers(&a, &b);
        assert!(!kept.is_empty(), "{a:?}..{b:?}");
        assert!(kept.windows(2).all(|p| p[0] < p[1]), "sorted, no repeats: {kept:?}");
    }
}

#[test]
fn lone_table_open_markers_linear_time() {
    let mut d = Document::new();
    let mut t = Table::new(1, 1, 100.0);
    const COUNT: usize = 2_000;
    t.controls.open = (0..COUNT).map(|i| ContentControl { tag: format!("ctrl_{i}"), ..Default::default() }).collect();
    d.body.insert(0, Arc::new(Block::Table(t)));

    let mut paras = Vec::new();
    let mut tables = Vec::new();
    let timer = std::time::Instant::now();
    crate::control::orphans(&d.body, &mut Vec::new(), 0, &mut paras, &mut tables);

    assert_eq!(paras.len(), 0);
    assert_eq!(tables.len(), COUNT);
    for (i, (path, nth, is_open)) in tables.iter().enumerate() {
        assert_eq!(*path, Path::top(0));
        assert_eq!(*nth, i);
        assert!(*is_open);
    }

    assert!(d.has_unbalanced_controls());
    let removed = d.balance_controls();
    assert_eq!(removed, COUNT);
    assert!(!d.has_unbalanced_controls());
    assert!(timer.elapsed().as_secs_f32() < 10.0, "{:?}", timer.elapsed());
    let Block::Table(tbl) = &*d.body[0] else { panic!("expected table") };
    assert!(tbl.controls.open.is_empty());
}

#[test]
fn lone_table_open_markers_mixed_tables_and_paired() {
    let mut d = Document::new();
    let mut t0 = Table::new(1, 1, 100.0);
    t0.controls.open = (0..3).map(|i| ContentControl { tag: format!("t0_{i}"), ..Default::default() }).collect();
    t0.controls.close = 1;

    let mut t1 = Table::new(1, 1, 100.0);
    t1.controls.open = (0..2).map(|i| ContentControl { tag: format!("t1_{i}"), ..Default::default() }).collect();

    d.body = vec![Arc::new(Block::Table(t0)), Arc::new(Block::Table(t1))];

    let mut paras = Vec::new();
    let mut tables = Vec::new();
    crate::control::orphans(&d.body, &mut Vec::new(), 0, &mut paras, &mut tables);

    assert_eq!(paras.len(), 0);
    assert_eq!(tables, vec![(Path::top(0), 0, true), (Path::top(0), 1, true), (Path::top(1), 0, true), (Path::top(1), 1, true),]);

    let removed = d.balance_controls();
    assert_eq!(removed, 4);
    assert!(!d.has_unbalanced_controls());

    let Block::Table(res0) = &*d.body[0] else { panic!("expected table") };
    assert_eq!(res0.controls.open.len(), 1);
    assert_eq!(res0.controls.open[0].tag, "t0_2");
    assert_eq!(res0.controls.close, 1);

    let Block::Table(res1) = &*d.body[1] else { panic!("expected table") };
    assert!(res1.controls.open.is_empty());
    assert_eq!(res1.controls.close, 0);
}

#[test]
fn missing_picture_names_are_capped_at_a_char_boundary() {
    let mut d = Document::new();
    let long: String = "é".repeat(5000);
    d.missing_pictures.push(MissingPicture { part: long.clone(), story: long, reason: MissingPictureReason::MissingPart });
    let item = &d.missing_pictures.list[0];
    assert_eq!((item.part.chars().count(), item.story.chars().count()), (1024, 1024));
}

#[test]
fn missing_pictures_are_capped_deduplicated_and_not_content() {
    let mut d = Document::new();
    let item = |n: usize| MissingPicture { part: format!("p{n}"), story: "word/document.xml".into(), reason: MissingPictureReason::MissingPart };
    d.missing_pictures.push(item(0));
    d.missing_pictures.push(item(0));
    assert_eq!(d.missing_pictures.list.len(), 1);
    for n in 1..MAX_MISSING_PICTURES + 3 {
        d.missing_pictures.push(item(n));
    }
    assert_eq!(d.missing_pictures.list.len(), MAX_MISSING_PICTURES);
    assert!(d.missing_pictures.truncated);
    let e = Document::new();
    assert_eq!(d, e);
    assert!(d.same_content(&e));
    assert!(!serde_json::to_string(&d).unwrap().contains("p0"));
    assert_eq!(MissingPictureReason::ExternalTarget.label(), "external target");
}
