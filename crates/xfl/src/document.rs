//! Neutral views of the native XFL document schema.
//!
//! This layer recognizes the document, stage, scene, layer and keyframe fields defined by XFL.
//! It does not construct a consumer's animation model. More detailed payloads remain available
//! through [`Document::root`] for the application adapter.
use crate::shape::XflElement;
use craft_xml::{Doctype, El, Error as XmlError, Limits, Local, Options, Pat, UnknownEntity};
use std::borrow::Cow;

/// Maximum bytes of XML in one XFL document or library item.
pub const MAX_XML_BYTES: usize = 256 * 1024 * 1024;
/// Maximum depth accepted by AniCraft's existing XFL XML adapter (root depth is one).
pub const MAX_XML_DEPTH: usize = 257;
/// Maximum XFL document elements accepted by the existing adapter.
pub const MAX_XML_ELEMENTS: usize = 4_000_000;

/// An XFL `DOMDocument.xml` tree plus typed, zero-copy views over its native structure.
#[derive(Clone, Debug)]
pub struct Document {
    root: El,
}

/// Stage metadata stored on the root `DOMDocument` element.
#[derive(Clone, Copy, Debug)]
pub struct Stage<'a> {
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub frame_rate: Option<f32>,
    pub background_color: Option<&'a str>,
}

/// The script-language version and document class selected by an XFL `PublishSettings.xml`.
/// Values stay in their native string form so each consumer can apply its own domain rules.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PublishSettings<'a> {
    pub action_script_version: Option<Cow<'a, str>>,
    pub document_class: Option<Cow<'a, str>>,
}

/// Bounds used when projecting native schema fields into a compact model.
#[derive(Clone, Copy, Debug)]
pub struct ModelLimits {
    pub max_timelines: usize,
    pub max_layers_per_timeline: usize,
}

/// Compact XFL metadata that does not retain or duplicate the parsed XML tree.
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentModel {
    pub stage: StageModel,
    pub timelines: Vec<TimelineModel>,
    /// Present for a library `DOMSymbolItem`; absent for normal `DOMDocument` files.
    pub symbol_timeline: Option<TimelineModel>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StageModel {
    pub width: Option<f32>,
    pub height: Option<f32>,
    pub frame_rate: Option<f32>,
    pub has_background_color: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct TimelineModel {
    pub layers: Vec<LayerModel>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LayerType {
    #[default]
    Normal,
    Folder,
    Guide,
    Mask,
    Other,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct LayerModel {
    pub layer_type: LayerType,
    pub parent_layer_index: Option<u32>,
    pub visible: Option<bool>,
    pub locked: Option<bool>,
    pub outline: Option<bool>,
    pub height_multiplier: Option<u32>,
    pub frames: Vec<FrameModel>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TweenType {
    Motion,
    Shape,
    MotionObject,
    #[default]
    Other,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FrameModel {
    pub index: Option<u32>,
    pub duration: Option<u32>,
    pub is_comment: bool,
    pub tween_type: TweenType,
}

/// A timeline (`DOMTimeline`) listed under `DOMDocument/timelines`.
#[derive(Debug)]
pub struct Timeline<'a, E: XflElement = El> {
    element: &'a E,
}

impl<E: XflElement> Copy for Timeline<'_, E> {}

impl<E: XflElement> Clone for Timeline<'_, E> {
    fn clone(&self) -> Self {
        *self
    }
}

/// A native `DOMLayer` element.
#[derive(Debug)]
pub struct Layer<'a, E: XflElement = El> {
    element: &'a E,
}

impl<E: XflElement> Copy for Layer<'_, E> {}

impl<E: XflElement> Clone for Layer<'_, E> {
    fn clone(&self) -> Self {
        *self
    }
}

/// A native `DOMFrame` element.
#[derive(Debug)]
pub struct Frame<'a, E: XflElement = El> {
    element: &'a E,
}

/// An `Include` entry in the document's `<symbols>` library list.
#[derive(Debug)]
pub struct SymbolInclude<'a, E: XflElement = El> {
    element: &'a E,
}

/// A folder entry in the document's `<folders>` library list.
#[derive(Debug)]
pub struct LibraryFolder<'a, E: XflElement = El> {
    element: &'a E,
}

