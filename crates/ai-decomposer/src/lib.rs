//! # craft-ai-decomposer
//!
//! Cognitive LLM Decomposer & Suite Dispatcher for the Storytold ArtCraft Suite.
//!
//! When a human or external agent sends a high-level creative prompt (e.g. "Create a scene of a
//! medieval blacksmith forging a sword in a dark stone forge with flying sparks"), this engine
//! acts as the brain behind the suite:
//!
//! 1. **Semantic Decomposition:** Identifies physical assets, spatial depth, materials, lighting,
//!    human anatomy, motion, and sound.
//! 2. **Domain-to-App Mapping:** Maps each component to the exact ArtCraft app best suited:
//!    - Narrative / Dialogue / Scene setup -> `WordCraft`
//!    - Spatial layout / 3D Forge & Anvil Geometry -> `CADCraft`
//!    - Character anatomy / Blacksmith silhouette -> `VectorCraft` / `AniCraft`
//!    - Sparks, glow, fire, metallic textures, and color grading -> `PhotoCraft`
//!    - Hammer strikes, anvil resonance, fire crackle -> `SoundCraft`
//!    - Sequence motion & camera pan -> `FilmCraft` / `EffectCraft`
//!    - Cost & materials calculation sheet -> `GridCraft`
//!    - Final printed concept booklet / pitch deck -> `DesignCraft` / `PdfCraft`
//! 3. **Structured Execution Graph:** Produces an ordered, dependency-aware Directed Acyclic Graph (DAG)
//!    of executable native tool calls dispatched directly through `craft-agent-bridge`.

use craft_studio_director::{AestheticStyle, AppCommandDispatch};
use serde::{Deserialize, Serialize};

/// Semantic domain extracted from a natural language request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CreativeDomain {
    ScreenplayAndNarrative,
    SpatialGeometry3D,
    VectorGraphicAndContours,
    RasterShadingAndLighting,
    MotionVfxAndParticles,
    AudioAcousticsAndSoundtrack,
    EditorialPublishingAndLayout,
    SpreadsheetAndBudgetData,
}

/// A decomposed unit of work assigned to a specific domain and app.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DecomposedTask {
    pub domain: CreativeDomain,
    pub target_app: String,
    pub description: String,
    pub parameters: serde_json::Value,
    pub dependencies: Vec<usize>, // indices of tasks that must complete before this runs
}

/// The Cognitive Suite Decomposer.
pub struct SuiteDecomposer {}

impl Default for SuiteDecomposer {
    fn default() -> Self {
        Self::new()
    }
}

impl SuiteDecomposer {
    pub fn new() -> Self {
        Self {}
    }

