//! [`AssbinCamera`]: the cameras block at the end of the scene (requirement CM-04), after the animation, texture and
//! light lists (each a `u32` count and that many blocks).
//!
//! A camera block (Observed on the one corpus project whose mesh has a camera, version 8): name, position (vec3),
//! viewing direction (vec3, Assimp's `mLookAt`), up direction (vec3), horizontal field of view (radians), two clip
//! distances, the aspect ratio (width / height), then three floats we do not use (36, 0, 0 there). The reference app stores the
//! camera already in its normalised mesh space, position equal to the scene camera of `/editor/camera.ini`; it also
//! divided the two directions by the normalisation radius, so they are not unit length (we normalise them). The
//! node of the same name, which places the camera in upstream Assimp, is not applied.

use super::RECORD;
use super::block::{read_block, read_count, read_string};
use super::scene::{SceneBlocks, scene_bytes};
use crate::container::bytes::ByteReader;
use crate::decode::DecodeError;

/// Smallest block of any trailing list: its size field and one `u32`.
const MIN_ITEM_LEN: usize = 12;

/// One camera of the scene, in the file's normalised mesh space.
#[derive(Debug, Clone, PartialEq)]
pub struct AssbinCamera {
    pub name: String,
    pub position: [f32; 3],
    /// Viewing direction (not unit length).
    pub look_at: [f32; 3],
    /// Up direction (not unit length).
    pub up: [f32; 3],
    /// Horizontal field of view, radians.
    pub horizontal_fov: f32,
    /// Width / height; 0 when the file has none.
    pub aspect: f32,
}

impl AssbinCamera {
    /// Every camera of a whole `scene.assbin`. A scene that ends before its cameras block has none.
    pub fn read_all(data: &[u8]) -> Result<Vec<AssbinCamera>, DecodeError> {
        let scene = scene_bytes(data)?;
        let blocks = SceneBlocks::split(&scene)?;
        let mut r = ByteReader::new(blocks.tail);
        if r.remaining() == 0 {
            return Ok(Vec::new());
        }
        // animations, textures, lights
        for _ in 0..3 {
            for _ in 0..read_count(&mut r, MIN_ITEM_LEN)? {
                read_block(&mut r)?;
            }
        }
        let count = read_count(&mut r, MIN_ITEM_LEN)?;
        (0..count).map(|_| AssbinCamera::parse(read_block(&mut r)?)).collect()
    }

    fn parse(block: &[u8]) -> Result<AssbinCamera, DecodeError> {
        let mut r = ByteReader::new(block);
        let name = read_string(&mut r)?;
        let mut f = [0.0f32; 13];
        for v in &mut f {
            *v = r.f32_le().map_err(DecodeError::truncated(RECORD))?;
        }
        let [px, py, pz, lx, ly, lz, ux, uy, uz, horizontal_fov, _clip_a, _clip_b, aspect] = f;
        Ok(AssbinCamera { name, position: [px, py, pz], look_at: [lx, ly, lz], up: [ux, uy, uz], horizontal_fov, aspect })
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_writer::{Buf, IDENTITY, header, node, scene_with_tail};
    use super::*;

    fn camera_block(name: &str) -> Buf {
        let mut b = Buf::default();
        b.string(name).f32s(&[3.9, 2.5, 3.6, -0.34, -0.23, -0.32, -0.17, 0.47, -0.16, 0.69, 10000.0, 10.0, 1.78, 36.0, 0.0, 0.0]);
        b
    }

    fn file(tail: Buf) -> Vec<u8> {
        let mut out = header(7, false);
        out.0.extend_from_slice(&scene_with_tail(node("RootNode", IDENTITY, &[], Vec::new()), Vec::new(), &["m"], tail).0);
        out.0
    }

    #[test]
    fn reads_the_cameras_after_the_other_lists() {
        let mut tail = Buf::default();
        tail.u32(0).u32(0).u32(1).block(Buf(vec![0; 8])).u32(2).block(camera_block("Camera")).block(camera_block("Side"));
        let cams = AssbinCamera::read_all(&file(tail)).unwrap();
        assert_eq!(cams.len(), 2);
        let c = &cams[0];
        assert_eq!((c.name.as_str(), c.position, c.horizontal_fov, c.aspect), ("Camera", [3.9, 2.5, 3.6], 0.69, 1.78));
        assert_eq!((c.look_at, c.up), ([-0.34, -0.23, -0.32], [-0.17, 0.47, -0.16]));
        assert_eq!(cams[1].name, "Side");
    }

    #[test]
    fn scenes_without_cameras_and_broken_blocks() {
        let mut empty = Buf::default();
        empty.zeros(16);
        assert!(AssbinCamera::read_all(&file(empty)).unwrap().is_empty());
        assert!(AssbinCamera::read_all(&file(Buf::default())).unwrap().is_empty(), "a scene ending after its materials");
        let mut short = Buf::default();
        short.u32(0).u32(0).u32(0).u32(1).block(Buf(vec![1, 0, 0, 0, b'C']));
        assert!(AssbinCamera::read_all(&file(short)).is_err());
        let mut lying = Buf::default();
        lying.u32(0).u32(0).u32(0).u32(1000);
        assert!(AssbinCamera::read_all(&file(lying)).is_err());
    }
}