/// Native metadata stored on a library `DOMSymbolItem`.
#[derive(Debug)]
pub struct SymbolItem<'a, E: XflElement = El> {
    element: &'a E,
}

impl<E: XflElement> Copy for Frame<'_, E> {}

impl<E: XflElement> Clone for Frame<'_, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<E: XflElement> Copy for SymbolInclude<'_, E> {}

impl<E: XflElement> Clone for SymbolInclude<'_, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<E: XflElement> Copy for LibraryFolder<'_, E> {}

impl<E: XflElement> Clone for LibraryFolder<'_, E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<E: XflElement> Copy for SymbolItem<'_, E> {}

impl<E: XflElement> Clone for SymbolItem<'_, E> {
    fn clone(&self) -> Self {
        *self
    }
}

fn attr_last<'a>(element: &'a impl XflElement, name: &str) -> Option<&'a str> {
    element.attr(name)
}

fn number_f32(element: &impl XflElement, name: &str) -> Option<f32> {
    attr_last(element, name).and_then(|value| value.trim().parse::<f32>().ok()).filter(|value| value.is_finite())
}

fn number_u32(element: &impl XflElement, name: &str) -> Option<u32> {
    attr_last(element, name).and_then(|value| value.trim().parse::<u32>().ok())
}

fn boolean(element: &impl XflElement, name: &str) -> Option<bool> {
    attr_last(element, name).map(|value| value.trim() == "true")
}

fn trimmed_text<'a>(element: &'a El) -> Option<Cow<'a, str>> {
    match element.text() {
        Cow::Borrowed(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then_some(Cow::Borrowed(trimmed))
        }
        Cow::Owned(text) => {
            let trimmed = text.trim();
            (!trimmed.is_empty()).then(|| Cow::Owned(trimmed.to_string()))
        }
    }
}

impl Document {
    /// Parse XFL XML with AniCraft's existing size, depth, element, attribute, entity, doctype
    /// and malformed-end-tag behavior. The shared XML parser owns byte parsing; this crate owns
    /// the XFL-specific limits and schema views.
    pub fn parse(bytes: &[u8]) -> Result<Self, XmlError> {
        let defaults = Limits::DEFAULT;
        let limits = Limits {
            max_input: MAX_XML_BYTES,
            max_depth: MAX_XML_DEPTH,
            max_nodes: MAX_XML_ELEMENTS.saturating_mul(2).saturating_add(1),
            max_elements: MAX_XML_ELEMENTS,
            max_attrs: defaults.max_attrs,
            max_attrs_total: defaults.max_attrs_total,
        };
        let options =
            Options { limits, strict_end_tags: true, unknown_entity: UnknownEntity::Literal, doctype: Doctype::Ignore, ..Options::default() };
        Ok(Self { root: options.parse(bytes)? })
    }

    /// The lossless XML tree used by consumer adapters for schema details this view does not
    /// interpret. Text and child order come directly from `craft-xml`.
    pub fn root(&self) -> &El {
        &self.root
    }

    /// The root's local element name (normally `DOMDocument`).
    pub fn root_name(&self) -> &str {
        &self.root.name
    }

