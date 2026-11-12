# craft-xml

Bounded XML for hostile input: an owned element tree, streaming of repeated elements, a pull
reader, escaping and a writer. It replaces the small XML helper each Craft app carried.

It is built on `quick-xml`'s tokenizer (MIT). Everything above the tokens is here: limits,
references, namespaces, recovery, the tree and the writer.

```rust
use craft_xml::{Local, Options};

let root = craft_xml::parse(b"<p:sp id='5'><p:t>one &amp; two</p:t></p:sp>").unwrap();
assert_eq!(root.attr_u32("id"), Some(5));
assert_eq!(root.child(Local("t")).map(|t| t.deep_text()).as_deref(), Some("one & two"));

// Rows go to the callback one at a time; the tree keeps the rest.
let part = b"<sheetData><row r='1'/><row r='2'/></sheetData>";
let mut rows = 0;
let sheet = Options::local().parse_streaming(part, &["row"], |row| {
    rows += row.attr_u32("r").unwrap_or(0);
    Ok::<(), craft_xml::Error>(())
});
assert_eq!((rows, sheet.map(|s| s.children.len())), (3, Ok(0)));
```

## Reading

`parse(bytes)` reads with `Options::default()`. `Options` is a plain struct; set what differs.

| Option | Default | Meaning |
| --- | --- | --- |
| `limits` | see below | What the reader refuses to exceed. |
| `names` | `AsWritten` | `AsWritten` keeps `p:sp`. `Local` keeps `sp`. `Canonical(&c)` rewrites to the caller's prefix for the namespace URI. |
| `ns_decls` | `true` | Keep `xmlns` declarations as attributes. `Options::local()` and `Options::canonical(..)` turn it off. |
| `strict` | `false` | See below. `.strict()` also makes an undefined entity an error. |
| `unknown_entity` | `Drop` | `Drop`, `Literal` (keep `&name;` as text) or `Error`. |
| `doctype` | `Ignore` | `Ignore`, `RejectInternalSubset` or `Reject`. Never processed, whichever is chosen. |
| `cdata`, `pis`, `comments` | `false` | Keep these as their own nodes. CDATA content is otherwise joined to the text around it. |
| `drop_blank_text` | `false` | Drop the text of an element with child elements when all of it is whitespace (indentation). |
| `split_depth` | `2` | For streaming: how far below the root an element may be and still be handed out. |

### Limits

| Limit | Default | Error |
| --- | --- | --- |
| `max_input` | 256 MiB | `TooLarge` |
| `max_depth` | 256 levels (the root is 1) | `TooDeep` |
| `max_nodes` | 2,000,000 nodes kept in the tree: elements, text stretches, kept CDATA, instructions and comments | `TooManyNodes` |
| `max_attrs` | 1024 on one start tag, namespace declarations and malformed ones included; with canonical names, also the declarations in scope at once | `TooManyAttrs` |
| `max_attrs_total` | 4,000,000 attributes kept in the tree, over all its elements | `TooManyAttrsTotal` |

A limit exceeded is an error, never a panic and never a truncated tree. `parse_streaming_partial`
returns the error together with what had been read, for a caller that wants the part's other
children when its rows cannot all be read; that tree is marked as partial by the error beside it. Elements handed to a
streaming callback leave the node and attribute counts, so a streamed part may be of any length. Text is not
counted separately: nothing expands, so it is never more than three times the input (the worst
case is input that is not UTF-8, where each bad byte becomes U+FFFD).

### What is never done

- **No DTD processing.** A `<!DOCTYPE>` is skipped or refused; its entity and element declarations are not read.
- **No entity expansion** beyond `&amp;` `&lt;` `&gt;` `&quot;` `&apos;` and numeric character references. A reference to a declared entity is an undefined entity. Nothing external is ever fetched.
- No validation, no XPath, no XInclude, no schema.
- No encodings other than UTF-8 and UTF-16. The encoding an XML declaration names is not consulted.
- Names are not checked on reading. The writer checks them (below).

### Lenient and strict

A lenient read keeps what a damaged document had:

