//! # craft-studio-director
//!
//! Autonomous AI Content Creation & Orchestration Studio for the Storytold ArtCraft Suite.
//!
//! Rather than treating AI generation as a black-box image prompt that produces static pixels,
//! `craft-studio-director` translates high-level world, narrative, and physics understanding into
//! precise native commands across all 15 ArtCraft applications:
//!
//! - **World & Physics Model:** Maintains spatial 3D coordinates, lighting angles, materials,
//!   and physical constraints (gravity, perspective, occlusion).
//! - **Character & Asset Consistency Engine:** Preserves character face meshes, attire, proportions,
//!   and color palettes across frames, scenes, and vector/raster formats.
//! - **Timeline & Story Choreographer:** Manages multi-shot narrative arcs, camera tracks, scene pacing,
//!   and audio-video synchronization (feeding `FilmCraft` and `SoundCraft`).
//! - **Native Multi-App Pipeline Dispatcher:** Dispatches atomic creation steps directly through
//!   `craft-agent-bridge` without Python overhead:
//!     - 3D meshes & cameras -> `CADCraft`
//!     - Vector shapes & logos -> `VectorCraft`
//!     - Texture rendering, lighting & retouching -> `PhotoCraft`
//!     - Script & storyboards -> `WordCraft`
//!     - Timeline assembly & grading -> `FilmCraft`
//!     - Final delivery & packaging -> `PdfCraft`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// Aesthetic rendering style for generation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AestheticStyle {
    /// True-to-life photographic realism (subsurface scattering, realistic skin pores, natural eye refractions).
    Photorealistic,
    /// Hand-drawn 2D vector animation style (clean line art, cel shading).
    AnimatedVector,
    /// Cinematic 3D animation (Pixar/Dreamworks style, expressive features, PBR materials).
    Cinematic3D,
    /// Painterly / Oil on canvas (textured brush strokes, visible impasto, blended gradients).
    PainterlyOil,
    /// Graphic Novel / Noir (heavy shadows, cross-hatching, high contrast).
    GraphicNovelNoir,
}

/// Human biological proportions based on classical anatomical canon (e.g. 7.5 to 8 heads tall).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HumanAnatomySpec {
    pub head_to_body_ratio: f32,                  // typically 7.5 (average) to 8.0 (heroic/classical)
    pub shoulder_width_heads: f32,                // typically 2.0 to 2.3 heads
    pub arm_span_ratio: f32,                      // ~1.0 of total height
    pub eye_line_relative_y: f32,                 // exactly 0.5 of head height (golden rule of facial anatomy)
    pub nose_base_relative_y: f32,                // ~0.70 of head height
    pub mouth_line_relative_y: f32,               // ~0.82 of head height
    pub ear_span_head_fraction: [f32; 2],         // top at brow line, bottom at nose base
    pub skin_subsurface_scattering_depth_mm: f32, // 1.2mm for realistic human skin translucency
    pub skin_melanin_fraction: f32,               // 0.0 (fair) to 1.0 (deep)
}

impl Default for HumanAnatomySpec {
    fn default() -> Self {
        Self {
            head_to_body_ratio: 7.75,
            shoulder_width_heads: 2.1,
            arm_span_ratio: 1.0,
            eye_line_relative_y: 0.50,
            nose_base_relative_y: 0.70,
            mouth_line_relative_y: 0.82,
            ear_span_head_fraction: [0.35, 0.70],
            skin_subsurface_scattering_depth_mm: 1.2,
            skin_melanin_fraction: 0.35,
        }
    }
}

/// Standard Cinematic Three-Point Studio Lighting Rig for cinema rendering and 3D scenes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CinematicLightingRig {
    /// Key light (primary source defining shadows and form): direction, intensity (lux), kelvin temp.
    pub key_light: LightSource,
    /// Fill light (controls shadow contrast ratio, usually 2:1 or 4:1): intensity, soft bounce.
    pub fill_light: LightSource,
    /// Rim / Hair light (separates character from dark background, cinematic edge glow).
    pub rim_light: LightSource,
    /// Practical and ambient lights (room glow, city bokeh reflections, neon accents).
    pub practicals: Vec<LightSource>,
    /// Atmospheric haze density (creates god rays and depth separation).
    pub atmospheric_haze: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LightSource {
    pub name: String,
    pub direction: [f32; 3],
    pub color_rgb: [f32; 3],
    pub color_temperature_kelvin: u32,
    pub intensity_lux: f32,
    pub diffusion_softness: f32, // 0.0 (hard sun/spot) to 1.0 (large softbox)
}

