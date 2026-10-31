//! Optional content (layers).
//!
//! An optional content group (OCG) is a named layer that a viewer can show or hide. Create
//! groups on the [`Document`] with [`Document::add_optional_content_group`], then mark
//! content on any surface with [`Surface::start_optional_content`] and
//! [`Surface::end_optional_content`].
//!
//! Every group is written once as `<< /Type /OCG /Name (…) >>`, and the catalog gets
//! `/OCProperties` listing all groups, also those that no page uses, with the hidden ones
//! in the default configuration's `/OFF` array. Marked content is written as
//! `/OC /ocN BDC … EMC`, with `/ocN` registered in the `/Properties` resources of the
//! stream that holds it.
//!
//! Optional content marks nest independently of tagged sections, but they must not overlap
//! them: a tagged section started inside a mark ends before the mark does, and a mark started
//! inside a tagged section ends before the section does. Marks are balanced per stream: a mark
//! started before [`Surface::push_group`] (or any other push that starts a new stream, such as
//! an opacity or a mask) can't be ended inside it.
//!
//! Optional content needs PDF 1.5 and is forbidden in PDF/A-1.
//!
//! [`Surface::push_group`]: crate::surface::Surface::push_group

use std::collections::HashMap;

use pdf_writer::types::PageMode;
use pdf_writer::writers::{self, Catalog};
use pdf_writer::{Dict, Finish, Name, Ref, TextStr};

use crate::configure::validate::VersionedFeature;
use crate::configure::ValidationError;
use crate::content::ContentBuilder;
use crate::document::Document;
use crate::error::{KrillaError, KrillaResult};
use crate::resource::{Resource, ResourceDictionaryBuilder, ResourceMapper};
use crate::serialize::SerializeContext;
use crate::surface::{Location, Surface};
use crate::util::NameExt;

/// An optional content group of a [`Document`], returned by
/// [`Document::add_optional_content_group`].
///
/// An id is only meaningful for the document that created it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OcgId(usize);

struct Group {
    ref_: Ref,
    visible: bool,
}

/// The document's optional content groups and settings.
#[derive(Default)]
pub(crate) struct OptionalContent {
    groups: Vec<Group>,
    page_mode: bool,
    /// Where the first unbalanced or overlapping mark was found;
    /// [`SerializeContext::finish`] fails with [`KrillaError::UnbalancedOptionalContent`].
    pub(crate) misuse: Option<Option<Location>>,
}

impl OptionalContent {
    fn group_ref(&self, id: OcgId) -> Option<Ref> {
        self.groups.get(id.0).map(|g| g.ref_)
    }

    fn record_misuse(&mut self, location: Option<Location>) {
        self.misuse.get_or_insert(location);
    }

    /// Writes `/OCProperties`, and `/PageMode /UseOC` when requested, if any group exists.
    pub(crate) fn write_catalog_entries(
        &self,
        catalog: &mut Catalog<'_>,
        remapper: &HashMap<Ref, Ref>,
    ) {
        if self.groups.is_empty() {
            return;
        }

        let all: Vec<Ref> = self
            .groups
            .iter()
            .filter_map(|g| remapper.get(&g.ref_).copied())
            .collect();
        let hidden: Vec<Ref> = self
            .groups
            .iter()
            .filter(|g| !g.visible)
            .filter_map(|g| remapper.get(&g.ref_).copied())
            .collect();

        let mut properties = catalog.insert(Name(b"OCProperties")).dict();
        properties
            .insert(Name(b"OCGs"))
            .array()
            .items(all.iter().copied());
        let mut config = properties.insert(Name(b"D")).dict();
        config.pair(Name(b"Name"), TextStr("Layers"));
        config
            .insert(Name(b"Order"))
            .array()
            .items(all.iter().copied());
        if !hidden.is_empty() {
            config.insert(Name(b"OFF")).array().items(hidden);
        }
        config.finish();
        properties.finish();

        if self.page_mode {
            catalog.page_mode(PageMode::UseOC);
        }
    }
}