- an end tag closes the innermost open element, whatever its name; an end tag with nothing open is skipped;
- elements still open at the end of the input are closed (a truncated part);
- reading stops when the root element ends: nothing after it is looked at;
- a malformed attribute is skipped; bytes that are not UTF-8 become U+FFFD.

A strict read makes each of those an error, and also a second root element and text outside the
root, in the pull reader as in the tree. Markup the tokenizer cannot read (`<a b='1`, an unterminated comment) is an error either
way. A repeated attribute is kept each time in both.

### Text

- Literal CR LF and CR in text and CDATA are read as LF (XML 1.0 §2.11). `&#13;` stays a CR.
- Literal tab, LF and CR in an attribute value are read as spaces. Character references stay.
- A numeric reference to a character XML 1.0 cannot carry (`&#0;`, `&#1;`, a surrogate) is an undefined entity.
- UTF-16 is recognised by its byte-order mark, or without one by the zero bytes of its first two characters. An unpaired surrogate becomes U+FFFD, also in a strict read.

### Namespaces

`Canonical { namespaces: &[(prefix, uri)], unknown, element, attr_value }`:

- an element in a listed namespace is `prefix:local`; in an unlisted one, `unknown:local` (`?:local` by default); in none, or with a prefix nothing declares, `local`;
- a prefixed attribute likewise; an unprefixed attribute keeps its name;
- `xml:` needs no entry;
- `element` overrides the prefix by element and parent; `attr_value` rewrites a value that holds prefixes while the scope is known (`Scope::uri`).

`?` cannot be part of an XML name, so the writer leaves unknown-namespace elements and attributes out.

## The tree

`El { name, attrs: Vec<(String, String)>, children: Vec<Node> }`, `Node::{El, Text, CData, Pi, Comment}`. Fields are public.

Where the apps disagreed, there are two spellings, not one behaviour:

| Question | Spellings | Which app used which |
| --- | --- | --- |
| Is a name matched whole or by local part? | `child("w:p")` / `child(Local("p"))`. Every method that takes a name takes either: `child`, `children`, `path`, `find`, `find_all`, `child_attr`, `child_text`, `child_mut`, `is`, `first_start`. | Whole: WordCraft, GridCraft, LightCraft, AniCraft. Local: DeckCraft, DesignCraft. |
| How is an attribute looked up? | `attr` (the exact name). `attr_or_plain("w:val")` (exact, else the unprefixed `val`). `attr_local("id")` (exact, else any prefix; `attr_local("r:id")` is any *prefixed* `id`). | `attr`: GridCraft, DesignCraft, LightCraft, AniCraft. `attr_or_plain`: WordCraft. `attr_local`: DeckCraft. |
| Does `text` include descendants? | `text()` is the element's own text and CDATA. `deep_text()` adds every descendant's. | Own: WordCraft `text`, GridCraft and LightCraft `.text`. Deep: DeckCraft `text`, DesignCraft `text_content`, WordCraft `deep_text`. |
| Is a number read exactly? | `attr_u32`, `attr_i32`, `attr_i64`, `attr_f64` read exactly (`value::to_*`). `value::to_i64_lossy`, `to_i32_lossy`, `to_u32_lossy` truncate a fraction and clamp. | Exact: GridCraft, AniCraft. Lossy: DeckCraft. |

Booleans (`attr_bool`, `flag`, `value::to_bool`): `1`, `true`, `on`, `t` and `0`, `false`, `off`,
`f`, trimmed, case-sensitive. That is GridCraft's set; DeckCraft's lacks `t`/`f` and IDML only
writes `true`/`false`, so both are covered. `attr_f64` is finite values only, as in every app.

`descendants`, `find`, `find_all`, `deep_text`, `node_count` and the writer do not recurse.

`node_count()` is what `max_nodes` counted for an element. A reader of a package of many parts
keeps one budget with it: charge each tree it keeps, and read the next part with what is left as
`max_nodes`, so a part over the budget stops before its tree is built.

## Writing