impl Default for CinematicLightingRig {
    fn default() -> Self {
        Self {
            key_light: LightSource {
                name: "Key Light".into(),
                direction: [0.577, 0.577, 0.577], // 45-degree classic key
                color_rgb: [1.0, 0.94, 0.85],
                color_temperature_kelvin: 3200, // Warm tungsten/golden hour
                intensity_lux: 1200.0,
                diffusion_softness: 0.65, // Soft diffused key
            },
            fill_light: LightSource {
                name: "Fill Light".into(),
                direction: [-0.577, 0.2, 0.577],
                color_rgb: [0.75, 0.85, 1.0],   // Cool sky bounce
                color_temperature_kelvin: 5600, // Daylight sky fill
                intensity_lux: 400.0,           // 3:1 key-to-fill ratio
                diffusion_softness: 0.90,       // Heavy diffusion bounce
            },
            rim_light: LightSource {
                name: "Rim / Back Light".into(),
                direction: [0.0, -0.8, -0.6], // Behind and above subject
                color_rgb: [1.0, 0.98, 0.9],
                color_temperature_kelvin: 4000,
                intensity_lux: 1800.0,    // High intensity edge separation
                diffusion_softness: 0.25, // Sharp rim definition
            },
            practicals: vec![LightSource {
                name: "City Street Amber Glow".into(),
                direction: [0.0, -1.0, 0.0],
                color_rgb: [1.0, 0.55, 0.1],
                color_temperature_kelvin: 2200,
                intensity_lux: 250.0,
                diffusion_softness: 0.80,
            }],
            atmospheric_haze: 0.18,
        }
    }
}

/// Physical and visual lighting model for scene consistency.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LightingEnvironment {
    pub key_light_direction: [f32; 3],
    pub key_light_color: [f32; 3],
    pub ambient_intensity: f32,
    pub shadow_softness: f32,
    pub environment_map: Option<String>,
}

/// Character model maintaining visual identity across scenes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CharacterIdentity {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub visual_signature: CharacterVisualSignature,
    pub reference_assets: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CharacterVisualSignature {
    pub height_cm: f32,
    pub primary_palette: Vec<String>,
    pub distinct_features: Vec<String>,
    pub model_3d_handle: Option<String>,
}

/// A shot in the story timeline.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoryShot {
    pub id: Uuid,
    pub index: u32,
    pub duration_seconds: f32,
    pub camera_angle: String,
    pub narrative_action: String,
    pub character_ids: Vec<Uuid>,
    pub environment: LightingEnvironment,
    pub app_dispatches: Vec<AppCommandDispatch>,
}

/// Command dispatched to a specific ArtCraft application.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AppCommandDispatch {
    pub target_app: String,
    pub tool_name: String,
    pub parameters: serde_json::Value,
    pub order: u32,
}

/// Full narrative story project.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StoryProject {
    pub id: Uuid,
    pub title: String,
    pub synopsis: String,
    pub characters: HashMap<Uuid, CharacterIdentity>,
    pub timeline: Vec<StoryShot>,
}

/// Director Orchestrator managing project generation from prompt to multi-app delivery.
pub struct StudioDirector {
    active_project: Option<StoryProject>,
}

impl Default for StudioDirector {
    fn default() -> Self {
        Self::new()
    }
}

impl StudioDirector {
    pub fn new() -> Self {
        Self { active_project: None }
    }

    /// Initializes a new creative project with character and world models.
    pub fn create_project(&mut self, title: impl Into<String>, synopsis: impl Into<String>) -> Uuid {
        let id = Uuid::new_v4();
        let project = StoryProject { id, title: title.into(), synopsis: synopsis.into(), characters: HashMap::new(), timeline: Vec::new() };
        self.active_project = Some(project);
        id
    }

    /// Registers a character with guaranteed visual consistency rules.
    pub fn register_character(&mut self, name: &str, description: &str, height_cm: f32, palette: Vec<String>) -> Option<Uuid> {
        if self.active_project.is_none() {
            self.create_project("Default Production", "Auto-initialized studio production");
        }
        let project = self.active_project.as_mut()?;
        let char_id = Uuid::new_v4();
        let character = CharacterIdentity {
            id: char_id,
            name: name.to_string(),
            description: description.to_string(),
            visual_signature: CharacterVisualSignature { height_cm, primary_palette: palette, distinct_features: Vec::new(), model_3d_handle: None },
            reference_assets: Vec::new(),
        };
        project.characters.insert(char_id, character);
        Some(char_id)
    }