impl Document {
    /// Add an optional content group (a layer) named `name`, shown when the document opens
    /// if `visible` is true.
    ///
    /// The group is listed in the catalog's `/OCProperties` even if no content uses it.
    /// Names are text strings, so any Unicode name works; they need not be unique.
    ///
    /// Optional content needs PDF 1.5 and is forbidden in PDF/A-1: adding a group registers
    /// [`ValidationError::OptionalContent`] and, below PDF 1.5,
    /// [`ValidationError::RequiresNewerPdfVersion`].
    pub fn add_optional_content_group(&mut self, name: impl Into<String>, visible: bool) -> OcgId {
        let sc = &mut self.serializer_context;
        let name = name.into();

        sc.register_validation_error(ValidationError::OptionalContent(sc.location));
        if sc.serialize_settings().pdf_version()
            < VersionedFeature::OptionalContent.minimum_pdf_version()
        {
            sc.register_validation_error(ValidationError::RequiresNewerPdfVersion(
                VersionedFeature::OptionalContent,
                sc.location,
            ));
        }

        let ref_ = sc.new_ref();
        let mut ocg = self
            .chunk_container
            .non_stream
            .optional_content
            .indirect(ref_)
            .dict();
        ocg.pair(Name(b"Type"), Name(b"OCG"));
        ocg.pair(Name(b"Name"), TextStr(&name));
        ocg.finish();

        let groups = &mut sc.optional_content.groups;
        groups.push(Group { ref_, visible });
        OcgId(groups.len() - 1)
    }

    /// Whether the document opens with the viewer's layers panel showing
    /// (`/PageMode /UseOC`). Only written when the document has optional content groups.
    /// Off by default.
    pub fn set_optional_content_page_mode(&mut self, use_oc: bool) {
        self.serializer_context.optional_content.page_mode = use_oc;
    }
}

impl Surface<'_> {
    /// Start content that belongs to the optional content group `group`, until the matching
    /// [`Surface::end_optional_content`].
    ///
    /// Marks nest, and a mark may contain tagged sections (see the [module
    /// documentation](crate::optional_content) for the nesting rules). Content left open
    /// when the surface finishes, or when a group, opacity or mask pushed after the start
    /// is popped, is closed there, and [`Document::finish`] then fails with
    /// [`KrillaError::UnbalancedOptionalContent`].
    ///
    /// Returns [`KrillaError::UnknownOptionalContentGroup`] and writes nothing if `group`
    /// does not belong to this document.
    pub fn start_optional_content(&mut self, group: OcgId) -> KrillaResult<()> {
        let (builder, sc) = self.content_and_context();
        let Some(ocg) = sc.optional_content.group_ref(group) else {
            return Err(KrillaError::UnknownOptionalContentGroup(sc.location));
        };

        let name = builder.rd_builder.register_resource(Properties(ocg));
        builder
            .content
            .begin_marked_content_with_properties(Name(b"OC"))
            .properties_named(name.to_pdf_name());
        let tagged = builder.active_marked_content;
        builder.optional_content.open.push(tagged);
        Ok(())
    }

    /// End the innermost optional content started with [`Surface::start_optional_content`]
    /// in the current stream.
    ///
    /// Returns [`KrillaError::UnbalancedOptionalContent`] and writes nothing if no optional
    /// content is open in the current stream, or if a tagged section started inside it is
    /// still open.
    pub fn end_optional_content(&mut self) -> KrillaResult<()> {
        let (builder, sc) = self.content_and_context();
        let tagged = builder.active_marked_content;
        if builder.optional_content.open.last() != Some(&tagged) {
            return Err(KrillaError::UnbalancedOptionalContent(sc.location));
        }

        builder.optional_content.open.pop();
        builder.content.end_marked_content();
        Ok(())
    }
}

/// The optional content marks open in one content stream. Each entry records whether a
/// tagged section was open when the mark started.
#[derive(Default)]
pub(crate) struct OpenMarks {
    open: Vec<bool>,
    overlap: bool,
}

impl OpenMarks {
    /// Called when a tagged section ends: a mark started inside it would be closed by its
    /// `EMC`.
    pub(crate) fn tag_ended(&mut self) {
        if self.open.last() == Some(&true) {
            self.overlap = true;
        }
    }
}

/// Close the marks still open in `builder` before its stream is finished, and record the
/// misuse for [`SerializeContext::finish`].
pub(crate) fn close_open_marks(builder: &mut ContentBuilder, sc: &mut SerializeContext) {
    let open = std::mem::take(&mut builder.optional_content.open);
    for _ in &open {
        builder.content.end_marked_content();
    }
    if !open.is_empty() || builder.optional_content.overlap {
        let location = sc.location;
        sc.optional_content.record_misuse(location);
    }
}

/// An entry of a resource dictionary's `/Properties`: an optional content group.
#[derive(Debug, Eq, PartialEq, Hash, Clone)]
pub(crate) struct Properties(Ref);

impl Resource for Properties {
    fn new(ref_: Ref) -> Self {
        Self(ref_)
    }

    fn get_ref(&self) -> Ref {
        self.0
    }

    fn get_dict<'a>(resources: &'a mut writers::Resources) -> Dict<'a> {
        resources.insert(Name(b"Properties")).dict()
    }

    fn get_prefix() -> &'static str {
        "oc"
    }

    fn get_mapper(b: &mut ResourceDictionaryBuilder) -> &mut ResourceMapper<Properties> {
        &mut b.properties
    }
}