- `esc`, `push_esc`: text. `&`, `<`, `>` escaped; CR written `&#13;`.
- `esc_attr`, `push_esc_attr`: a double-quoted attribute value. Also `"`, and tab, LF, CR as `&#9;` `&#10;` `&#13;`.
- Both drop characters XML 1.0 cannot carry (`is_xml_char`).
- `El::to_xml`, `write_xml`: one line. `El::to_xml_pretty(indent)`, `write_xml_pretty`: an element whose children are all elements has them one per line; any other element is on one line exactly as `to_xml` writes it.
- An element or attribute whose name is not an XML name (`is_name`) is not written, with everything inside it. A name can never inject markup.
- `Writer`: `Writer::document()` (with the `standalone="yes"` declaration), `Writer::new()` (a fragment), `Writer::pretty(indent)`; `decl`, `doctype`, `pi`; `tag(name).attr(k, v).attr_opt(k, v).open()` / `.empty()` / `.text(t)`; `open`, `empty`, `leaf` with a slice of attributes; `close()` (the writer knows the name); `text`, `cdata`, `raw`, `tree`; `finish` closes what is open.

### Round trip

Reading what `to_xml` wrote gives an equal tree. Reading a document and writing it keeps the
elements, attributes in order, text, and with the options on, CDATA sections, processing
instructions and comments. It does not keep:

- the XML declaration, the DOCTYPE, and anything outside the root element;
- quoting and spacing inside tags, and how a character was written (`&#65;` against `A`);
- `<a></a>` against `<a/>`;
- comments, processing instructions and CDATA sections as sections, unless the options keep them;
- a literal CR; literal tab, LF and CR in attribute values (they were read as spaces);
- with `to_xml_pretty`, whitespace-only text between the children of an element-only element;
- namespace declarations, when names are read as `Local` or `Canonical`.

## Migration

### WordCraft `crates/docx/src/xml.rs`

| WordCraft | craft-xml |
| --- | --- |
| `MAX_DEPTH`, `MAX_ELEMENTS`, `MAX_ATTRS` | `Limits`. `max_nodes` counts text nodes too: use twice the element count. |
| `NAMESPACES`, `OTHER_NAMESPACES`, `STRICT`, `prefix_for` | The tables stay in WordCraft, as `Canonical::namespaces`; `Canonical::prefix`. |
| `element_prefix`, `drawing_prefix` | `Canonical::element`. The rules stay in WordCraft. |
| `requirement_prefix` and the `Requires` rewrite | `Canonical::attr_value` with `Scope::uri`. |
| `parse`, `parse_ns(extra)`, `parse_strict` | `Options::canonical(&c).parse`; extra entries first in the table; `.strict()`. |
| `Node`, `El { name, attrs, kids }` | `Node`, `El { name, attrs, children }` |
| `attr` | `attr_or_plain` |
| `els`, `child`, `children`, `find`, `deep_text`, `local`, `to_xml` | `elements`, then the same names |
| `text` | `text` (a `Cow`) |
| `child_val(n)` | `child(n).and_then(\|c\| c.attr_or_plain("w:val"))` |
| `utf16_to_utf8` | `to_utf8` |
| `esc` | `esc` for text, `esc_attr` for attribute values |
| `W::new`, `open`, `empty`, `leaf`, `text`, `raw`, `into_bytes` | `Writer::document`, then the same names |
| `W::close(name)`, `W::val(name, v)`, `w.s` | `close()`, `empty(name, &[("w:val", v)])`, `as_str()` |
| `is_rel_attr`, `REL_*`, `DECORATIVE_*`, `body_ns` | Stay in WordCraft. |

### GridCraft `crates/xlsx/src/xml.rs`

| GridCraft | craft-xml |
| --- | --- |
| `MAX_DEPTH`, `MAX_XML_NODES` | `Limits` |
| `parse` | `Options { unknown_entity: Literal, ..Options::local() }.parse` |
| `parse_streaming(xml, split, cb)` | `.parse_streaming(xml, split, cb)`; `cb` returns any error that is `From<craft_xml::Error>` |
| `first_start(xml, name)` | `Options::local().first_start(xml, name)` |
| `El.children` (elements), `El.text` | `elements()`, `text()` |
| `attr`, `attr_u32`, `attr_i64`, `attr_f64`, `attr_bool`, `flag`, `child`, `path` | The same names |
| `kids(n)`, `child_val(n)` | `children(n)`, `child_attr(n, "val")` |
| `to_utf8`, `esc`, `push_esc`, `esc_attr` | The same names |
| `push_u64`, `push_a1`, `num`, `push_num`, `rich_text`, `phonetic_data`, `encode_xstring`, `decode_xstring`, `t_el`, `si_body` | Stay in GridCraft. |