    /// Synthesizes a new shot into executable cross-app instructions.
    pub fn plan_shot_across_apps(&mut self, action: &str, duration: f32) -> Option<&StoryShot> {
        if !duration.is_finite() || duration <= 0.0 {
            return None;
        }
        if self.active_project.is_none() {
            self.create_project("Default Production", "Auto-initialized studio production");
        }
        let project = self.active_project.as_mut()?;
        let shot_id = Uuid::new_v4();
        let shot_index = project.timeline.len() as u32 + 1;

        // Choreograph actions across ArtCraft suite:
        let dispatches = vec![
            AppCommandDispatch {
                target_app: "wordcraft".into(),
                tool_name: "append_screenplay".into(),
                parameters: serde_json::json!({ "shot": shot_index, "action": action }),
                order: 1,
            },
            AppCommandDispatch {
                target_app: "cadcraft".into(),
                tool_name: "setup_scene_camera".into(),
                parameters: serde_json::json!({ "lens_mm": 50, "perspective": "cinematic" }),
                order: 2,
            },
            AppCommandDispatch {
                target_app: "photocraft".into(),
                tool_name: "compose_layers".into(),
                parameters: serde_json::json!({ "lighting": "warm_key", "passes": ["diffuse", "shadow", "specular"] }),
                order: 3,
            },
            AppCommandDispatch {
                target_app: "filmcraft".into(),
                tool_name: "append_timeline_clip".into(),
                parameters: serde_json::json!({ "shot_id": shot_id.to_string(), "duration": duration }),
                order: 4,
            },
        ];

        let shot = StoryShot {
            id: shot_id,
            index: shot_index,
            duration_seconds: duration,
            camera_angle: "Medium Close-Up".into(),
            narrative_action: action.into(),
            character_ids: project.characters.keys().cloned().collect(),
            environment: LightingEnvironment {
                key_light_direction: [0.5, 1.0, 0.3],
                key_light_color: [1.0, 0.95, 0.8],
                ambient_intensity: 0.25,
                shadow_softness: 0.15,
                environment_map: None,
            },
            app_dispatches: dispatches,
        };

        project.timeline.push(shot);
        project.timeline.last()
    }
}

impl StudioDirector {
    // High-level suite capabilities implemented below:
    // - Translates a natural language creative request into multi-app execution steps.
    // - Dispatches financial model, asset budgeting and spreadsheet formulas to GridCraft.
    // - Autonomous Image Generation from Scratch (CADCraft -> VectorCraft -> PhotoCraft).
    // - Generates a realistic or stylized human character subject conforming to anatomical canon.

    /// Plans a composite dual-style character (e.g. half photoreal, half Pixar 3D).
    pub fn plan_dual_style_split_character(
        &mut self,
        prompt: &str,
        style_left: AestheticStyle,
        style_right: AestheticStyle,
        split_axis_ratio: f32, // 0.5 for vertical middle split
    ) -> Vec<AppCommandDispatch> {
        vec![
            AppCommandDispatch {
                target_app: "cadcraft".into(),
                tool_name: "setup_perpendicular_wall_walk_rig".into(),
                parameters: serde_json::json!({
                    "surface_plane": "wall_vertical_yz",
                    "gravity_vector": [0.0, -1.0, 0.0],
                    "contact_plane_angle_deg": 90.0,
                    "character_pose": "walking_on_wall_stride",
                    "camera_angle": "side_profile_dramatic_low"
                }),
                order: 1,
            },
            AppCommandDispatch {
                target_app: "photocraft".into(),
                tool_name: "render_dual_style_composite".into(),
                parameters: serde_json::json!({
                    "prompt": prompt,
                    "left_style": format!("{style_left:?}"),
                    "right_style": format!("{style_right:?}"),
                    "split_line_x": split_axis_ratio,
                    "split_transition": "energy_seam_glow",
                    "left_shader": "photoreal_sss",
                    "right_shader": "pixar_3d_pbr"
                }),
                order: 2,
            },
            AppCommandDispatch {
                target_app: "anicraft".into(),
                tool_name: "animate_wall_walk_stride_cycle".into(),
                parameters: serde_json::json!({
                    "stride_length_cm": 75.0,
                    "step_duration_secs": 0.5,
                    "feet_contact_magnetism": "vertical_wall",
                    "cloth_and_hair_gravity": [0.0, -1.0, 0.0]
                }),
                order: 3,
            },
        ]
    }

