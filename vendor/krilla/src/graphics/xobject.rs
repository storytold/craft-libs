use std::ops::DerefMut;
use std::sync::Arc;

use pdf_writer::writers::Group;
use pdf_writer::{Finish, Name, Ref};

use crate::chunk_container::ChunkContainer;
use crate::configure::ValidationError;
use crate::geom::Rect;
use crate::graphics::blend::GroupOptions;
use crate::graphics::color::{rgb, DEVICE_CMYK, DEVICE_RGB};
use crate::resource;
use crate::resource::{Resource, Resourceable};
use crate::serialize::{Cacheable, GroupColorSpace, MaybeDeviceColorSpace, SerializeContext};
use crate::stream::{FilterStreamBuilder, Stream};
use crate::util::{NameExt, Prehashed};

/// What a group XObject is used for, which decides its `/CS` under
/// [`GroupColorSpace::Cmyk`].
#[derive(Debug, Hash, Eq, PartialEq, Clone, Copy)]
pub(crate) enum GroupRole {
    /// Drawn in a page or another form: inherits the parent's blending space.
    Content,
    /// The `/G` of a soft mask: has no parent, so it names its blending space.
    MaskSource,
}

#[derive(Debug, Hash, Eq, PartialEq)]
struct Repr {
    stream: Stream,
    isolated: bool,
    knockout: bool,
    transparency_group_color_space: bool,
    custom_bbox: Option<Rect>,
    role: GroupRole,
}

#[derive(Debug, Hash, Clone, Eq, PartialEq)]
pub(crate) struct XObject(Arc<Prehashed<Repr>>);

impl XObject {
    pub(crate) fn new(
        stream: Stream,
        isolated: bool,
        transparency_group_color_space: bool,
        custom_bbox: Option<Rect>,
    ) -> Self {
        Self::with_role(
            stream,
            isolated,
            transparency_group_color_space,
            custom_bbox,
            GroupRole::Content,
        )
    }

    /// A content group with the given group attributes.
    pub(crate) fn group(stream: Stream, options: GroupOptions) -> Self {
        Self::build(
            stream,
            options.isolated,
            options.knockout,
            false,
            None,
            GroupRole::Content,
        )
    }

    pub(crate) fn with_role(
        stream: Stream,
        isolated: bool,
        transparency_group_color_space: bool,
        custom_bbox: Option<Rect>,
        role: GroupRole,
    ) -> Self {
        Self::build(
            stream,
            isolated,
            false,
            transparency_group_color_space,
            custom_bbox,
            role,
        )
    }

    fn build(
        stream: Stream,
        isolated: bool,
        knockout: bool,
        mut transparency_group_color_space: bool,
        custom_bbox: Option<Rect>,
        role: GroupRole,
    ) -> Self {
        // In case a mask was invoked in the content stream, we _always_ create
        // a new transparency group. Please see <https://github.com/typst/typst/issues/5509>.
        // Just to provide a brief explanation: I have not found any mention
        // of a transparency group being required when using a soft mask in the
        // PDF spec. However, Apple Preview (and only them!) have a weird bug
        // where if you transform an XObject with a soft mask but not transparency
        // group, the transform gets applied twice to the mask, resulting in
        // rendering issues. Using a transparency group in this case seems to
        // fix the issue.
        // TODO: Apply group transparency to page as well.
        transparency_group_color_space |= stream.uses_mask;

        Self(Arc::new(Prehashed::new(Repr {
            stream,
            isolated,
            knockout,
            transparency_group_color_space,
            custom_bbox,
            role,
        })))
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.0.stream.is_empty()
    }

    pub(crate) fn bbox(&self) -> Rect {
        self.0.custom_bbox.unwrap_or(self.0.stream.bbox)
    }
}

impl Cacheable for XObject {
    fn serialize(
        self,
        sc: &mut SerializeContext,
        chunk_container: &mut ChunkContainer,
        root_ref: Ref,
    ) {
        let mut chunk = sc.new_chunk();

        for validation_error in &self.0.stream.validation_errors {
            sc.register_validation_error(validation_error.clone());
        }

        let use_transparency_group =
            self.0.isolated || self.0.knockout || self.0.transparency_group_color_space;

        if use_transparency_group {
            sc.register_validation_error(ValidationError::Transparency(sc.location));
        }

        if use_transparency_group || sc.uses_transparency(&self.0.stream.resource_dictionary) {
            sc.mark_transparent(root_ref);
        }

        let serialize_settings = sc.serialize_settings();

        // "Ordinarily, the CS entry may be present only for isolated transparency groups (those
        // for which I is true), and even then it is optional. However, this entry shall be present in
        // the group attributes dictionary for any transparency group XObject that has no parent
        // group or page from which to inherit."
        //
        // So while we don't technically always need to write it, let's just play it save
        // and always add a group color space in case we have a transparency group.
        //
        // Under `GroupColorSpace::Cmyk`, content groups inherit the CMYK page group instead.
        let transparency_group_cs = if !use_transparency_group {
            None
        } else {
            match (serialize_settings.group_color_space, self.0.role) {
                (GroupColorSpace::Rgb, _) => Some(sc.register_colorspace(
                    chunk_container,
                    rgb::color_space(serialize_settings.no_device_cs).into(),
                )),
                (GroupColorSpace::Cmyk, GroupRole::Content) => None,
                (GroupColorSpace::Cmyk, GroupRole::MaskSource) => {
                    Some(sc.mask_group_color_space(chunk_container))
                }
            }
        };

        let x_object_stream = FilterStreamBuilder::new_from_content_stream(
            &self.0.stream.content,
            &serialize_settings,
        )
        .finish(&serialize_settings);
        let mut x_object = chunk.form_xobject(root_ref, x_object_stream.encoded_data());
        x_object_stream.write_filters(x_object.deref_mut().deref_mut());

        self.0.stream.resource_dictionary.to_pdf_resources(
            &mut x_object,
            sc,
            &mut chunk_container.non_stream.resource_dictionaries,
        );
        x_object.bbox(
            self.0
                .custom_bbox
                .unwrap_or(self.0.stream.bbox)
                .to_pdf_rect(),
        );

        if use_transparency_group {
            let mut group = x_object.group();
            let transparency = group.transparency();

            if self.0.isolated {
                transparency.isolated(self.0.isolated);
            }

            if self.0.knockout {
                transparency.knockout(true);
            }

            if let Some(transparency_group_cs) = transparency_group_cs {
                write_group_color_space(transparency, transparency_group_cs);
            }

            transparency.finish();
            group.finish();
        }

        x_object.finish();
        chunk_container.streams.x_objects.push(chunk);
    }
}

/// Write the `/CS` of a transparency group: DeviceRGB, DeviceCMYK, sRGB or the CMYK profile.
pub(crate) fn write_group_color_space(group: &mut Group<'_>, cs: MaybeDeviceColorSpace) {
    let pdf_cs = group.insert(Name(b"CS"));

    match cs {
        MaybeDeviceColorSpace::DeviceRgb => pdf_cs.primitive(DEVICE_RGB.to_pdf_name()),
        MaybeDeviceColorSpace::DeviceCMYK => pdf_cs.primitive(DEVICE_CMYK.to_pdf_name()),
        MaybeDeviceColorSpace::ColorSpace(cs) => pdf_cs.primitive(cs.get_ref()),
        _ => unreachable!(),
    }
}

impl Resourceable for XObject {
    type Resource = resource::XObject;
}