    /// Reads the current publish profile, falling back to the first profile and then the root.
    /// This is also used for `PublishSettings.xml` files, whose schema is separate from the main
    /// `DOMDocument` tree.
    pub fn publish_settings(&self) -> PublishSettings<'_> {
        let profile = self
            .root
            .elements()
            .find(|element| Local("flash_profile").matches(&element.name) && boolean(*element, "current") == Some(true))
            .or_else(|| self.root.elements().find(|element| Local("flash_profile").matches(&element.name)))
            .unwrap_or(&self.root);
        let properties = profile.child(Local("PublishFlashProperties")).unwrap_or(profile);
        let value = |name: &str| properties.child(Local(name)).and_then(trimmed_text);
        PublishSettings { action_script_version: value("ActionScriptVersion"), document_class: value("DocumentClass") }
    }

    /// The XFL stage metadata, applying finite-number checks but leaving product defaults and
    /// clamping to the consumer.
    pub fn stage(&self) -> Stage<'_> {
        Stage {
            width: number_f32(&self.root, "width"),
            height: number_f32(&self.root, "height"),
            frame_rate: number_f32(&self.root, "frameRate"),
            background_color: attr_last(&self.root, "backgroundColor"),
        }
    }

    /// Timelines in document order. Non-`DOMTimeline` extension nodes are skipped.
    pub fn timelines(&self) -> impl Iterator<Item = Timeline<'_>> {
        self.root
            .child(Local("timelines"))
            .into_iter()
            .flat_map(El::elements)
            .filter(|element| Local("DOMTimeline").matches(&element.name))
            .map(|element| Timeline { element })
    }

    /// The timeline nested under `DOMSymbolItem/timeline`, if this is library symbol XML.
    pub fn symbol_timeline(&self) -> Option<Timeline<'_>> {
        self.root.child(Local("timeline"))?.child(Local("DOMTimeline")).map(|element| Timeline { element })
    }

    /// Projects bounded schema metadata and returns the XML tree by ownership. Consumers can
    /// convert that tree into their compatibility representation without keeping two full trees
    /// alive during import.
    pub fn into_parts(self, limits: ModelLimits) -> (DocumentModel, El) {
        let stage = StageModel {
            width: number_f32(&self.root, "width"),
            height: number_f32(&self.root, "height"),
            frame_rate: number_f32(&self.root, "frameRate"),
            has_background_color: attr_last(&self.root, "backgroundColor").is_some(),
        };
        let timelines = self.timelines().take(limits.max_timelines).map(|timeline| timeline.into_model(limits.max_layers_per_timeline)).collect();
        let symbol_timeline = self.symbol_timeline().map(|timeline| timeline.into_model(limits.max_layers_per_timeline));
        (DocumentModel { stage, timelines, symbol_timeline }, self.root)
    }
}

impl<'a, E: XflElement> Timeline<'a, E> {
    /// Creates a borrowed XFL timeline view over a consumer's compatible XML element.
    pub fn new(element: &'a E) -> Self {
        Self { element }
    }

    pub fn name(self) -> Option<&'a str> {
        attr_last(self.element, "name")
    }

    /// Raw timeline XML for application-specific payload conversion.
    pub fn element(self) -> &'a E {
        self.element
    }

    /// Layers in native order. Non-`DOMLayer` extension nodes are skipped.
    pub fn layers(self) -> impl Iterator<Item = Layer<'a, E>> {
        self.element
            .child("layers")
            .into_iter()
            .flat_map(|element| element.elements())
            .filter(|element| element.local_name() == "DOMLayer")
            .map(|element| Layer { element })
    }

    fn into_model(self, max_layers: usize) -> TimelineModel {
        TimelineModel { layers: self.layers().take(max_layers).map(Layer::into_model).collect() }
    }
}

impl<'a, E: XflElement> Layer<'a, E> {
    /// Creates a borrowed XFL layer view over a consumer's compatible XML element.
    pub fn new(element: &'a E) -> Self {
        Self { element }
    }

    pub fn name(self) -> Option<&'a str> {
        attr_last(self.element, "name")
    }

    pub fn layer_type(self) -> Option<&'a str> {
        attr_last(self.element, "layerType")
    }

    pub fn parent_layer_index(self) -> Option<u32> {
        number_u32(self.element, "parentLayerIndex")
    }

    pub fn visible(self) -> Option<bool> {
        boolean(self.element, "visible")
    }

    pub fn locked(self) -> Option<bool> {
        boolean(self.element, "locked")
    }

    pub fn outline(self) -> Option<bool> {
        boolean(self.element, "outline")
    }

    pub fn color(self) -> Option<&'a str> {
        attr_last(self.element, "color")
    }

    pub fn height_multiplier(self) -> Option<u32> {
        number_u32(self.element, "heightMultiplier")
    }

    /// Frames in native order. Non-`DOMFrame` extension nodes are skipped.
    pub fn element(self) -> &'a E {
        self.element
    }

    pub fn frames(self) -> impl Iterator<Item = Frame<'a, E>> {
        self.element
            .child("frames")
            .into_iter()
            .flat_map(|element| element.elements())
            .filter(|element| element.local_name() == "DOMFrame")
            .map(|element| Frame { element })
    }

    fn into_model(self) -> LayerModel {
        let layer_type = match attr_last(self.element, "layerType") {
            Some("folder") => LayerType::Folder,
            Some("guide") => LayerType::Guide,
            Some("mask") => LayerType::Mask,
            Some("normal") | None => LayerType::Normal,
            Some(_) => LayerType::Other,
        };
        LayerModel {
            layer_type,
            parent_layer_index: number_u32(self.element, "parentLayerIndex"),
            visible: boolean(self.element, "visible"),
            locked: boolean(self.element, "locked"),
            outline: boolean(self.element, "outline"),
            height_multiplier: number_u32(self.element, "heightMultiplier"),
            frames: self.frames().map(Frame::into_model).collect(),
        }
    }
}