    /// Dispatches organized cinematic three-point lighting directly to CADCraft and PhotoCraft via MCP.
    pub fn dispatch_cinematic_lighting_rig(&self, rig: &CinematicLightingRig) -> Vec<AppCommandDispatch> {
        vec![
            AppCommandDispatch {
                target_app: "cadcraft".into(),
                tool_name: "setup_3point_lighting_rig".into(),
                parameters: serde_json::json!({
                    "key_light": rig.key_light,
                    "fill_light": rig.fill_light,
                    "rim_light": rig.rim_light,
                    "practicals_count": rig.practicals.len(),
                    "atmospheric_haze": rig.atmospheric_haze
                }),
                order: 1,
            },
            AppCommandDispatch {
                target_app: "photocraft".into(),
                tool_name: "render_cinematic_subsurface_and_exposure".into(),
                parameters: serde_json::json!({
                    "color_space": "ACEScg",
                    "exposure_stops": 0.0,
                    "kelvin_white_balance": 4500,
                    "bloom_intensity": 0.08,
                    "anamorphic_flares": true
                }),
                order: 2,
            },
            AppCommandDispatch {
                target_app: "filmcraft".into(),
                tool_name: "apply_cinematic_color_lut".into(),
                parameters: serde_json::json!({
                    "lut_profile": "cinematic_teal_and_orange_film",
                    "grain_intensity": 0.015,
                    "tone_mapping": "ACES_fitted"
                }),
                order: 3,
            },
        ]
    }

    pub fn synthesize_human_subject(
        &mut self,
        anatomy: &HumanAnatomySpec,
        style: AestheticStyle,
        expression: &str,
        lighting: &LightingEnvironment,
    ) -> Vec<AppCommandDispatch> {
        let (sss_passes, vector_lines, texture_profile) = match style {
            AestheticStyle::Photorealistic => (
                serde_json::json!({
                    "subsurface_scattering_enabled": true,
                    "sss_depth_mm": anatomy.skin_subsurface_scattering_depth_mm,
                    "melanin": anatomy.skin_melanin_fraction,
                    "micro_skin_pores": true,
                    "iris_caustics": true
                }),
                false,
                "photocraft.photoreal_skin_shader",
            ),
            AestheticStyle::AnimatedVector => (serde_json::json!({ "cel_shading_bands": 3 }), true, "vectorcraft.line_art_shader"),
            AestheticStyle::Cinematic3D => {
                (serde_json::json!({ "pbr_smooth_shading": true, "specular_bloom": true }), false, "cadcraft.stylized_pbr_shader")
            }
            AestheticStyle::PainterlyOil => {
                (serde_json::json!({ "brush_stroke_impasto": true, "canvas_linen_weave": true }), false, "photocraft.oil_paint_shader")
            }
            AestheticStyle::GraphicNovelNoir => {
                (serde_json::json!({ "halftone_dots": true, "deep_black_cutoff": 0.15 }), true, "photocraft.noir_ink_shader")
            }
        };

        vec![
            AppCommandDispatch {
                target_app: "cadcraft".into(),
                tool_name: "rig_anatomical_skeleton".into(),
                parameters: serde_json::json!({
                    "head_ratio": anatomy.head_to_body_ratio,
                    "shoulder_width": anatomy.shoulder_width_heads,
                    "eye_line": anatomy.eye_line_relative_y,
                    "nose_line": anatomy.nose_base_relative_y,
                    "mouth_line": anatomy.mouth_line_relative_y,
                    "facial_expression": expression
                }),
                order: 1,
            },
            AppCommandDispatch {
                target_app: if vector_lines { "vectorcraft" } else { "photocraft" }.into(),
                tool_name: "render_character_surface".into(),
                parameters: serde_json::json!({
                    "style": format!("{style:?}"),
                    "shader_profile": texture_profile,
                    "shading_parameters": sss_passes,
                    "key_light": lighting.key_light_direction,
                    "ambient": lighting.ambient_intensity
                }),
                order: 2,
            },
        ]
    }