### DeckCraft `crates/pptx/src/xml.rs`

| DeckCraft | craft-xml |
| --- | --- |
| `MAX_DEPTH`, `MAX_NODES` | `Limits` |
| `parse(bytes)?.root`, `Doc::ns_decls` | `craft_xml::parse(bytes)?`, `root.ns_decls()` (prefix and URI) |
| `attr(n)`, `has_attr(n)` | `attr_local(n)`, `attr_local(n).is_some()` |
| `i64(n)`, `i32(n)`, `u32(n)` | `attr_local(n).and_then(value::to_i64_lossy)` and `to_i32_lossy`, `to_u32_lossy` |
| `f64(n)`, `bool(n)` | `attr_local(n).and_then(value::to_f64)` and `to_bool` |
| `is(l)`, `child(l)`, `children_named(l)`, `path(ls)`, `find(l)` | `is(Local(l))`, `child(Local(l))`, `children(Local(l))`, `path(&[Local(..)])`, `find(Local(l))` |
| `find_all(l, &mut out)` | `out.extend(e.find_all(Local(l)))` |
| `text()` | `deep_text()` |
| `child_val(l)` | `child(Local(l)).and_then(\|c\| c.attr_local("val"))` |
| `local`, `prefix`, `set_attr`, `elements`, `elements_mut`, `to_xml`, `esc`, `esc_attr` | The same names |
| `W::new`, `W::frag`, `finish` | `Writer::document`, `Writer::new`, `into_bytes` |
| `A::new().a(k, v)`, `.o(k, opt)` | `tag(name).attr(k, v)`, `.attr_opt(k, opt)` |
| `A::b(k, opt_bool)`, `A::t(k, bool)` | `.attr_opt(k, opt.map(u8::from))`, `.attr_opt(k, flag.then_some(1))` |
| `open(name, a)`, `open0`, `empty(name, a)`, `empty0` | `tag(name)….open()` / `.empty()`, or `open(name, &[])`, `empty(name, &[])` |
| `close(name)`, `val(name, v)`, `elt(name, t)` | `close()`, `tag(name).attr("val", v).empty()`, `leaf(name, &[], t)` |

### DesignCraft `crates/idml/src/xml.rs`, `crates/pptx/src/xml.rs`, `crates/engine/src/cmd/xml.rs`

