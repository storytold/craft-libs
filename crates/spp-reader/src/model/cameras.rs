//! [`SavedCameras`]: the reference app's viewer cameras from `/editor/camera.ini` (object serialisation, read with the generic
//! walker). A faithful mirror; turning them into our cameras is `translate::cameras`' job.
//!
//! Layout (Observed on all 157 corpus projects, version 2019.3 … 9.0): root `CameraSettingsData` with
//! `defaultCamera` (the viewer camera as it was when the project was saved), `sceneCameras[]` (the cameras of the
//! imported mesh; one corpus project has one) and `selectedCamera` (a reference to the active one's `uid`). Each is
//! a `CameraTransformationData`: `label`, `uid`, `type` (enum: 0 perspective, 1 orthographic — Inferred from 33
//! projects whose `orthographicHeight` fits their framing), `position` (vec3) and `rotation` (quaternion
//! `[x, y, z, w]` turning the camera's −z viewing axis and +y up axis into the scene; Verified: the viewer camera's
//! −z points at the model), both in the file's normalised mesh space (the space of `scene.assbin`'s vertices),
//! `focalLength` and `filmSize` (vec2, width × height, in mm: 20.7264 × 15.5448 for the viewer camera, the scene
//! camera's own film otherwise), `orthographicHeight` (normalised units), `focusDistance`, `aperture`, `features`.

use super::RawTree;
use super::raw::RawObject;
use crate::decode::DecodeError;

/// Where the cameras live inside a project file.
pub const CAMERA_INI: &str = "/editor/camera.ini";

/// How a project camera projects (`type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraProjection {
    Perspective,
    Orthographic,
    Other(u64),
}

/// One `CameraTransformationData`.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectCamera {
    pub uid: Option<u64>,
    pub label: String,
    pub projection: CameraProjection,
    pub position: [f32; 3],
    /// Quaternion `[x, y, z, w]`.
    pub rotation: [f32; 4],
    /// Millimetres.
    pub focal_length: f32,
    /// Width and height in millimetres.
    pub film_size: [f32; 2],
    pub orthographic_height: f32,
    pub focus_distance: Option<f32>,
}

impl ProjectCamera {
    /// `None` when the pose or the lens is missing.
    fn from_object(o: &RawObject) -> Option<ProjectCamera> {
        let rotation = match o.floats("rotation")? {
            [x, y, z, w, ..] => [*x, *y, *z, *w],
            _ => return None,
        };
        let film_size = match o.floats("filmSize")? {
            [w, h, ..] => [*w, *h],
            _ => return None,
        };
        Some(ProjectCamera {
            uid: o.uid(),
            label: o.string("label").unwrap_or_default().to_string(),
            projection: match o.u64("type") {
                Some(0) | None => CameraProjection::Perspective,
                Some(1) => CameraProjection::Orthographic,
                Some(n) => CameraProjection::Other(n),
            },
            position: o.vec3("position")?,
            rotation,
            focal_length: o.f32("focalLength")?,
            film_size,
            orthographic_height: o.f32("orthographicHeight").unwrap_or(0.0),
            focus_distance: o.f32("focusDistance"),
        })
    }
}

/// The cameras of a project.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SavedCameras {
    /// The viewer camera (`defaultCamera`), if readable.
    pub viewer: Option<ProjectCamera>,
    /// The mesh's cameras (`sceneCameras`), the readable ones in order.
    pub scene: Vec<ProjectCamera>,
    /// `uid` of the active camera (`selectedCamera`).
    pub selected: Option<u64>,
}

impl SavedCameras {
    pub fn parse(data: &[u8]) -> Result<SavedCameras, DecodeError> {
        let tree = RawTree::parse(data)?;
        let Some(root) = tree.root.as_ref() else {
            return Ok(SavedCameras::default());
        };
        Ok(SavedCameras {
            viewer: root.object("defaultCamera").and_then(ProjectCamera::from_object),
            scene: root.objects("sceneCameras").filter_map(ProjectCamera::from_object).collect(),
            selected: root.u64("selectedCamera"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode::document::test_writer::DocWriter;

    const VEC2: u8 = 0x02;
    const VEC3: u8 = 0x03;
    const VEC4: u8 = 0x04;

    fn floats(w: &mut DocWriter, name: &str, tag: u8, v: &[f32]) {
        w.field(name).raw(&[tag]);
        v.iter().for_each(|f| {
            w.raw(&f.to_le_bytes());
        });
    }

    fn camera(w: &mut DocWriter, label: &str, uid: u64, kind: u64) {
        w.object(0x14, "CameraTransformationData", 8);
        floats(w, "filmSize", VEC2, &[36.0, 20.25]);
        w.field("focalLength").raw(&[0x01]).raw(&50f32.to_le_bytes());
        w.field("label").raw(&[0x10]).raw(&(label.len() as u32).to_le_bytes()).raw(label.as_bytes());
        w.field("orthographicHeight").raw(&[0x01]).raw(&2f32.to_le_bytes());
        floats(w, "position", VEC3, &[1.0, 2.0, 3.0]);
        floats(w, "rotation", VEC4, &[0.0, 0.0, 0.0, 1.0]);
        w.field("type").raw(&[0x09]).raw(&kind.to_le_bytes());
        w.field("uid").raw(&[0x0c]).raw(&uid.to_le_bytes());
        w.close();
    }

    fn file() -> Vec<u8> {
        let mut w = DocWriter::new(2);
        w.object(0x12, "CameraSettingsData", 3);
        w.field("defaultCamera");
        camera(&mut w, "Default camera", 8, 0);
        w.field("sceneCameras").list(0x11, 2);
        camera(&mut w, "Camera", 9, 1);
        // an unreadable entry (no lens) is left out
        w.object(0x14, "CameraTransformationData", 0);
        w.close();
        w.close();
        w.field("selectedCamera").raw(&[0x0f]).raw(&9u64.to_le_bytes());
        w.close();
        w.bytes
    }

    #[test]
    fn reads_viewer_and_scene_cameras() {
        let c = SavedCameras::parse(&file()).unwrap();
        let viewer = c.viewer.unwrap();
        assert_eq!((viewer.label.as_str(), viewer.uid, viewer.projection), ("Default camera", Some(8), CameraProjection::Perspective));
        assert_eq!((viewer.position, viewer.rotation, viewer.film_size, viewer.focal_length), ([1.0, 2.0, 3.0], [0.0, 0.0, 0.0, 1.0], [36.0, 20.25], 50.0));
        assert_eq!(c.scene.len(), 1);
        assert_eq!(c.scene[0].projection, CameraProjection::Orthographic);
        assert_eq!(c.scene[0].orthographic_height, 2.0);
        assert_eq!(c.selected, Some(9));
        assert!(SavedCameras::parse(&[1, 2, 3]).is_err());
    }
}