impl<'a, E: XflElement> Frame<'a, E> {
    /// Creates a borrowed XFL frame view over a consumer's compatible XML element.
    pub fn new(element: &'a E) -> Self {
        Self { element }
    }

    pub fn index(self) -> Option<u32> {
        number_u32(self.element, "index")
    }

    pub fn duration(self) -> Option<u32> {
        number_u32(self.element, "duration")
    }

    pub fn label_type(self) -> Option<&'a str> {
        attr_last(self.element, "labelType")
    }

    pub fn name(self) -> Option<&'a str> {
        attr_last(self.element, "name")
    }

    pub fn tween_type(self) -> Option<&'a str> {
        attr_last(self.element, "tweenType")
    }

    /// Raw frame XML for application-specific shape, action, sound and tween conversion.
    pub fn element(self) -> &'a E {
        self.element
    }

    fn into_model(self) -> FrameModel {
        let tween_type = match attr_last(self.element, "tweenType") {
            Some("motion") => TweenType::Motion,
            Some("shape") => TweenType::Shape,
            Some("motion object") => TweenType::MotionObject,
            _ => TweenType::Other,
        };
        FrameModel {
            index: number_u32(self.element, "index"),
            duration: number_u32(self.element, "duration"),
            is_comment: attr_last(self.element, "labelType").is_some_and(|value| value.trim() == "comment"),
            tween_type,
        }
    }
}

impl<'a, E: XflElement> SymbolInclude<'a, E> {
    /// Creates a borrowed view over a native `Include` element.
    pub fn new(element: &'a E) -> Self {
        Self { element }
    }

    /// The package-relative library XML path from `href`, kept in native string form.
    pub fn href(self) -> Option<&'a str> {
        attr_last(self.element, "href")
    }

    /// The source element for application-specific path resolution.
    pub fn element(self) -> &'a E {
        self.element
    }
}

impl<'a, E: XflElement> LibraryFolder<'a, E> {
    /// Creates a borrowed view over a native `DOMFolderItem` element.
    pub fn new(element: &'a E) -> Self {
        Self { element }
    }

    /// The folder path from `name`, kept in native string form.
    pub fn name(self) -> Option<&'a str> {
        attr_last(self.element, "name")
    }

    /// The source element for application-specific path validation.
    pub fn element(self) -> &'a E {
        self.element
    }
}

impl<'a, E: XflElement> SymbolItem<'a, E> {
    /// Creates a borrowed view over a native `DOMSymbolItem` element.
    pub fn new(element: &'a E) -> Self {
        Self { element }
    }

    pub fn name(self) -> Option<&'a str> {
        attr_last(self.element, "name")
    }

    pub fn symbol_type(self) -> Option<&'a str> {
        attr_last(self.element, "symbolType")
    }

    pub fn linkage_export_for_action_script(self) -> Option<&'a str> {
        attr_last(self.element, "linkageExportForAS")
    }

    pub fn linkage_class_name(self) -> Option<&'a str> {
        attr_last(self.element, "linkageClassName")
    }

    pub fn linkage_identifier(self) -> Option<&'a str> {
        attr_last(self.element, "linkageIdentifier")
    }

    pub fn linkage_base_class(self) -> Option<&'a str> {
        attr_last(self.element, "linkageBaseClass")
    }

    /// Whether the symbol declares any native 9-slice scale-grid attribute, even if malformed.
    pub fn has_scale_grid(self) -> bool {
        ["scaleGridLeft", "scaleGridRight", "scaleGridTop", "scaleGridBottom"].iter().any(|name| attr_last(self.element, name).is_some())
    }

    pub fn scale_grid_left(self) -> Option<f32> {
        number_f32(self.element, "scaleGridLeft")
    }

    pub fn scale_grid_right(self) -> Option<f32> {
        number_f32(self.element, "scaleGridRight")
    }

    pub fn scale_grid_top(self) -> Option<f32> {
        number_f32(self.element, "scaleGridTop")
    }

    pub fn scale_grid_bottom(self) -> Option<f32> {
        number_f32(self.element, "scaleGridBottom")
    }

    /// The symbol's optional timeline wrapper, for the consumer to map into its own model.
    pub fn timeline(self) -> Option<&'a E> {
        self.element.child("timeline")
    }

    /// The source element for application-specific symbol validation and mapping.
    pub fn element(self) -> &'a E {
        self.element
    }
}