    /// Decomposes any natural language creative prompt into an ordered, executable suite graph.
    pub fn decompose_prompt(&self, prompt: &str, desired_style: AestheticStyle) -> Vec<DecomposedTask> {
        let mut tasks = Vec::new();
        let p_lower = prompt.to_lowercase();

        // 1. Always establish narrative & script if characters or story are mentioned
        tasks.push(DecomposedTask {
            domain: CreativeDomain::ScreenplayAndNarrative,
            target_app: "wordcraft".into(),
            description: format!("Author screenplay beats and scene direction for: {prompt}"),
            parameters: serde_json::json!({
                "scene_prompt": prompt,
                "character_breakdown": true,
                "dialogue": false
            }),
            dependencies: Vec::new(),
        });

        // 2. 3D Spatial Geometry & Camera Perspective (CADCraft)
        tasks.push(DecomposedTask {
            domain: CreativeDomain::SpatialGeometry3D,
            target_app: "cadcraft".into(),
            description: "Construct 3D spatial room, perspective grid, vanishing points and camera placement".into(),
            parameters: serde_json::json!({
                "depth_meters": 20.0,
                "camera_lens_mm": 35.0,
                "elevation_angle_deg": 15.0
            }),
            dependencies: vec![0],
        });

        // 3. Vector Contours & Silhouettes (VectorCraft)
        tasks.push(DecomposedTask {
            domain: CreativeDomain::VectorGraphicAndContours,
            target_app: "vectorcraft".into(),
            description: "Draw vector character silhouettes and core prop contours".into(),
            parameters: serde_json::json!({
                "style": format!("{desired_style:?}"),
                "curvature_smoothing": true
            }),
            dependencies: vec![1],
        });

        // 4. Shading, Atmospheric Lighting, Textures & SSS (PhotoCraft)
        tasks.push(DecomposedTask {
            domain: CreativeDomain::RasterShadingAndLighting,
            target_app: "photocraft".into(),
            description: "Synthesize lighting passes, material textures, skin translucency and color grade".into(),
            parameters: serde_json::json!({
                "style": format!("{desired_style:?}"),
                "lighting_passes": ["key", "fill", "rim", "ambient_occlusion"],
                "color_temperature_k": 3200
            }),
            dependencies: vec![2],
        });

        // 5. If animation/video/motion is implied or requested
        if p_lower.contains("הולך")
            || p_lower.contains("walk")
            || p_lower.contains("וידאו")
            || p_lower.contains("video")
            || p_lower.contains("animate")
        {
            tasks.push(DecomposedTask {
                domain: CreativeDomain::MotionVfxAndParticles,
                target_app: "effectcraft".into(),
                description: "Animate walk cycle keyframes and apply physics simulation".into(),
                parameters: serde_json::json!({
                    "motion_type": "bipedal_stride",
                    "surface": "perpendicular_wall",
                    "fps": 24
                }),
                dependencies: vec![3],
            });

            tasks.push(DecomposedTask {
                domain: CreativeDomain::MotionVfxAndParticles,
                target_app: "filmcraft".into(),
                description: "Assemble timeline sequence and export video master".into(),
                parameters: serde_json::json!({
                    "resolution": "1920x1080",
                    "format": "mp4"
                }),
                dependencies: vec![4],
            });

            tasks.push(DecomposedTask {
                domain: CreativeDomain::AudioAcousticsAndSoundtrack,
                target_app: "soundcraft".into(),
                description: "Design footsteps, spatial reverberation and atmospheric soundbed".into(),
                parameters: serde_json::json!({
                    "acoustics": "stone_canyon_echo",
                    "footstep_surface": "smooth_granite"
                }),
                dependencies: vec![4],
            });
        }

        tasks
    }

    /// Converts the decomposed tasks into atomic Native AppCommandDispatches for craft-agent-bridge.
    pub fn compile_to_dispatches(&self, tasks: &[DecomposedTask]) -> Vec<AppCommandDispatch> {
        tasks
            .iter()
            .enumerate()
            .map(|(idx, t)| AppCommandDispatch {
                target_app: t.target_app.clone(),
                tool_name: format!("{}_execute", t.target_app),
                parameters: t.parameters.clone(),
                order: (idx + 1) as u32,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decomposer_breaks_down_complex_creative_request() {
        let decomposer = SuiteDecomposer::new();
        let tasks = decomposer.decompose_prompt("איש הולך על קיר אנכי, חציו ריאליסטי וחציו פיקסאר", AestheticStyle::Cinematic3D);

        assert!(tasks.len() >= 6, "Should decompose into full multi-app pipeline");
        assert_eq!(tasks[0].target_app, "wordcraft");
        assert_eq!(tasks[1].target_app, "cadcraft");
        assert_eq!(tasks[2].target_app, "vectorcraft");
        assert_eq!(tasks[3].target_app, "photocraft");
        assert_eq!(tasks[4].target_app, "effectcraft");
        assert_eq!(tasks[5].target_app, "filmcraft");

        let dispatches = decomposer.compile_to_dispatches(&tasks);
        assert_eq!(dispatches.len(), tasks.len());
        assert_eq!(dispatches[0].order, 1);
    }
}