| DesignCraft | craft-xml |
| --- | --- |
| `parse` | `Options { strict: true, pis: true, ..Options::default() }.parse` |
| `Node::Pi(target, content)` | `Node::Pi { target, data }` |
| `El::new(n).attr(k, v).child(c).text(t)` | `El::new(n).with_attr(k, v).with_child(c).with_text(t)` |
| `set`, `push`, `get`, `local`, `elements` | `set_attr`, `push`, `attr`, `local`, `elements` |
| `find(l)`, `find_all(l)` (direct children) | `child(Local(l))`, `children(Local(l))` |
| `text_content` | `deep_text` |
| `write(out, depth)` | `write_xml_pretty(out, "\t")` |
| `escape_into(out, s, attr)` | `push_esc(out, s)`, `push_esc_attr(out, s)` |
| `document(root)`, `document_with_pi(root, pi)` | `Writer::pretty("\t")`, then `decl(true)`, `pi(target, data)` or `raw(pi)`, `tree(root)`, `into_bytes()` |
| `prop`, `prop_el`, `num`, `boolean` (IDML's `<Properties>` form) | Stay in DesignCraft, on `attr` and `child`. |
| pptx `esc` | `esc` for text, `esc_attr` for attribute values. `emu`, `emu_len`, `angle`, `pct`, `hex` stay. |
| cmd `esc`, `valid_name` | `esc`/`esc_attr`; `is_name` (which also accepts `:`). The rest is the app's structure commands and stays. |

### The others

| App helper | craft-xml |
| --- | --- |
| WordCraft `formats/src/odt/xml.rs`: `events`, `events_at`, `Ev`, `Attrs`, `get`, `local` | `Options::events(utf8)`, `Events::next`, `Events::span`, `Event`. `Event::End` names the start tag. An error is returned, not swallowed. `length` stays. |
| AniCraft `crates/xml`: `parse` | `Options { strict: true, drop_blank_text: true, unknown_entity: Literal, ..Options::default() }.parse` |
| AniCraft `parse_mixed`, `TEXT_NODE` | Not needed: text nodes keep their place among the children. |
| AniCraft `Element::attr`, `u32`, `child`, `f32`, `list(n)` | `attr`, `attr_u32`, `child`, `attr_f64(n) as f32`, `child(n)` then `elements()` |
| LightCraft `lensdb/src/xml.rs`: `parse`, `Node`, `attr`, `num`, `kids`, `kid`, `kid_text`, `entity`, `unescape` | `Options::parse` with its limits (`max_depth: 32`), `El`, `attr`, `attr_f64`, `children`, `child`, `child_text` (trim it), `entity`, `unescape` |
| FilmCraft `interchange/src/xml.rs`: `escape`, `escape_attr`, `XmlWriter` | `esc`, `esc_attr`, `Writer::pretty("  ")` with `decl(false)` and `doctype(name)`; `text_attrs` is `leaf` |
| FilmCraft `parse`, `child`, `children`, `elements`, `text`, `path_text`, `child_text`, `child_i64`, `child_f64` | `Options { doctype: RejectInternalSubset, .. }.parse` and the tree's `child`, `children`, `elements`, `text`, `path`, `child_text`, `value::to_i64_lossy`, `value::to_f64`. The result is an `El`, not a `roxmltree::Document`. |
| FigCraft `vectordrawable/src/xml.rs`: `esc` | `esc_attr` |
| MailCraft `dav`, `ews` readers | `Options { doctype: Reject, .. }.strict()`, with `Canonical` for the DAV and EWS namespaces |

### Behaviour that changes on migration

- **Bytes written.** `esc` no longer escapes quotes in text; WordCraft, FilmCraft, FigCraft and DesignCraft's pptx writer escaped `'` and this crate never does (values are double-quoted); DesignCraft's IDML writer wrote `&#x9;` where this writes `&#9;`, and wrote a literal CR in text where this writes `&#13;`. All read back the same, except the CR, which now survives.
- **Bytes that are not UTF-8** in text were an error in WordCraft, GridCraft and DesignCraft's lenient readers. Here a lenient read replaces them and a strict read refuses them.
- **GridCraft** kept an attribute value raw when it held an undefined entity; here the rest of the value is still resolved. It did not normalise line ends in CDATA; here they are.
- **DeckCraft** kept reading after the root element and could fail on what followed; a lenient read here stops.
- **Node counts** include text, so an element limit becomes about twice the number.
- **LightCraft** did not normalise literal tabs and line ends in attribute values to spaces; here they are.
- **FilmCraft** rounded a fractional `child_i64`; `value::to_i64_lossy` truncates. Its `child_f64` accepted `NaN` and infinities; `value::to_f64` does not.
- **AniCraft** kept attributes in a map (the last of a repeated name won); here they are a list and `attr` returns the first.

### Left to the apps

- FigCraft's `Out` layout (one attribute per line, as Android Studio formats drawables).
- EffectCraft's `xml_project.rs`: a project codec with its own reader. It writes control characters as `&#N;`, which XML 1.0 forbids and this crate does not read.
- VectorCraft's `xml_id.rs`: object names as XML IDs (`_xHH_` escapes), a naming rule and not XML syntax.
- PhotoCraft's `io/src/xml.rs` and FilmCraft's `parse` guard `roxmltree` and return its document. Code that moves to `El` gets the same guards from `Limits` and `Doctype`; code that stays on `roxmltree` keeps its guard.
- Booleans that are not `xsd:boolean`: FilmCraft's `TRUE`/`FALSE`, AniCraft's "anything but `true` is false".
- Number formatting (`num`), units, colours, and every format's own vocabulary.