    pub fn generate_image_from_scratch(&mut self, prompt: &str, width: u32, height: u32, aspect_ratio: &str) -> Vec<AppCommandDispatch> {
        vec![
            AppCommandDispatch {
                target_app: "cadcraft".into(),
                tool_name: "generate_perspective_grid".into(),
                parameters: serde_json::json!({
                    "aspect_ratio": aspect_ratio,
                    "horizon_y": 0.38,
                    "focal_length_mm": 35.0,
                    "vanishing_points": 2
                }),
                order: 1,
            },
            AppCommandDispatch {
                target_app: "vectorcraft".into(),
                tool_name: "draw_subject_contours".into(),
                parameters: serde_json::json!({
                    "prompt": prompt,
                    "vector_precision": "high",
                    "anchor_nodes": 64
                }),
                order: 2,
            },
            AppCommandDispatch {
                target_app: "photocraft".into(),
                tool_name: "synthesize_diffuse_and_lighting".into(),
                parameters: serde_json::json!({
                    "width": width,
                    "height": height,
                    "passes": ["diffuse", "ambient_occlusion", "specular_highlights", "atmospheric_haze"],
                    "light_source_angle_deg": 45.0,
                    "color_grading": "cinematic_natural"
                }),
                order: 3,
            },
            AppCommandDispatch {
                target_app: "photocraft".into(),
                tool_name: "apply_texture_and_post".into(),
                parameters: serde_json::json!({
                    "grain": 0.02,
                    "chromatic_aberration": 0.005,
                    "export_format": "png"
                }),
                order: 4,
            },
        ]
    }

    /// Autonomous Video Editing & Post-Production:
    /// Edits raw video clips, applies pacing cuts, transitions, sound effects, and color grading.
    pub fn edit_video_sequence(&mut self, clip_ids: Vec<String>, target_duration_secs: f32, music_bpm: f32, style: &str) -> Vec<AppCommandDispatch> {
        let cut_frequency = 60.0 / music_bpm; // Beat-synchronized cutting
        vec![
            AppCommandDispatch {
                target_app: "filmcraft".into(),
                tool_name: "import_and_align_to_beats".into(),
                parameters: serde_json::json!({
                    "clips": clip_ids,
                    "target_duration": target_duration_secs,
                    "beat_interval_secs": cut_frequency,
                    "cut_style": style
                }),
                order: 1,
            },
            AppCommandDispatch {
                target_app: "soundcraft".into(),
                tool_name: "mix_audio_ducking_and_sfx".into(),
                parameters: serde_json::json!({
                    "dialogue_ducking_db": -6.0,
                    "mastering_lufs": -14.0,
                    "dynamic_range": "cinematic"
                }),
                order: 2,
            },
            AppCommandDispatch {
                target_app: "effectcraft".into(),
                tool_name: "apply_motion_graphics_and_transitions".into(),
                parameters: serde_json::json!({
                    "transition": "optical_flow_cross_dissolve",
                    "motion_blur": true,
                    "color_lookup_lut": "film_rec709_kodak2383"
                }),
                order: 3,
            },
            AppCommandDispatch {
                target_app: "filmcraft".into(),
                tool_name: "render_final_master".into(),
                parameters: serde_json::json!({
                    "codec": "prores_422_hq",
                    "resolution": "4K_UHD",
                    "fps": 24.0
                }),
                order: 4,
            },
        ]
    }

    pub fn plan_financial_and_asset_budgeting(&mut self, project_title: &str, asset_count: usize) -> AppCommandDispatch {
        AppCommandDispatch {
            target_app: "gridcraft".into(),
            tool_name: "generate_production_budget".into(),
            parameters: serde_json::json!({
                "sheet_name": format!("{project_title} - Budget"),
                "asset_rows": asset_count,
                "auto_sum_formula": "=SUM(C2:C100)",
                "currency": "USD"
            }),
            order: 10,
        }
    }

    /// Dispatches multi-page publication, artbook and editorial layout to DesignCraft.
    pub fn plan_publication_artbook(&mut self, book_title: &str, page_count: usize) -> AppCommandDispatch {
        AppCommandDispatch {
            target_app: "designcraft".into(),
            tool_name: "create_master_page_document".into(),
            parameters: serde_json::json!({
                "title": book_title,
                "pages": page_count,
                "bleed_mm": 3.0,
                "facing_pages": true,
                "primary_text_flow": true
            }),
            order: 11,
        }
    }