/// Symbol library entries in source order, skipping non-`Include` extension nodes.
pub fn symbol_includes<'a, E: XflElement>(root: &'a E) -> impl Iterator<Item = SymbolInclude<'a, E>> + 'a {
    root.child("symbols").into_iter().flat_map(XflElement::elements).filter(|element| element.local_name() == "Include").map(SymbolInclude::new)
}

/// Library folders in source order, skipping non-`DOMFolderItem` extension nodes.
pub fn library_folders<'a, E: XflElement>(root: &'a E) -> impl Iterator<Item = LibraryFolder<'a, E>> + 'a {
    root.child("folders").into_iter().flat_map(XflElement::elements).filter(|element| element.local_name() == "DOMFolderItem").map(LibraryFolder::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_stage_scene_layer_and_frame_schema_without_building_an_app_model() {
        let bytes = br##"<!DOCTYPE DOMDocument [<!ENTITY custom "kept">]>
<DOMDocument width="640" height="480" frameRate="29.97" backgroundColor="#123456">
  <timelines>
    <Extension/>
    <DOMTimeline name="Scene &custom;"><layers>
      <Extension/>
      <DOMLayer name="Background" layerType="normal" visible="false" locked="true" outline="true" color="#abcdef" heightMultiplier="2" parentLayerIndex="0">
        <frames><DOMFrame index="4" duration="3" name="Intro" labelType="name" tweenType="motion"><elements><Shape/></elements></DOMFrame></frames>
      </DOMLayer>
    </layers></DOMTimeline>
  </timelines>
</DOMDocument>"##;
        let document = Document::parse(bytes).unwrap();
        assert_eq!(document.root_name(), "DOMDocument");
        let stage = document.stage();
        assert_eq!((stage.width, stage.height, stage.frame_rate), (Some(640.0), Some(480.0), Some(29.97)));
        assert_eq!(stage.background_color, Some("#123456"));
        let mut timelines = document.timelines();
        let timeline = timelines.next().unwrap();
        assert_eq!(timeline.name(), Some("Scene &custom;"), "declared entities stay literal");
        assert!(timelines.next().is_none());
        let mut layers = timeline.layers();
        let layer = layers.next().unwrap();
        assert_eq!(layer.name(), Some("Background"));
        assert_eq!(layer.layer_type(), Some("normal"));
        assert_eq!(layer.parent_layer_index(), Some(0));
        assert_eq!((layer.visible(), layer.locked(), layer.outline()), (Some(false), Some(true), Some(true)));
        assert_eq!(layer.color(), Some("#abcdef"));
        assert_eq!(layer.height_multiplier(), Some(2));
        let frame = layer.frames().next().unwrap();
        assert_eq!((frame.index(), frame.duration()), (Some(4), Some(3)));
        assert_eq!((frame.name(), frame.label_type(), frame.tween_type()), (Some("Intro"), Some("name"), Some("motion")));
        assert_eq!(frame.element().child("elements").unwrap().child("Shape").unwrap().name, "Shape");
    }

    #[test]
    fn borrowed_timeline_views_accept_compatible_element_adapters() {
        #[derive(Debug)]
        struct AppElement {
            name: String,
            attributes: Vec<(String, String)>,
            children: Vec<AppElement>,
        }

        impl XflElement for AppElement {
            type Elements<'a> = std::slice::Iter<'a, Self>;

            fn local_name(&self) -> &str {
                self.name.rsplit(':').next().unwrap_or(&self.name)
            }

            fn attr(&self, name: &str) -> Option<&str> {
                self.attributes.iter().rev().find(|(key, _)| key == name).map(|(_, value)| value.as_str())
            }

            fn elements(&self) -> Self::Elements<'_> {
                self.children.iter()
            }

            fn text_content(&self) -> Cow<'_, str> {
                Cow::Borrowed("")
            }
        }

        fn element(name: &str, attributes: &[(&str, &str)], children: Vec<AppElement>) -> AppElement {
            AppElement {
                name: name.to_string(),
                attributes: attributes.iter().map(|(key, value)| ((*key).to_string(), (*value).to_string())).collect(),
                children,
            }
        }

        let root = element(
            "x:DOMDocument",
            &[],
            vec![element(
                "x:timelines",
                &[],
                vec![element(
                    "x:DOMTimeline",
                    &[("name", "Old"), ("name", "Scene")],
                    vec![element(
                        "x:layers",
                        &[],
                        vec![element(
                            "x:DOMLayer",
                            &[("name", "Old"), ("name", "Layer"), ("color", "#abcdef"), ("visible", "false")],
                            vec![element(
                                "x:frames",
                                &[],
                                vec![element(
                                    "x:DOMFrame",
                                    &[
                                        ("index", "999"),
                                        ("index", "3"),
                                        ("duration", "2"),
                                        ("name", "Intro"),
                                        ("labelType", "name"),
                                        ("tweenType", "motion"),
                                    ],
                                    vec![],
                                )],
                            )],
                        )],
                    )],
                )],
            )],
        );
        let timeline_element = root.child("timelines").unwrap().child("DOMTimeline").unwrap();
        let timeline = Timeline::new(timeline_element);
        assert_eq!(timeline.name(), Some("Scene"));
        let layer = timeline.layers().next().unwrap();
        assert_eq!(layer.name(), Some("Layer"));
        assert_eq!(layer.color(), Some("#abcdef"));
        assert_eq!(layer.visible(), Some(false));
        let frame = layer.frames().next().unwrap();
        assert_eq!((frame.index(), frame.duration()), (Some(3), Some(2)));
        assert_eq!((frame.name(), frame.label_type(), frame.tween_type()), (Some("Intro"), Some("name"), Some("motion")));
    }

    #[test]
    fn uses_last_duplicate_attribute_like_the_legacy_xfl_adapter() {
        let document = Document::parse(br##"<DOMDocument width="1" width="2"><timelines/></DOMDocument>"##).unwrap();
        assert_eq!(document.stage().width, Some(2.0));
    }

    #[test]
    fn preserves_lenient_doctype_entities_and_strict_end_tag_behavior() {
        let document = Document::parse(br##"<!DOCTYPE DOMDocument><DOMDocument width="&custom;"/>tail"##).unwrap();
        assert_eq!(document.stage().width, None);
        assert!(Document::parse(b"<DOMDocument><timelines></DOMDocument>").is_err());
    }

    #[test]
    fn applies_the_existing_xfl_depth_and_attribute_budgets() {
        let too_deep = format!("{}{}", "<x>".repeat(MAX_XML_DEPTH + 1), "</x>".repeat(MAX_XML_DEPTH + 1));
        assert!(matches!(Document::parse(too_deep.as_bytes()), Err(XmlError::TooDeep(_))));

        let mut too_many_attrs = String::from("<DOMDocument");
        for _ in 0..=Limits::DEFAULT.max_attrs {
            too_many_attrs.push_str(" x=''");
        }
        too_many_attrs.push_str("/>");
        assert!(matches!(Document::parse(too_many_attrs.as_bytes()), Err(XmlError::TooManyAttrs(_))));
    }

    #[test]
    fn projects_neutral_schema_fields_and_returns_the_xml_tree_by_move() {
        let document = Document::parse(
            br##"<DOMDocument width="640" backgroundColor="#fff"><timelines><DOMTimeline name="Scene"><layers><DOMLayer name="Layer" layerType="mask" visible="false"><frames><DOMFrame index="7" duration="2" labelType="comment" tweenType="shape"/></frames></DOMLayer></layers></DOMTimeline></timelines></DOMDocument>"##,
        )
        .unwrap();
        let (model, root) = document.into_parts(ModelLimits { max_timelines: 1, max_layers_per_timeline: 1 });
        assert_eq!(model.stage, StageModel { width: Some(640.0), height: None, frame_rate: None, has_background_color: true });
        assert_eq!(model.timelines.len(), 1);
        let layer = &model.timelines[0].layers[0];
        assert_eq!(layer.layer_type, LayerType::Mask);
        assert_eq!(layer.visible, Some(false));
        assert_eq!(layer.frames[0], FrameModel { index: Some(7), duration: Some(2), is_comment: true, tween_type: TweenType::Shape });
        assert_eq!(root.attr("width"), Some("640"));
        assert_eq!(root.child("timelines").unwrap().child("DOMTimeline").unwrap().attr("name"), Some("Scene"));
    }

    #[test]
    fn projects_symbol_timeline_under_the_native_symbol_wrapper() {
        let document = Document::parse(b"<DOMSymbolItem><timeline><DOMTimeline><layers><DOMLayer><frames><DOMFrame index='1'/></frames></DOMLayer></layers></DOMTimeline></timeline></DOMSymbolItem>").unwrap();
        let (model, _) = document.into_parts(ModelLimits { max_timelines: 10, max_layers_per_timeline: 10 });
        assert!(model.timelines.is_empty());
        assert_eq!(model.symbol_timeline.unwrap().layers[0].frames[0].index, Some(1));
    }

    #[test]
    fn recognizes_prefixed_xfl_timeline_elements_by_local_name() {
        let document = Document::parse(
            b"<x:DOMDocument xmlns:x='urn:adobe:xfl'><x:timelines><x:DOMTimeline><x:layers><x:DOMLayer><x:frames><x:DOMFrame index='4' duration='2'/></x:frames></x:DOMLayer></x:layers></x:DOMTimeline></x:timelines></x:DOMDocument>",
        )
        .unwrap();
        {
            let mut timelines = document.timelines();
            let timeline = timelines.next().unwrap();
            assert!(timelines.next().is_none());
            let mut layers = timeline.layers();
            let layer = layers.next().unwrap();
            assert!(layers.next().is_none());
            let frame = layer.frames().next().unwrap();
            assert_eq!((frame.index(), frame.duration()), (Some(4), Some(2)));
        }
        let (model, _) = document.into_parts(ModelLimits { max_timelines: 1, max_layers_per_timeline: 1 });
        assert_eq!(model.timelines[0].layers[0].frames[0].index, Some(4));

        let symbol =
            Document::parse(b"<x:DOMSymbolItem xmlns:x='urn:adobe:xfl'><x:timeline><x:DOMTimeline/></x:timeline></x:DOMSymbolItem>").unwrap();
        assert!(symbol.symbol_timeline().is_some());
    }

    #[test]
    fn reads_prefixed_publish_settings_children_by_local_name() {
        let document = Document::parse(
            b"<x:PublishSettings xmlns:x='urn:adobe:xfl'><x:flash_profile current='true'><x:PublishFlashProperties><x:ActionScriptVersion>3</x:ActionScriptVersion><x:DocumentClass>com.example.Main</x:DocumentClass></x:PublishFlashProperties></x:flash_profile></x:PublishSettings>",
        )
        .unwrap();
        let settings = document.publish_settings();
        assert_eq!(settings.action_script_version.as_deref(), Some("3"));
        assert_eq!(settings.document_class.as_deref(), Some("com.example.Main"));
    }

    #[test]
    fn selects_current_publish_profile_and_uses_native_value_strings() {
        let document = Document::parse(
            br#"<PublishSettings><flash_profile><PublishFlashProperties><ActionScriptVersion>2</ActionScriptVersion><DocumentClass>OldClass</DocumentClass></PublishFlashProperties></flash_profile><flash_profile current="true"><PublishFlashProperties><ActionScriptVersion> 3 </ActionScriptVersion><DocumentClass>  com.example.Main  </DocumentClass></PublishFlashProperties></flash_profile></PublishSettings>"#,
        )
        .unwrap();
        let settings = document.publish_settings();
        assert_eq!(settings.action_script_version.as_deref(), Some("3"));
        assert_eq!(settings.document_class.as_deref(), Some("com.example.Main"));
    }

    #[test]
    fn publish_settings_fall_back_to_first_profile_then_root() {
        let first = Document::parse(
            br#"<PublishSettings><flash_profile><PublishFlashProperties><ActionScriptVersion>2</ActionScriptVersion></PublishFlashProperties></flash_profile><flash_profile><PublishFlashProperties><ActionScriptVersion>3</ActionScriptVersion></PublishFlashProperties></flash_profile></PublishSettings>"#,
        )
        .unwrap();
        assert_eq!(first.publish_settings().action_script_version.as_deref(), Some("2"));

        let root = Document::parse(
            br#"<PublishSettings><ActionScriptVersion> 3 </ActionScriptVersion><DocumentClass>RootClass</DocumentClass></PublishSettings>"#,
        )
        .unwrap();
        let settings = root.publish_settings();
        assert_eq!(settings.action_script_version.as_deref(), Some("3"));
        assert_eq!(settings.document_class.as_deref(), Some("RootClass"));
    }

    #[test]
    fn reads_symbol_library_entries_and_native_symbol_metadata() {
        let document = Document::parse(
            br#"<DOMDocument><symbols><Extension/><Include href="LIBRARY/Shape.xml"/><Include href="LIBRARY/Button.xml"/></symbols><folders><Extension/><DOMFolderItem name="Characters/Hero"/></folders></DOMDocument>"#,
        )
        .unwrap();
        let includes: Vec<_> = symbol_includes(document.root()).map(SymbolInclude::href).collect();
        assert_eq!(includes, [Some("LIBRARY/Shape.xml"), Some("LIBRARY/Button.xml")]);
        let include = symbol_includes(document.root()).next().unwrap();
        assert_eq!(include.element().local_name(), "Include");
        let folders: Vec<_> = library_folders(document.root()).map(LibraryFolder::name).collect();
        assert_eq!(folders, [Some("Characters/Hero")]);
        let folder = library_folders(document.root()).next().unwrap();
        assert_eq!(folder.element().local_name(), "DOMFolderItem");

        let symbol = Document::parse(
            br#"<x:DOMSymbolItem xmlns:x="urn:adobe:xfl" name="Characters/Hero" name="Characters/Final" symbolType="graphic" linkageExportForAS="true" linkageClassName="example.Hero" linkageIdentifier="legacy.Hero" linkageBaseClass="flash.display.MovieClip" scaleGridLeft="10" scaleGridRight="90" scaleGridTop="5" scaleGridBottom="75"><x:timeline><x:DOMTimeline/></x:timeline></x:DOMSymbolItem>"#,
        )
        .unwrap();
        let item = SymbolItem::new(symbol.root());
        assert_eq!(item.element().local_name(), "DOMSymbolItem");
        assert_eq!(item.name(), Some("Characters/Final"), "the last duplicate attribute wins");
        assert_eq!(item.symbol_type(), Some("graphic"));
        assert_eq!(item.linkage_export_for_action_script(), Some("true"));
        assert_eq!(item.linkage_class_name(), Some("example.Hero"));
        assert_eq!(item.linkage_identifier(), Some("legacy.Hero"));
        assert_eq!(item.linkage_base_class(), Some("flash.display.MovieClip"));
        assert_eq!(
            (item.scale_grid_left(), item.scale_grid_right(), item.scale_grid_top(), item.scale_grid_bottom()),
            (Some(10.0), Some(90.0), Some(5.0), Some(75.0))
        );
        assert!(item.has_scale_grid());
        assert!(item.timeline().is_some());

        let malformed = Document::parse(br#"<DOMSymbolItem scaleGridLeft="not-a-number"/>"#).unwrap();
        let malformed = SymbolItem::new(malformed.root());
        assert!(malformed.has_scale_grid(), "attribute presence remains observable when its value is malformed");
        assert_eq!(malformed.scale_grid_left(), None);

        let absent = Document::parse(b"<DOMSymbolItem/>").unwrap();
        assert!(!SymbolItem::new(absent.root()).has_scale_grid());
    }
}