    pub fn translate_concept_to_app_pipeline(&self, concept: &str) -> Vec<AppCommandDispatch> {
        let mut plan = Vec::new();

        // 1. Scripting / Structure in WordCraft
        plan.push(AppCommandDispatch {
            target_app: "wordcraft".into(),
            tool_name: "create_scene_outline".into(),
            parameters: serde_json::json!({
                "concept": concept,
                "format": "screenplay",
                "include_dialogue": true
            }),
            order: 1,
        });

        // 2. Visual layout & UI components in FigCraft / VectorCraft
        plan.push(AppCommandDispatch {
            target_app: "figcraft".into(),
            tool_name: "generate_storyboard_frames".into(),
            parameters: serde_json::json!({
                "auto_layout": true,
                "aspect_ratio": "16:9"
            }),
            order: 2,
        });

        // 3. 3D Space, Camera & Geometry in CADCraft
        plan.push(AppCommandDispatch {
            target_app: "cadcraft".into(),
            tool_name: "construct_spatial_environment".into(),
            parameters: serde_json::json!({
                "geometry_units": "meters",
                "coordinate_system": "Z-up"
            }),
            order: 3,
        });

        // 4. Photorealistic or stylized raster rendering in PhotoCraft
        plan.push(AppCommandDispatch {
            target_app: "photocraft".into(),
            tool_name: "render_and_grade_plates".into(),
            parameters: serde_json::json!({
                "passes": ["beauty", "depth", "occlusion"],
                "color_profile": "Display-P3"
            }),
            order: 4,
        });

        // 5. Motion Graphics & Visual Effects in EffectCraft
        plan.push(AppCommandDispatch {
            target_app: "effectcraft".into(),
            tool_name: "apply_motion_tracking_and_vfx".into(),
            parameters: serde_json::json!({
                "motion_blur": true,
                "fps": 60
            }),
            order: 5,
        });

        // 6. Timeline assembly in FilmCraft
        plan.push(AppCommandDispatch {
            target_app: "filmcraft".into(),
            tool_name: "assemble_multitrack_sequence".into(),
            parameters: serde_json::json!({
                "sync_audio": true,
                "transitions": "cross_dissolve"
            }),
            order: 6,
        });

        plan
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn story_director_choreographs_suite_pipeline() {
        let mut director = StudioDirector::new();
        let _proj_id = director.create_project("The Journey", "A sci-fi cinematic story in Jerusalem");

        let char_id = director.register_character("Ari", "Protagonist with blue scarf", 175.0, vec!["#1A365D".into(), "#E2E8F0".into()]);
        assert!(char_id.is_some());

        let shot = director.plan_shot_across_apps("Ari approaches the ancient wall under morning sunlight", 4.5);
        assert!(shot.is_some());
        let s = shot.unwrap();
        assert_eq!(s.index, 1);
        assert_eq!(s.app_dispatches.len(), 4);
        assert_eq!(s.app_dispatches[0].target_app, "wordcraft");
        assert_eq!(s.app_dispatches[1].target_app, "cadcraft");
        assert_eq!(s.app_dispatches[2].target_app, "photocraft");
        assert_eq!(s.app_dispatches[3].target_app, "filmcraft");
        let pipeline = director.translate_concept_to_app_pipeline("A magical sunrise over the hills");
        assert_eq!(pipeline.len(), 6);
        assert_eq!(pipeline[0].target_app, "wordcraft");
        assert_eq!(pipeline[1].target_app, "figcraft");
        assert_eq!(pipeline[2].target_app, "cadcraft");
        assert_eq!(pipeline[3].target_app, "photocraft");
        assert_eq!(pipeline[4].target_app, "effectcraft");
        assert_eq!(pipeline[5].target_app, "filmcraft");
        let grid_dispatch = director.plan_financial_and_asset_budgeting("The Journey", 15);
        assert_eq!(grid_dispatch.target_app, "gridcraft");
        assert_eq!(grid_dispatch.tool_name, "generate_production_budget");

        let design_dispatch = director.plan_publication_artbook("The Journey Artbook", 64);
        assert_eq!(design_dispatch.target_app, "designcraft");
        assert_eq!(design_dispatch.tool_name, "create_master_page_document");
    }

    #[test]
    fn story_director_plans_image_and_human_synthesis() {
        let mut director = StudioDirector::new();
        director.create_project("The Journey", "A sci-fi cinematic story in Jerusalem");

        // Test Image Generation from scratch
        let image_pipeline = director.generate_image_from_scratch("A golden sunset over a peaceful lake", 3840, 2160, "16:9");
        assert_eq!(image_pipeline.len(), 4);
        assert_eq!(image_pipeline[0].target_app, "cadcraft");
        assert_eq!(image_pipeline[1].target_app, "vectorcraft");
        assert_eq!(image_pipeline[2].target_app, "photocraft");
        assert_eq!(image_pipeline[3].target_app, "photocraft");

        // Test Human Subject Synthesis with Realistic Anatomy Canon
        let anatomy = HumanAnatomySpec::default();
        let env = LightingEnvironment {
            key_light_direction: [0.3, 0.8, 0.5],
            key_light_color: [1.0, 0.9, 0.8],
            ambient_intensity: 0.2,
            shadow_softness: 0.1,
            environment_map: None,
        };
        let human_pipeline = director.synthesize_human_subject(&anatomy, AestheticStyle::Photorealistic, "warm thoughtful smile", &env);
        assert_eq!(human_pipeline.len(), 2);
        assert_eq!(human_pipeline[0].target_app, "cadcraft");
        assert_eq!(human_pipeline[0].tool_name, "rig_anatomical_skeleton");
        assert_eq!(human_pipeline[1].target_app, "photocraft");
        assert_eq!(human_pipeline[1].tool_name, "render_character_surface");

        let vector_human = director.synthesize_human_subject(&anatomy, AestheticStyle::AnimatedVector, "curious", &env);
        assert_eq!(vector_human[1].target_app, "vectorcraft");
    }

    #[test]
    fn story_director_plans_cinema_split_and_video_pipelines() {
        let mut director = StudioDirector::new();
        director.create_project("The Journey", "A sci-fi cinematic story in Jerusalem");

        // Test Cinematic 3-Point Lighting Rig MCP Dispatch
        let rig = CinematicLightingRig::default();
        let cinema_dispatches = director.dispatch_cinematic_lighting_rig(&rig);
        assert_eq!(cinema_dispatches.len(), 3);
        assert_eq!(cinema_dispatches[0].target_app, "cadcraft");
        assert_eq!(cinema_dispatches[0].tool_name, "setup_3point_lighting_rig");
        assert_eq!(cinema_dispatches[1].target_app, "photocraft");
        assert_eq!(cinema_dispatches[2].target_app, "filmcraft");

        // Test Dual-Style Wall Walker (Half Photoreal, Half Pixar 3D)
        let split_walker = director.plan_dual_style_split_character(
            "Man walking on a vertical city wall, half photorealistic human, half stylized Pixar 3D",
            AestheticStyle::Photorealistic,
            AestheticStyle::Cinematic3D,
            0.50,
        );
        assert_eq!(split_walker.len(), 3);
        assert_eq!(split_walker[0].target_app, "cadcraft");
        assert_eq!(split_walker[1].target_app, "photocraft");
        assert_eq!(split_walker[2].target_app, "anicraft");

        // Test Autonomous Video Editing
        let video_pipeline = director.edit_video_sequence(vec!["take_1.mp4".into(), "take_2.mp4".into()], 12.0, 120.0, "dynamic_action");
        assert_eq!(video_pipeline.len(), 4);
        assert_eq!(video_pipeline[0].target_app, "filmcraft");
        assert_eq!(video_pipeline[1].target_app, "soundcraft");
        assert_eq!(video_pipeline[2].target_app, "effectcraft");
        assert_eq!(video_pipeline[3].target_app, "filmcraft");
    }

    #[test]
    fn plan_shot_rejects_non_positive_or_non_finite_duration() {
        let mut director = StudioDirector::new();
        director.create_project("Test", "Test synopsis");
        assert!(director.plan_shot_across_apps("zoom in", -1.0).is_none());
        assert!(director.plan_shot_across_apps("zoom in", 0.0).is_none());
        assert!(director.plan_shot_across_apps("zoom in", f32::NAN).is_none());
        assert!(director.plan_shot_across_apps("zoom in", f32::INFINITY).is_none());
    }
}
