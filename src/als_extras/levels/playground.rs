use bevy_als::als::character::AlsCollisionLayer;
use std::{collections::HashMap, path::PathBuf};

use crate::source_gaussian_bloom::{SourceGaussianBloom, SourceGaussianBloomPlugin};
use bevy::asset::AssetId;
use bevy::camera::{Exposure, visibility::NoFrustumCulling};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::core_pipeline::tonemapping::TonemappingLuts;
use bevy::light::{CascadeShadowConfigBuilder, NotShadowCaster};
use bevy::post_process::auto_exposure::{
    AutoExposure, AutoExposureCompensationCurve, AutoExposurePlugin,
};
use bevy::prelude::*;
use bevy::render::view::{ColorGrading, ColorGradingSection};
use bevy::sprite::Anchor;
use bevy::text::{LetterSpacing, LineHeight, TextLayoutInfo};
use bevy::world_serialization::WorldAssetRoot;
use ron::de::from_str;
use serde::Deserialize;

use bevy_als::als::character::AlsCrushRecoveryFloor;
use crate::player_start::{AlsPlayerStart, AlsPlayerStartPlugin};
use avian3d::prelude::{Collider, CollisionLayers, LayerMask, RigidBody};

use crate::playground_materials::PlaygroundMaterialsPlugin;
use crate::playground_materials::{
    PlaygroundMaterialLibrary, PlaygroundTilesExtension, PlaygroundTilesMaterial,
    PlaygroundTilesSettings,
};
use crate::playground_sky::{
    PlaygroundSkyMaterial, PlaygroundSkyPlugin, spawn_playground_sky,
};
use bevy_als::extensions::{RichCurveKey, sample_rich_curve};

pub const PLAYGROUND_SHADOW_DEPTH_BIAS: f32 = DirectionalLight::DEFAULT_SHADOW_DEPTH_BIAS;
pub const PLAYGROUND_SHADOW_NORMAL_BIAS: f32 = DirectionalLight::DEFAULT_SHADOW_NORMAL_BIAS;

pub struct PlaygroundPlugin {
    pub manifest: PathBuf,
}

impl Plugin for PlaygroundPlugin {
    fn build(&self, app: &mut App) {
        let manifest = self.manifest.clone();
        app.add_plugins((
            PlaygroundMaterialsPlugin,
            PlaygroundSkyPlugin,
            AlsPlayerStartPlugin,
            AutoExposurePlugin,
            SourceGaussianBloomPlugin,
        ))
        .init_resource::<PlaygroundMovingCurve>()
        .add_systems(
            Startup,
            (
                move |world: &mut World| {
                    world.insert_resource(PlaygroundManifestPath(manifest.clone()));
                },
                playground_setup_system,
            )
                .chain(),
        )
        .add_systems(Update, playground_moving_object_system)
        .add_systems(
            Update,
            (
                playground_box_physics_bridge,
                world_text_mesh_system,
                playground_material_tint_system
                    .before(crate::projected_decal::PackedDecalPrepareSet),
                playground_camera_exposure_system,
            ),
        );
    }
}

fn playground_box_physics_bridge(
    mut commands: Commands,
    bodies: Query<
        (
            Entity,
            &Name,
            &PlaygroundBoxCollider,
            Option<&ChildOf>,
            Option<&CollisionLayers>,
        ),
        Added<PlaygroundBoxCollider>,
    >,
    moving_roots: Query<(), With<PlaygroundMovingObject>>,
    root_names: Query<&Name>,
) {
    for (entity, name, collider, parent, layers) in &bodies {
        let shape = Collider::cuboid(
            collider.half_extents.x * 2.0,
            collider.half_extents.y * 2.0,
            collider.half_extents.z * 2.0,
        );
        if let Some(&ChildOf(root)) = parent
            && moving_roots.contains(root)
        {
            continue;
        } else {
            let parent_is_floor = parent.is_some_and(|&ChildOf(root)| {
                root_names
                    .get(root)
                    .is_ok_and(|root_name| root_name.as_str().ends_with(":BP_SimpleObject5"))
            });
            let is_floor = parent_is_floor || name.as_str() == "BP_SimpleObject5:aabb";
            let mut entity_commands = commands.entity(entity);
            entity_commands.insert((
                RigidBody::Static,
                shape,
                layers.copied().unwrap_or_else(|| {
                    CollisionLayers::new(
                        [AlsCollisionLayer::Default, AlsCollisionLayer::Visibility],
                        LayerMask::ALL,
                    )
                }),
            ));
            if is_floor {
                entity_commands.insert(AlsCrushRecoveryFloor {
                    half_extents: collider.half_extents,
                });
            }
        }
    }
}

#[derive(Resource)]
pub struct PlaygroundManifestPath(pub PathBuf);

#[derive(Debug, Deserialize)]
pub struct PlaygroundManifest {
    #[serde(default)]
    pub actors: Vec<PlaygroundActor>,
    #[serde(default)]
    pub lighting: Option<SceneLighting>,
    #[serde(default)]
    pub sun_rotator: Option<PlaygroundRotator>,
    #[serde(default)]
    pub terrain: Option<SceneTerrain>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SceneTerrain {
    pub origin_ue_x_cm: f32,
    pub origin_ue_y_cm: f32,
    pub step_cm: f32,
    pub heights_cm: Vec<Vec<f32>>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct SceneLighting {
    pub sun_direction: Option<[f32; 3]>,
    pub sun_intensity: Option<f32>,
    pub sun_color: Option<[f32; 4]>,
    pub sky_zenith: Option<[f32; 4]>,
    pub sky_horizon: Option<[f32; 4]>,
    pub sky_sun_brightness: Option<f32>,
    pub sky_horizon_falloff: Option<f32>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PlaygroundActor {
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub location: [f32; 3],
    #[serde(default)]
    pub rotation_yaw: f32,
    #[serde(default)]
    pub rotation: Option<PlaygroundRotator>,
    #[serde(default)]
    pub scale: [f32; 3],
    #[serde(default)]
    pub bounds_origin: Option<[f32; 3]>,
    #[serde(default)]
    pub bounds_extent: Option<[f32; 3]>,
    #[serde(default, rename = "mesh")]
    pub mesh_path: Option<String>,
    #[serde(default)]
    pub components: Vec<PlaygroundComponent>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub text_color: Option<[f32; 4]>,
    #[serde(default)]
    pub text_world_size: Option<f32>,
    #[serde(default)]
    pub text_horizontal_alignment: Option<String>,
    #[serde(default)]
    pub text_vertical_alignment: Option<String>,
    #[serde(default)]
    pub text_x_scale: Option<f32>,
    #[serde(default)]
    pub text_y_scale: Option<f32>,
    #[serde(default)]
    pub text_horizontal_spacing_adjust: Option<f32>,
    #[serde(default)]
    pub text_vertical_spacing_adjust: Option<f32>,
    #[serde(default)]
    pub top_color: Option<[f32; 3]>,
    #[serde(default)]
    pub side_color: Option<[f32; 3]>,
    #[serde(default)]
    pub tiling: Option<f32>,
    #[serde(default)]
    pub movement_spline_start: Option<[f32; 3]>,
    #[serde(default)]
    pub movement_spline_end: Option<[f32; 3]>,
    #[serde(default)]
    pub move_duration: Option<f32>,
    #[serde(default)]
    pub rotate_speed_deg: Option<f32>,
}

#[derive(Debug, Deserialize)]
pub struct PlaygroundComponent {
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "mesh")]
    pub mesh_path: Option<String>,
    #[serde(default)]
    pub location: Option<[f32; 3]>,
    #[serde(default)]
    pub rotation: Option<PlaygroundRotator>,
    #[serde(default)]
    pub scale: Option<[f32; 3]>,
    #[serde(default)]
    pub local_box_origin: Option<[f32; 3]>,
    #[serde(default)]
    pub local_box_extent: Option<[f32; 3]>,
    #[serde(default)]
    pub world_aabb_center: Option<[f32; 3]>,
    #[serde(default)]
    pub world_aabb_extent: Option<[f32; 3]>,
}

#[derive(Debug, Deserialize)]
pub struct PlaygroundRotator {
    pub pitch: f32,
    pub yaw: f32,
    pub roll: f32,
}

#[derive(Component, Debug)]
pub struct PlaygroundStatic;

#[derive(Component, Debug)]
pub struct PlaygroundBoxCollider {
    pub half_extents: Vec3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlaygroundColliderKind {
    Box,
    Convex { source_hulls: u8 },
    TriMesh,
}

impl PlaygroundColliderKind {
    pub fn is_box(self) -> bool {
        matches!(self, Self::Box)
    }
}

#[derive(Component, Debug, Clone)]
pub struct PlaygroundCollisionSource {
    pub mesh_base: String,
    pub kind: PlaygroundColliderKind,
}

impl PlaygroundCollisionSource {
    pub fn new(mesh_base: impl Into<String>) -> Self {
        let mesh_base = mesh_base.into();
        Self {
            kind: playground_collider_kind(&mesh_base),
            mesh_base,
        }
    }
}

pub fn playground_collider_kind(mesh_base: &str) -> PlaygroundColliderKind {
    match mesh_base {
        "SM_Als_Block_2x2" | "SM_Als_Block_3x3" | "SM_Als_Crate" | "SM_Als_Floor_20x20"
        | "SM_Als_Pillar" | "SM_Als_TitleBoard" | "SM_Als_Wall_2x20" | "SM_Als_Wall_3x5" => {
            PlaygroundColliderKind::Box
        }
        "SM_Als_Arch_01" => PlaygroundColliderKind::Convex { source_hulls: 5 },
        "SM_Als_Arch_02" => PlaygroundColliderKind::Convex { source_hulls: 4 },
        "SM_Als_Barrel" | "SM_Als_Barrel_Broken" => {
            PlaygroundColliderKind::Convex { source_hulls: 1 }
        }
        "SM_Als_Bench" => PlaygroundColliderKind::Convex { source_hulls: 4 },
        "SM_Als_Cart"
        | "SM_Als_Crate_Broken"
        | "SM_Als_Fountain_01"
        | "SM_Als_Rock"
        | "SM_Als_Stairs_01"
        | "SM_Als_Stairs_02"
        | "SM_Als_Stairs_03" => PlaygroundColliderKind::TriMesh,
        _ => PlaygroundColliderKind::Box,
    }
}

#[derive(Component, Debug)]
pub struct PlaygroundMovingObject;

#[derive(Component, Debug)]
pub struct PlaygroundPhysicsDriven;

#[derive(Debug, Clone, Copy)]
pub enum PlaygroundKinematicKind {
    MoveSpline {
        duration: f32,
        start: Vec3,
        end: Vec3,
    },
    RotateWorldTime {
        rate_yzx_radians: bevy::math::DVec3,
    },
}

#[derive(Component, Debug)]
pub struct PlaygroundKinematic {
    pub base_transform: Transform,
    pub kind: PlaygroundKinematicKind,
}

#[derive(Deserialize)]
pub(crate) struct MovingTransformDefinition {
    translation: [f32; 3],
    rotation: [f32; 4],
    scale: [f32; 3],
}

impl MovingTransformDefinition {
    pub(crate) fn transform(&self) -> Transform {
        Transform::from_translation(Vec3::from_array(self.translation))
            .with_rotation(Quat::from_array(self.rotation))
            .with_scale(Vec3::from_array(self.scale))
    }
}

#[derive(Deserialize)]
struct MovingActorDefinition {
    name: String,
    actor: MovingTransformDefinition,
    primitive: MovingTransformDefinition,
    mesh_scene: String,
    mesh_base: String,
    collider_center: [f32; 3],
    collider_rotation: [f32; 4],
    half_extents: [f32; 3],
    #[serde(flatten)]
    motion: MovingMotionDefinition,
}

#[derive(Deserialize, Clone, Copy)]
#[serde(untagged)]
enum MovingMotionDefinition {
    Spline {
        start: [f32; 3],
        end: [f32; 3],
        duration: f32,
    },
    Rotation {
        rotation_rate_yzx_radians: [f64; 3],
    },
}

#[derive(Deserialize)]
struct MovingActorDefinitions {
    actors: Vec<MovingActorDefinition>,
}

const MOVING_ACTOR_DEFINITIONS: &str =
    include_str!("../../../assets/ALSExtras/Levels/L_Als_Playground/moving_actors.runtime.json");
const ROTATING_ACTOR_DEFINITIONS: &str =
    include_str!("../../../assets/ALSExtras/Levels/L_Als_Playground/rotating_actors.runtime.json");

pub fn rotating_object_rotation(rate_yzx_radians: bevy::math::DVec3, elapsed_seconds: f32) -> Quat {
    let angles = rate_yzx_radians * f64::from(elapsed_seconds);
    bevy::math::DQuat::from_euler(EulerRot::YZX, angles.x, angles.y, angles.z)
        .as_quat()
        .normalize()
}

const MOVING_OBJECT_CURVE_PATH: &str = "assets/ALSExtras/Environment/CF_Als_MovingObject.rich.json";
const MOVING_OBJECT_CURVE_JSON: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/ALSExtras/Environment/CF_Als_MovingObject.rich.json"
));

#[derive(Resource, Clone, Default, Deserialize)]
pub struct PlaygroundMovingCurve {
    keys: Vec<RichCurveKey>,
}

impl PlaygroundMovingCurve {
    pub fn sample(&self, time_normalized: f32) -> f32 {
        sample_rich_curve(&self.keys, time_normalized)
    }

    pub fn is_source_loaded(&self) -> bool {
        self.keys.len() >= 2
    }

    pub fn key_count(&self) -> usize {
        self.keys.len()
    }
}

pub fn load_moving_object_curve() -> Result<PlaygroundMovingCurve, String> {
    let source: PlaygroundMovingCurve = serde_json::from_str(MOVING_OBJECT_CURVE_JSON)
        .map_err(|error| format!("parse {MOVING_OBJECT_CURVE_PATH}: {error}"))?;
    if source.keys.len() < 2 {
        return Err(format!(
            "curve {MOVING_OBJECT_CURVE_PATH} has fewer than two keys"
        ));
    }
    Ok(source)
}

pub fn moving_object_curve_input(elapsed_seconds: f32, duration_seconds: f32) -> f32 {
    if duration_seconds <= 0.0 {
        return 0.0;
    }
    let ratio = (elapsed_seconds / duration_seconds).max(0.0);
    let whole = ratio.floor() as i64;
    let fraction = ratio.fract();
    if whole.rem_euclid(2) == 0 {
        fraction
    } else {
        1.0 - fraction
    }
}

pub fn moving_object_position(
    curve: &PlaygroundMovingCurve,
    start: Vec3,
    end: Vec3,
    elapsed_seconds: f32,
    duration_seconds: f32,
) -> Vec3 {
    let curve_time = moving_object_curve_input(elapsed_seconds, duration_seconds);
    start.lerp(end, curve.sample(curve_time))
}

#[derive(Resource, Debug, Clone, Deserialize)]
struct PlaygroundLighting {
    exposure: PlaygroundCameraExposure,
    sky_light: PlaygroundSkyLight,
}

#[derive(Debug, Clone, Deserialize)]
struct PlaygroundSkyLight {
    intensity: f32,
}

#[derive(Debug, Clone, Deserialize)]
struct PlaygroundCameraExposure {
    histogram_log_luminance_range: [f32; 2],
    filter: [f32; 2],
    speed_brighten: f32,
    speed_darken: f32,
    exponential_transition_distance: f32,
    compensation_ev: f32,
    unit_exposure_ev100: f32,
    contrast: f32,
}

fn imported_lighting() -> PlaygroundLighting {
    serde_json::from_str(include_str!(
        "../../../assets/ALSExtras/Levels/L_Als_Playground/playground_lighting.runtime.json"
    ))
    .expect("Source lighting data must parse")
}

#[derive(Component, Debug)]
pub struct PlaygroundSceneExposure;

const CM_TO_M: f32 = 0.01;
const CUBE_8_ACTOR: &str = "StaticMeshActor_UAID_54BF6414A059D82202_1613061415";
const LANDSCAPE_TILES_COLOR: [f32; 3] = [0.25, 0.5, 0.125];
const LANDSCAPE_TILES_TILING: f32 = 0.08;

#[derive(Component, Debug)]
pub struct PlaygroundLandscapeTilesMaterial;

#[derive(Component, Debug)]
struct PlaygroundNoShadowCaster;

const SKIP_CLASSES: &[&str] = &[
    "Brush",
    "WorldSettings",
    "RecastNavMesh",
    "NavMeshBoundsVolume",
    "LandscapeGizmoActiveActor",
    "BP_Sky_Sphere_C",
    "SkyAtmosphere",
    "SkyLight",
    "AmbientSound",
    "PostProcessVolume",
    "DirectionalLight",
    "BuoyancyManager",
    "DefaultPhysicsVolume",
    "GameplayDebuggerPlayerManager",
    "AbstractNavData",
    "L_Als_Playground_C",
    "B_Als_AICharacter_C",
];

fn is_skipped(class: &str) -> bool {
    SKIP_CLASSES.contains(&class)
}

pub fn ue_point_to_bevy(p_cm: Vec3) -> Vec3 {
    Vec3::new(p_cm.x * CM_TO_M, p_cm.z * CM_TO_M, p_cm.y * CM_TO_M)
}

pub fn ue_extents_to_bevy(e_cm: Vec3) -> Vec3 {
    Vec3::new(e_cm.x * CM_TO_M, e_cm.z * CM_TO_M, e_cm.y * CM_TO_M)
}

pub fn ue_scale_to_bevy(scale: Vec3) -> Vec3 {
    Vec3::new(scale.x, scale.z, scale.y)
}

pub fn ue_quat_to_bevy(q: Quat) -> Quat {
    Quat::from_xyzw(-q.x, -q.z, -q.y, q.w)
}

pub fn ue_rotator_to_bevy_quat(pitch_deg: f32, yaw_deg: f32, roll_deg: f32) -> Quat {
    let pitch = pitch_deg.to_radians();
    let yaw = yaw_deg.to_radians();
    let roll = roll_deg.to_radians();
    let qx = Quat::from_rotation_x(-roll);
    let qy = Quat::from_rotation_y(-pitch);
    let qz = Quat::from_rotation_z(yaw);
    ue_quat_to_bevy(qz * qy * qx)
}

pub fn ue_yaw_to_bevy_quat(yaw_deg: f32) -> Quat {
    ue_rotator_to_bevy_quat(0.0, yaw_deg, 0.0)
}

pub fn ue_directional_light_ray_to_bevy(pitch_deg: f32, yaw_deg: f32) -> Vec3 {
    let (sin_pitch, cos_pitch) = pitch_deg.to_radians().sin_cos();
    let (sin_yaw, cos_yaw) = yaw_deg.to_radians().sin_cos();
    Vec3::new(cos_pitch * cos_yaw, sin_pitch, cos_pitch * sin_yaw).normalize()
}

fn gltf_asset_base(mesh_path: &str) -> Option<String> {
    if !mesh_path.contains(".SM_Als_") {
        return None;
    }
    let file = mesh_path.rsplit('/').next()?;
    let base = file.split('.').next()?;
    Some(base.to_string())
}

pub fn is_engine_basic(mesh_path: &str) -> bool {
    mesh_path.starts_with("/Engine/BasicShapes/")
}

struct BodyPlan {
    loc_cm: Vec3,
    quat: Quat,
    visual_scale: Vec3,
    half_extents_model: Vec3,
    center_offset_model: Vec3,
    center_world: Vec3,
    obb: bool,
    mesh_base: Option<String>,
}

fn actor_rotator(actor: &PlaygroundActor) -> (f32, f32, f32) {
    match &actor.rotation {
        Some(r) => (r.pitch, r.yaw, r.roll),
        None => (0.0, actor.rotation_yaw, 0.0),
    }
}

fn comp_rotator(comp: &PlaygroundComponent, fallback: (f32, f32, f32)) -> (f32, f32, f32) {
    match &comp.rotation {
        Some(r) => (r.pitch, r.yaw, r.roll),
        None => fallback,
    }
}

fn body_plan_from_comp(
    comp: &PlaygroundComponent,
    actor_loc: [f32; 3],
    actor_scale: [f32; 3],
    rotator: (f32, f32, f32),
) -> Option<BodyPlan> {
    let loc_arr = comp.location.unwrap_or(actor_loc);
    let scale_arr = comp.scale.unwrap_or(actor_scale);
    let loc_cm = Vec3::from(loc_arr);
    let scale = Vec3::from(scale_arr);
    let rot = comp_rotator(comp, rotator);
    let quat = ue_rotator_to_bevy_quat(rot.0, rot.1, rot.2);
    let mesh_base = comp.mesh_path.as_deref().and_then(gltf_asset_base);

    if let (Some(org), Some(ext)) = (comp.local_box_origin, comp.local_box_extent) {
        let scaled_ext = Vec3::from(ext) * scale;
        return Some(BodyPlan {
            loc_cm,
            quat,
            visual_scale: ue_scale_to_bevy(scale),
            half_extents_model: ue_extents_to_bevy(scaled_ext),
            center_offset_model: ue_point_to_bevy(Vec3::from(org) * scale),
            center_world: ue_point_to_bevy(loc_cm),
            obb: true,
            mesh_base,
        });
    }

    if let (Some(c), Some(_e)) = (comp.world_aabb_center, comp.world_aabb_extent) {
        return Some(BodyPlan {
            loc_cm,
            quat,
            visual_scale: ue_scale_to_bevy(scale),
            half_extents_model: comp
                .world_aabb_extent
                .map(|e| ue_extents_to_bevy(Vec3::from(e)))
                .unwrap_or(Vec3::ONE),
            center_offset_model: Vec3::ZERO,
            center_world: ue_point_to_bevy(Vec3::from(c)),
            obb: false,
            mesh_base,
        });
    }

    None
}

fn body_plan_from_actor_bounds(actor: &PlaygroundActor) -> Option<BodyPlan> {
    let org = actor.bounds_origin?;
    let ext = actor.bounds_extent?;
    let rotator = actor_rotator(actor);
    let quat = ue_rotator_to_bevy_quat(rotator.0, rotator.1, rotator.2);
    let scale = Vec3::from(actor.scale);
    if actor
        .components
        .iter()
        .any(|c| c.mesh_path.as_deref() == Some("/Engine/BasicShapes/Cube.Cube"))
    {
        let base = ue_extents_to_bevy(Vec3::from(ext));
        return Some(BodyPlan {
            loc_cm: Vec3::from(org),
            quat,
            visual_scale: ue_scale_to_bevy(scale),
            half_extents_model: (base / scale).max(Vec3::splat(0.01)),
            center_offset_model: Vec3::ZERO,
            center_world: ue_point_to_bevy(Vec3::from(org)),
            obb: true,
            mesh_base: None,
        });
    }
    Some(BodyPlan {
        loc_cm: Vec3::from(org),
        quat,
        visual_scale: ue_scale_to_bevy(scale),
        half_extents_model: ue_extents_to_bevy(Vec3::from(ext)),
        center_offset_model: Vec3::ZERO,
        center_world: ue_point_to_bevy(Vec3::from(org)),
        obb: true,
        mesh_base: None,
    })
}

fn inverse_scale(scale: Vec3) -> Vec3 {
    Vec3::new(scale.x.recip(), scale.y.recip(), scale.z.recip())
}

fn spawn_box_visual(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: PlaygroundMaterialTint,
    transform: Transform,
    half_extents: Vec3,
    name: String,
    no_shadow_caster: bool,
    collision_layers: CollisionLayers,
) -> Entity {
    let role = material.role();
    let entity = commands
        .spawn((
            Name::new(name),
            transform,
            Mesh3d(meshes.add(Mesh::from(Cuboid::new(
                half_extents.x * 2.0,
                half_extents.y * 2.0,
                half_extents.z * 2.0,
            )))),
            PlaygroundStatic,
            PlaygroundBoxCollider { half_extents },
            collision_layers,
            PlaygroundMaterialApplied(role),
        ))
        .id();
    if no_shadow_caster {
        commands.entity(entity).insert(NotShadowCaster);
    }
    match material {
        PlaygroundMaterialTint::Main(handle) => {
            commands.entity(entity).insert(MeshMaterial3d(handle));
        }
        PlaygroundMaterialTint::Tiles(handle) | PlaygroundMaterialTint::Translucent(handle) => {
            commands.entity(entity).insert(MeshMaterial3d(handle));
        }
    }
    entity
}

fn color_parameter(value: [f32; 3]) -> Vec4 {
    Vec4::new(value[0], value[1], value[2], 1.0)
}

fn make_tiles_material(
    library: &PlaygroundMaterialLibrary,
    side_color_a: [f32; 3],
    top_color_a: [f32; 3],
    side_color_b: [f32; 3],
    top_color_b: [f32; 3],
    tiling: f32,
) -> Option<PlaygroundTilesMaterial> {
    Some(PlaygroundTilesMaterial {
        base: StandardMaterial {
            base_color: Color::linear_rgb(1.0, 1.0, 1.0),
            normal_map_texture: library.tiles_normal.clone(),
            occlusion_texture: library.tiles_occlusion.clone(),
            uv_transform: bevy::math::Affine2::from_scale(Vec2::splat(tiling)),
            flip_normal_map_y: true,
            perceptual_roughness: 0.5,
            ..default()
        },
        extension: PlaygroundTilesExtension {
            decals: library.decal_buffers.materials.clone(),
            probe: library.decal_buffers.probe.clone(),
            settings: PlaygroundTilesSettings {
                side_color_a: color_parameter(side_color_a),
                top_color_a: color_parameter(top_color_a),
                side_color_b: color_parameter(side_color_b),
                top_color_b: color_parameter(top_color_b),
                tiling_and_mode: Vec4::new(tiling, 0.0, 0.0, 0.0),
            },
            grid_texture: library.tiles_grid.clone()?,
        },
    })
}

fn tiles_material_for_actor(
    actor: &PlaygroundActor,
    library: &PlaygroundMaterialLibrary,
    tiles_materials: &mut Assets<PlaygroundTilesMaterial>,
    main_material: &Handle<StandardMaterial>,
) -> PlaygroundMaterialTint {
    let side_color = actor.side_color.unwrap_or([1.0, 1.0, 1.0]);
    let top_color = actor.top_color.unwrap_or([1.0, 1.0, 1.0]);
    let tiling = actor.tiling.unwrap_or(1.0);
    make_tiles_material(
        library, side_color, top_color, side_color, top_color, tiling,
    )
    .map(|material| PlaygroundMaterialTint::Tiles(tiles_materials.add(material)))
    .unwrap_or_else(|| {
        warn!("PLAYGROUND tiles material unavailable for {}", actor.name);
        PlaygroundMaterialTint::Main(main_material.clone())
    })
}

fn make_translucent_material(
    library: &PlaygroundMaterialLibrary,
) -> Option<PlaygroundTilesMaterial> {
    Some(PlaygroundTilesMaterial {
        base: StandardMaterial {
            base_color: Color::linear_rgb(1.0, 1.0, 1.0),
            perceptual_roughness: 0.5,
            alpha_mode: AlphaMode::Blend,
            double_sided: true,
            cull_mode: None,
            ..default()
        },
        extension: PlaygroundTilesExtension {
            decals: library.decal_buffers.materials.clone(),
            probe: library.decal_buffers.probe.clone(),
            settings: PlaygroundTilesSettings {
                side_color_a: Vec4::ONE,
                top_color_a: Vec4::ONE,
                side_color_b: Vec4::ONE,
                top_color_b: Vec4::ONE,
                tiling_and_mode: Vec4::new(1.0, 1.0, 0.0, 0.0),
            },
            grid_texture: library.tiles_grid.clone()?,
        },
    })
}

pub fn playground_setup_system(
    manifest_path: Option<Res<PlaygroundManifestPath>>,
    mut commands: Commands,
    assets: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut tiles_materials: ResMut<Assets<PlaygroundTilesMaterial>>,
    mut sky_materials: ResMut<Assets<PlaygroundSkyMaterial>>,
    mut images: ResMut<Assets<Image>>,
    material_library: Res<PlaygroundMaterialLibrary>,
) {
    let Some(manifest_path) = manifest_path else {
        error!("PLAYGROUND manifest path unavailable");
        return;
    };
    let text = match std::fs::read_to_string(&manifest_path.0) {
        Ok(t) => t,
        Err(e) => {
            error!(
                "PLAYGROUND manifest read failed path={:?} err={e}",
                manifest_path.0
            );
            return;
        }
    };
    let manifest: PlaygroundManifest = match from_str(&text) {
        Ok(m) => m,
        Err(e) => {
            error!("PLAYGROUND manifest parse failed err={e}");
            return;
        }
    };
    let mut moving_definitions: MovingActorDefinitions =
        serde_json::from_str(MOVING_ACTOR_DEFINITIONS)
            .expect("Generated moving actor definitions must be valid");
    let rotating_definitions: MovingActorDefinitions =
        serde_json::from_str(ROTATING_ACTOR_DEFINITIONS)
            .expect("Generated rotating actor definitions must be valid");
    moving_definitions
        .actors
        .extend(rotating_definitions.actors);

    let main_material = materials.add(StandardMaterial {
        base_color: Color::linear_rgb(1.0, 1.0, 1.0),
        perceptual_roughness: 0.5,
        ..default()
    });
    let translucent_material =
        make_translucent_material(&material_library).map(|material| tiles_materials.add(material));
    let landscape_material = make_tiles_material(
        &material_library,
        LANDSCAPE_TILES_COLOR,
        LANDSCAPE_TILES_COLOR,
        LANDSCAPE_TILES_COLOR,
        LANDSCAPE_TILES_COLOR,
        LANDSCAPE_TILES_TILING,
    )
    .map(|material| tiles_materials.add(material));

    let visibility: HashMap<String, bool> = serde_json::from_str(include_str!(
        "../../../assets/ALSExtras/Levels/L_Als_Playground/visibility.runtime.json"
    ))
    .expect("Generated Playground visibility must be valid");

    let mut spawned_static = 0usize;
    let mut spawned_moving = 0usize;
    let mut skipped = 0usize;
    let mut spawn_point_found = false;
    let mut spawned_landscape = 0usize;
    let mut spawned_text = 0usize;

    match load_moving_object_curve() {
        Ok(curve) => {
            info!(
                "[PGKIN] moving curve loaded keys={} path={MOVING_OBJECT_CURVE_PATH}",
                curve.key_count()
            );
            commands.insert_resource(curve);
        }
        Err(error) => error!("[PGKIN] moving curve unavailable err={error}"),
    }

    for actor in &manifest.actors {
        if actor.class == "Landscape" || actor.class == "LandscapeProxy" {
            let scene = assets.load("ALSExtras/Levels/L_Als_Playground/Landscape1.glb#Scene0");
            let mut landscape = commands.spawn((
                Name::new(format!("Landscape:{}", actor.name)),
                Transform::IDENTITY,
                WorldAssetRoot(scene),
                PlaygroundStatic,
                PlaygroundLandscape,
            ));
            if let Some(material) = &landscape_material {
                landscape.insert(PlaygroundMaterialTint::Tiles(material.clone()));
            } else {
                warn!("PLAYGROUND landscape tiles material unavailable");
            }
            spawned_landscape += 1;
            continue;
        }
        if actor.class.starts_with("TextRenderActor") || actor.class == "TextRenderActor" {
            spawn_text_label(&mut commands, actor);
            spawned_text += 1;
            continue;
        }
        if is_skipped(&actor.class) {
            skipped += 1;
            continue;
        }
        match actor.class.as_str() {
            "PlayerStart" => {
                commands.spawn((
                    Name::new(format!("PlayerStart:{}", actor.name)),
                    AlsPlayerStart::default(),
                    Transform::from_translation(ue_point_to_bevy(Vec3::from(actor.location)))
                        .with_rotation(ue_yaw_to_bevy_quat(actor.rotation_yaw)),
                ));
                spawn_point_found = true;
                continue;
            }
            "B_Als_MovingObject_C" | "B_Als_RotatingObject_C" => {
                let Some(definition) = moving_definitions
                    .actors
                    .iter()
                    .find(|value| value.name == actor.name)
                else {
                    error!("PLAYGROUND moving actor definition missing: {}", actor.name);
                    continue;
                };
                let anchor_transform = definition.actor.transform();
                let anchor = commands
                    .spawn((
                        Name::new(format!("Actor:{}", actor.name)),
                        anchor_transform,
                        GlobalTransform::from(anchor_transform),
                        Visibility::Inherited,
                    ))
                    .id();
                let transform = definition.primitive.transform();
                let global = GlobalTransform::from(anchor_transform).mul_transform(transform);
                let (prefix, kind) = match definition.motion {
                    MovingMotionDefinition::Spline {
                        duration,
                        start,
                        end,
                    } => (
                        "Moving",
                        PlaygroundKinematicKind::MoveSpline {
                            duration,
                            start: Vec3::from_array(start),
                            end: Vec3::from_array(end),
                        },
                    ),
                    MovingMotionDefinition::Rotation {
                        rotation_rate_yzx_radians,
                    } => (
                        "Rotating",
                        PlaygroundKinematicKind::RotateWorldTime {
                            rate_yzx_radians: bevy::math::DVec3::from_array(
                                rotation_rate_yzx_radians,
                            ),
                        },
                    ),
                };
                let visual_material = tiles_material_for_actor(
                    actor,
                    &material_library,
                    &mut tiles_materials,
                    &main_material,
                );
                let entity = commands
                    .spawn((
                        Name::new(format!("{prefix}:{}", actor.name)),
                        ChildOf(anchor),
                        transform,
                        global,
                        PlaygroundStatic,
                        PlaygroundMovingObject,
                        WorldAssetRoot(assets.load(definition.mesh_scene.clone())),
                        visual_material,
                        PlaygroundCollisionSource::new(&definition.mesh_base),
                        PlaygroundKinematic {
                            base_transform: Transform {
                                rotation: anchor_transform.rotation,
                                ..global.compute_transform()
                            },
                            kind,
                        },
                    ))
                    .id();
                commands.entity(entity).with_children(|children| {
                    children.spawn((
                        Name::new(format!("{prefix}:collider")),
                        Transform::from_translation(Vec3::from_array(definition.collider_center))
                            .with_rotation(Quat::from_array(definition.collider_rotation)),
                        PlaygroundBoxCollider {
                            half_extents: Vec3::from_array(definition.half_extents),
                        },
                    ));
                });
                spawned_moving += 1;
                continue;
            }
            _ => {}
        }

        let rotator = actor_rotator(actor);
        let mut plans: Vec<BodyPlan> = actor
            .components
            .iter()
            .filter_map(|c| body_plan_from_comp(c, actor.location, actor.scale, rotator))
            .collect();
        if plans.is_empty() {
            if let Some(p) = body_plan_from_actor_bounds(actor) {
                plans.push(p);
            }
        }
        if plans.is_empty() {
            warn!("PLAYGROUND no geometry for {} {}", actor.class, actor.name);
            skipped += 1;
            continue;
        }

        for plan in &plans {
            let blocks_visibility = visibility.get(&actor.name).copied().unwrap_or(true);
            let collision_layers = CollisionLayers::new(
                if blocks_visibility {
                    LayerMask::from(AlsCollisionLayer::Default) | AlsCollisionLayer::Visibility
                } else {
                    LayerMask::from(AlsCollisionLayer::Default)
                },
                LayerMask::ALL,
            );
            let no_shadow_caster = actor.name == CUBE_8_ACTOR;
            let visual_mat = if no_shadow_caster {
                translucent_material
                    .clone()
                    .map(PlaygroundMaterialTint::Translucent)
                    .unwrap_or_else(|| {
                        warn!(
                            "PLAYGROUND translucent material unavailable for {}",
                            actor.name
                        );
                        PlaygroundMaterialTint::Main(main_material.clone())
                    })
            } else if actor.top_color.is_some() || actor.side_color.is_some() {
                tiles_material_for_actor(
                    actor,
                    &material_library,
                    &mut tiles_materials,
                    &main_material,
                )
            } else {
                PlaygroundMaterialTint::Main(main_material.clone())
            };
            match (&plan.mesh_base, plan.obb) {
                (Some(base), true) => {
                    let scene =
                        assets.load(format!("ALSExtras/Environment/Meshes/{base}.glb#Scene0"));
                    let collision_source = PlaygroundCollisionSource::new(base);
                    let use_box_collider = collision_source.kind.is_box();
                    let root_tf = Transform::from_translation(ue_point_to_bevy(plan.loc_cm))
                        .with_rotation(plan.quat)
                        .with_scale(plan.visual_scale);
                    let offset = plan.center_offset_model / plan.visual_scale;
                    let mut visual = commands.spawn((
                        Name::new(format!("{base}:{}", actor.name)),
                        root_tf,
                        WorldAssetRoot(scene),
                        PlaygroundStatic,
                        visual_mat.clone(),
                    ));
                    if no_shadow_caster {
                        visual.insert(PlaygroundNoShadowCaster);
                    }
                    visual.insert(collision_source);
                    if use_box_collider {
                        visual.with_children(|p| {
                            p.spawn((
                                Name::new(format!("{base}:collider")),
                                Transform::from_translation(offset)
                                    .with_scale(inverse_scale(plan.visual_scale)),
                                PlaygroundBoxCollider {
                                    half_extents: plan.half_extents_model,
                                },
                            ));
                        });
                    }
                    spawned_static += 1;
                }
                (Some(base), false) => {
                    let scene =
                        assets.load(format!("ALSExtras/Environment/Meshes/{base}.glb#Scene0"));
                    let collision_source = PlaygroundCollisionSource::new(base);
                    let use_box_collider = collision_source.kind.is_box();
                    let root_tf = Transform::from_translation(ue_point_to_bevy(plan.loc_cm))
                        .with_rotation(plan.quat)
                        .with_scale(plan.visual_scale);
                    let mut visual = commands.spawn((
                        Name::new(format!("{base}:{}", actor.name)),
                        root_tf,
                        WorldAssetRoot(scene),
                        PlaygroundStatic,
                        visual_mat.clone(),
                    ));
                    if no_shadow_caster {
                        visual.insert(PlaygroundNoShadowCaster);
                    }
                    visual.insert(collision_source);
                    if use_box_collider {
                        visual.with_children(|p| {
                            p.spawn((
                                Name::new(format!("{}:aabb", actor.name)),
                                Transform::from_translation(
                                    (plan.center_world - ue_point_to_bevy(plan.loc_cm))
                                        / plan.visual_scale,
                                )
                                .with_scale(inverse_scale(plan.visual_scale)),
                                PlaygroundBoxCollider {
                                    half_extents: plan.half_extents_model,
                                },
                            ));
                        });
                    }
                    spawned_static += 1;
                }
                (None, true) => {
                    let center = plan.center_world + plan.quat * plan.center_offset_model;
                    spawn_box_visual(
                        &mut commands,
                        &mut meshes,
                        visual_mat.clone(),
                        Transform::from_translation(center).with_rotation(plan.quat),
                        plan.half_extents_model,
                        format!("{}:box", actor.name),
                        no_shadow_caster,
                        collision_layers,
                    );
                    spawned_static += 1;
                }
                (None, false) => {
                    spawn_box_visual(
                        &mut commands,
                        &mut meshes,
                        visual_mat.clone(),
                        Transform::from_translation(plan.center_world),
                        plan.half_extents_model,
                        format!("{}:box", actor.name),
                        no_shadow_caster,
                        collision_layers,
                    );
                    spawned_static += 1;
                }
            }
        }
    }

    if let Some(lighting) = &manifest.lighting {
        commands.insert_resource(imported_lighting());
        spawn_sun_light(&mut commands, lighting, manifest.sun_rotator.as_ref());
        spawn_playground_sky(
            &mut commands,
            &mut meshes,
            &mut images,
            &mut sky_materials,
            lighting,
            manifest.sun_rotator.as_ref(),
        );
    } else if let Some(rot) = &manifest.sun_rotator {
        let ray = ue_directional_light_ray_to_bevy(rot.pitch, rot.yaw);
        commands.spawn((
            Name::new("SunLight"),
            DirectionalLight {
                illuminance: 10.0,
                shadow_maps_enabled: true,
                ..default()
            },
            Transform::from_rotation(Quat::from_rotation_arc(Vec3::NEG_Z, ray)),
        ));
    }

    info!(
        "PLAYGROUND setup done actors={} static_bodies={spawned_static} moving_hooks={spawned_moving} skipped={skipped} spawn_point={spawn_point_found} landscape={spawned_landscape} text={spawned_text}",
        manifest.actors.len()
    );
}

#[derive(Component, Debug)]
pub struct PlaygroundTextLabel;

#[derive(Component, Debug)]
pub struct PlaygroundWorldTextMesh;

#[derive(Component)]
struct WorldTextMeshBuilt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaygroundTextHorizontalAlignment {
    Left,
    Center,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaygroundTextVerticalAlignment {
    Top,
    Center,
    Bottom,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct PlaygroundTextLayoutTarget {
    pub center: Vec3,
    pub width: f32,
    pub height: f32,
    pub color_linear: [f32; 4],
    pub horizontal: PlaygroundTextHorizontalAlignment,
    pub vertical: PlaygroundTextVerticalAlignment,
    pub depth_scale: f32,
    pub source_origin: Vec3,
    pub source_scale: Vec2,
    pub source_letter_spacing: f32,
    pub source_line_height: f32,
    pub shares_blue_text_frame: bool,
    pub leading_line_breaks: u32,
    pub leading_spaces: u32,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct PlaygroundTextFit {
    pub width_error: f32,
    pub height_error: f32,
    pub center_error: f32,
    pub overflow_error: f32,
    pub shared_frame_error: f32,
    pub local_min_x: f32,
    pub local_max_x: f32,
    pub inline_anchor_error: f32,
    pub inline_frame_error: f32,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct PlaygroundTextGlyphColor(pub [f32; 4]);

#[derive(Component, Debug, Clone)]
struct PlaygroundTextLineLayout {
    right_edges: Vec<f32>,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct PlaygroundTextFacing {
    pub expected_front: Vec3,
}

#[derive(Component)]
pub struct PlaygroundLandscape;

#[derive(Component)]
pub struct PlaygroundSky;

fn environment_image(width: u32, mip_count: u32, bytes: &[u8]) -> Image {
    use bevy::render::render_resource::{
        Extent3d, TextureDataOrder, TextureDimension, TextureFormat, TextureViewDescriptor,
        TextureViewDimension,
    };
    let mut image = Image::new_uninit(
        Extent3d {
            width,
            height: width,
            depth_or_array_layers: 6,
        },
        TextureDimension::D2,
        TextureFormat::Rgba16Float,
        bevy::asset::RenderAssetUsages::all(),
    );
    image.texture_descriptor.mip_level_count = mip_count;
    image.texture_view_descriptor = Some(TextureViewDescriptor {
        dimension: Some(TextureViewDimension::Cube),
        ..default()
    });
    image.data_order = TextureDataOrder::MipMajor;
    image.data = Some(bytes.to_vec());
    image
}

fn playground_camera_exposure_system(
    mut commands: Commands,
    lighting: Option<Res<PlaygroundLighting>>,
    mut curves: ResMut<Assets<AutoExposureCompensationCurve>>,
    mut images: ResMut<Assets<Image>>,
    mut tone_luts: ResMut<TonemappingLuts>,
    cameras: Query<Entity, (With<Camera3d>, Without<PlaygroundSceneExposure>)>,
) {
    let Some(lighting) = lighting else {
        return;
    };
    let exposure = &lighting.exposure;
    for entity in &cameras {
        let display: serde_json::Value = serde_json::from_str(include_str!(
            "../../../assets/ALSExtras/Environment/Sky/playground_display.runtime.json"
        ))
        .expect("Source display data must parse");
        let bloom: SourceGaussianBloom = serde_json::from_value(display["bloom"].clone())
            .expect("Source Gaussian settings must parse");
        let dimension = display["lut_dimension"].as_u64().unwrap() as u32;
        let mut lut = Image::new(
            bevy::render::render_resource::Extent3d {
                width: dimension,
                height: dimension,
                depth_or_array_layers: dimension,
            },
            bevy::render::render_resource::TextureDimension::D3,
            include_bytes!("../../../assets/ALSExtras/Environment/Sky/playground_filmic.rgba16f")
                .to_vec(),
            bevy::render::render_resource::TextureFormat::Rgba16Float,
            bevy::asset::RenderAssetUsages::all(),
        );
        lut.sampler = bevy::image::ImageSampler::linear();
        tone_luts.tony_mc_mapface = images.add(lut);
        let [minimum, maximum] = exposure.histogram_log_luminance_range;
        let compensation_curve = curves.add(
            AutoExposureCompensationCurve::from_curve(
                bevy::curve::cubic_splines::LinearSpline::new([
                    Vec2::new(minimum, exposure.compensation_ev),
                    Vec2::new(maximum, exposure.compensation_ev),
                ]),
            )
            .expect("Source exposure compensation must be continuous"),
        );
        let definition: serde_json::Value = serde_json::from_str(include_str!(
            "../../../assets/ALSExtras/Environment/Sky/playground_environment.runtime.json"
        ))
        .expect("Source environment metadata must parse");
        let diffuse = environment_image(
            definition["diffuse_resolution"].as_u64().unwrap() as u32,
            1,
            include_bytes!(
                "../../../assets/ALSExtras/Environment/Sky/playground_sky_irradiance.rgba16f"
            ),
        );
        let radiance = environment_image(
            definition["radiance_resolution"].as_u64().unwrap() as u32,
            definition["radiance_mip_count"].as_u64().unwrap() as u32,
            include_bytes!(
                "../../../assets/ALSExtras/Environment/Sky/playground_sky_radiance.rgba16f"
            ),
        );
        commands.entity(entity).insert((
            bevy::light::EnvironmentMapLight {
                diffuse_map: images.add(diffuse),
                specular_map: images.add(radiance),
                intensity: lighting.sky_light.intensity,
                ..default()
            },
            Exposure {
                ev100: exposure.unit_exposure_ev100,
            },
            AutoExposure {
                range: minimum..=maximum,
                filter: exposure.filter[0]..=exposure.filter[1],
                speed_brighten: exposure.speed_brighten,
                speed_darken: exposure.speed_darken,
                exponential_transition_distance: exposure.exponential_transition_distance,
                compensation_curve,
                ..default()
            },
            ColorGrading::with_identical_sections(
                default(),
                ColorGradingSection {
                    contrast: exposure.contrast,
                    ..default()
                },
            ),
            Tonemapping::TonyMcMapface,
            bloom,
            PlaygroundSceneExposure,
        ));
    }
}

pub fn playground_moving_object_system(
    time: Res<Time>,
    moving_curve: Res<PlaygroundMovingCurve>,
    parents: Query<&GlobalTransform>,
    mut objects: Query<
        (&mut Transform, &PlaygroundKinematic, Option<&ChildOf>),
        (
            With<PlaygroundMovingObject>,
            Without<PlaygroundPhysicsDriven>,
        ),
    >,
) {
    let t = time.elapsed_secs();
    for (mut transform, kinematic, parent) in &mut objects {
        let parent = parent.and_then(|parent| parents.get(parent.parent()).ok());
        match kinematic.kind {
            PlaygroundKinematicKind::RotateWorldTime { rate_yzx_radians } => {
                let rotation = rotating_object_rotation(rate_yzx_radians, t);
                transform.rotation = if parent.is_some() {
                    rotation
                } else {
                    kinematic.base_transform.rotation * rotation
                };
            }
            PlaygroundKinematicKind::MoveSpline {
                duration,
                start,
                end,
            } => {
                let position = moving_object_position(&moving_curve, start, end, t, duration);
                transform.translation = parent.map_or(position, |parent| {
                    parent.affine().inverse().transform_point3(position)
                });
            }
        }
    }
}

#[derive(Component, Debug, Clone)]
pub enum PlaygroundMaterialTint {
    Main(Handle<StandardMaterial>),
    Tiles(Handle<PlaygroundTilesMaterial>),
    Translucent(Handle<PlaygroundTilesMaterial>),
}

impl PlaygroundMaterialTint {
    fn role(&self) -> PlaygroundMaterialRole {
        match self {
            Self::Main(_) => PlaygroundMaterialRole::Main,
            Self::Tiles(_) => PlaygroundMaterialRole::Tiles,
            Self::Translucent(_) => PlaygroundMaterialRole::Translucent,
        }
    }
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaygroundMaterialRole {
    Main,
    Tiles,
    Translucent,
}

#[derive(Component, Debug, Clone, Copy)]
pub struct PlaygroundMaterialApplied(pub PlaygroundMaterialRole);

fn playground_material_tint_system(
    mut commands: Commands,
    roots: Query<(
        Entity,
        &PlaygroundMaterialTint,
        Has<PlaygroundLandscape>,
        Has<PlaygroundNoShadowCaster>,
    )>,
    children_query: Query<&Children>,
    standard_materials: Query<(), With<MeshMaterial3d<StandardMaterial>>>,
) {
    fn walk_and_apply(
        entity: Entity,
        tint: &PlaygroundMaterialTint,
        commands: &mut Commands,
        children_query: &Query<&Children>,
        standard_materials: &Query<(), With<MeshMaterial3d<StandardMaterial>>>,
        landscape: bool,
        no_shadow_caster: bool,
        found: &mut u32,
    ) {
        if let Ok(children) = children_query.get(entity) {
            for child in children.iter() {
                if standard_materials.contains(child) {
                    let applied = PlaygroundMaterialApplied(tint.role());
                    match tint {
                        PlaygroundMaterialTint::Main(handle) => {
                            commands
                                .entity(child)
                                .insert((MeshMaterial3d(handle.clone()), applied));
                        }
                        PlaygroundMaterialTint::Tiles(handle)
                        | PlaygroundMaterialTint::Translucent(handle) => {
                            commands
                                .entity(child)
                                .remove::<MeshMaterial3d<StandardMaterial>>();
                            commands.entity(child).insert((
                                MeshMaterial3d(handle.clone()),
                                applied,
                                NoFrustumCulling,
                            ));
                        }
                    }
                    if landscape {
                        commands
                            .entity(child)
                            .insert(PlaygroundLandscapeTilesMaterial);
                    }
                    if no_shadow_caster {
                        commands.entity(child).insert(NotShadowCaster);
                    }
                    *found += 1;
                }
                walk_and_apply(
                    child,
                    tint,
                    commands,
                    children_query,
                    standard_materials,
                    landscape,
                    no_shadow_caster,
                    found,
                );
            }
        }
    }

    for (root, tint, landscape, no_shadow_caster) in &roots {
        let mut found = 0u32;
        walk_and_apply(
            root,
            tint,
            &mut commands,
            &children_query,
            &standard_materials,
            landscape,
            no_shadow_caster,
            &mut found,
        );
        if found > 0 {
            commands.entity(root).remove::<PlaygroundMaterialTint>();
        }
    }
}

#[derive(Resource, Debug, Clone)]
pub struct PlaygroundTerrainData {
    pub origin_ue: [f32; 2],
    pub step_cm: f32,
    pub rows_ue_y: usize,
    pub cols_ue_x: usize,
    pub heights_cm: Vec<Vec<f32>>,
}

pub fn spawn_landscape(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    location_ue: [f32; 3],
) {
    let pos = ue_point_to_bevy(Vec3::from(location_ue));
    let mat = materials.add(StandardMaterial {
        base_color: Color::srgb(0.25, 0.5, 0.125),
        perceptual_roughness: 0.95,
        ..default()
    });
    let data = commands
        .spawn((Name::new("Landscape1"), Transform::from_translation(pos)))
        .id();
    spawn_landscape_grid(commands, meshes, materials, mat, data, None, pos);
}

#[allow(clippy::too_many_arguments)]
fn spawn_landscape_grid(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    _materials: &mut Assets<StandardMaterial>,
    mat: Handle<StandardMaterial>,
    entity: Entity,
    terrain: Option<&PlaygroundTerrainData>,
    pos: Vec3,
) {
    let rows = terrain.map(|t| t.rows_ue_y).unwrap_or(64);
    let cols = terrain.map(|t| t.cols_ue_x).unwrap_or(64);
    let step_m = terrain
        .map(|t| (t.step_cm * CM_TO_M) as f32)
        .unwrap_or(120.0 / 64.0);
    let height_at = |ri: usize, ci: usize| -> f32 {
        match terrain {
            Some(t) => {
                let r = ri.min(t.heights_cm.len() - 1);
                let row = &t.heights_cm[r];
                let c = ci.min(row.len() - 1);
                row[c] * CM_TO_M
            }
            None => terrain_height(
                -60.0 + ci as f32 * 120.0 / 64.0,
                -60.0 + ri as f32 * 120.0 / 64.0,
            ),
        }
    };
    let mut positions = Vec::with_capacity(rows * cols);
    let mut uvs = Vec::with_capacity(rows * cols);
    for ri in 0..rows {
        for ci in 0..cols {
            let lx = ci as f32 * step_m;
            let lz = ri as f32 * step_m;
            let h = height_at(ri, ci);
            positions.push([lx, h, lz]);
            uvs.push([
                ci as f32 / (cols - 1).max(1) as f32,
                ri as f32 / (rows - 1).max(1) as f32,
            ]);
        }
    }
    let mut indices: Vec<u32> = Vec::with_capacity((rows - 1) * (cols - 1) * 6);
    for ri in 0..rows - 1 {
        for ci in 0..cols - 1 {
            let a = (ri * cols + ci) as u32;
            let b = a + 1;
            let c = ((ri + 1) * cols + ci) as u32;
            let d = c + 1;
            indices.extend_from_slice(&[a, c, b, b, c, d]);
        }
    }
    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions.clone());
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    let per_tri = 3usize;
    let tri_count = indices.len() / 3;
    let mut normals = vec![[0.0f32, 1.0, 0.0]; positions.len()];
    for ti in 0..tri_count {
        let ia = indices[ti * per_tri] as usize;
        let ib = indices[ti * per_tri + 1] as usize;
        let ic = indices[ti * per_tri + 2] as usize;
        let pa = Vec3::from_array(positions[ia]);
        let pb = Vec3::from_array(positions[ib]);
        let pc = Vec3::from_array(positions[ic]);
        let n = (pb - pa).cross(pc - pa).normalize_or_zero();
        for idx in [ia, ib, ic] {
            let existing = Vec3::from_array(normals[idx]);
            normals[idx] = (existing + n).normalize_or_zero().to_array();
        }
    }
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));

    commands.entity(entity).insert((
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(mat),
        PlaygroundLandscape,
        Transform::from_translation(pos),
    ));
}

pub fn terrain_height(x: f32, z: f32) -> f32 {
    let h = 0.35 * ((x * 0.06).sin() * (z * 0.05).cos())
        + 0.20 * ((x * 0.12 + z * 0.09).sin() + (z * 0.15).sin())
        + 0.08 * ((x * 0.3).sin() * (z * 0.25).cos());
    h.max(-0.5)
}

const TEXT_LAYOUT_FONT_SIZE: f32 = 100.0;
fn text_horizontal_alignment(value: Option<&str>) -> PlaygroundTextHorizontalAlignment {
    match value {
        Some("EHTA_Left") => PlaygroundTextHorizontalAlignment::Left,
        Some("EHTA_Right") => PlaygroundTextHorizontalAlignment::Right,
        _ => PlaygroundTextHorizontalAlignment::Center,
    }
}

fn text_vertical_alignment(value: Option<&str>) -> PlaygroundTextVerticalAlignment {
    match value {
        Some("EVRTA_TextTop") => PlaygroundTextVerticalAlignment::Top,
        Some("EVRTA_TextCenter") => PlaygroundTextVerticalAlignment::Center,
        _ => PlaygroundTextVerticalAlignment::Bottom,
    }
}

fn text_anchor(
    horizontal: PlaygroundTextHorizontalAlignment,
    vertical: PlaygroundTextVerticalAlignment,
) -> Anchor {
    match (horizontal, vertical) {
        (PlaygroundTextHorizontalAlignment::Left, PlaygroundTextVerticalAlignment::Top) => {
            Anchor::TOP_LEFT
        }
        (PlaygroundTextHorizontalAlignment::Left, PlaygroundTextVerticalAlignment::Center) => {
            Anchor::CENTER_LEFT
        }
        (PlaygroundTextHorizontalAlignment::Left, PlaygroundTextVerticalAlignment::Bottom) => {
            Anchor::BOTTOM_LEFT
        }
        (PlaygroundTextHorizontalAlignment::Center, PlaygroundTextVerticalAlignment::Top) => {
            Anchor::TOP_CENTER
        }
        (PlaygroundTextHorizontalAlignment::Center, PlaygroundTextVerticalAlignment::Center) => {
            Anchor::CENTER
        }
        (PlaygroundTextHorizontalAlignment::Center, PlaygroundTextVerticalAlignment::Bottom) => {
            Anchor::BOTTOM_CENTER
        }
        (PlaygroundTextHorizontalAlignment::Right, PlaygroundTextVerticalAlignment::Top) => {
            Anchor::TOP_RIGHT
        }
        (PlaygroundTextHorizontalAlignment::Right, PlaygroundTextVerticalAlignment::Center) => {
            Anchor::CENTER_RIGHT
        }
        (PlaygroundTextHorizontalAlignment::Right, PlaygroundTextVerticalAlignment::Bottom) => {
            Anchor::BOTTOM_RIGHT
        }
    }
}

fn text_justify(alignment: PlaygroundTextHorizontalAlignment) -> Justify {
    match alignment {
        PlaygroundTextHorizontalAlignment::Left => Justify::Left,
        PlaygroundTextHorizontalAlignment::Center => Justify::Center,
        PlaygroundTextHorizontalAlignment::Right => Justify::Right,
    }
}

fn text_color_linear(color: [f32; 4]) -> Color {
    Color::linear_rgba(color[0], color[1], color[2], color[3])
}

fn text_target_bounds(actor: &PlaygroundActor, rotation: Quat) -> (Vec3, Vec2) {
    let fallback_center = ue_point_to_bevy(Vec3::from(actor.location));
    let fallback_size = actor.text_world_size.unwrap_or(30.0) * CM_TO_M;
    let Some(extent_cm) = actor.bounds_extent else {
        return (fallback_center, Vec2::splat(fallback_size));
    };
    let center = actor
        .bounds_origin
        .map(Vec3::from)
        .map(ue_point_to_bevy)
        .unwrap_or(fallback_center);
    let extents = ue_extents_to_bevy(Vec3::from(extent_cm));
    let width = 2.0 * extents.dot((rotation * Vec3::X).abs());
    let height = 2.0 * extents.dot((rotation * Vec3::Y).abs());
    (center, Vec2::new(width.max(0.001), height.max(0.001)))
}

fn shares_blue_text_frame(actor_name: &str) -> bool {
    actor_name == "TextRenderActor3_1"
}

fn leading_text_layout_whitespace(text: &str) -> (u32, u32) {
    let normalized = text.replace("\r\n", "\n");
    let mut line_breaks = 0u32;
    let mut chars = normalized.chars().peekable();
    while matches!(chars.peek(), Some(&'\n')) {
        chars.next();
        line_breaks += 1;
    }
    let spaces = chars.take_while(|character| *character == ' ').count() as u32;
    (line_breaks, spaces)
}

fn ue_text_mesh_rotation_to_bevy_quat(pitch_deg: f32, yaw_deg: f32, roll_deg: f32) -> Quat {
    ue_rotator_to_bevy_quat(pitch_deg, yaw_deg, roll_deg)
        * Quat::from_rotation_y(std::f32::consts::FRAC_PI_2)
}

fn spawn_text_label(commands: &mut Commands, actor: &PlaygroundActor) {
    let text_str = actor.text.clone().unwrap_or_default();
    if text_str.is_empty() {
        return;
    }
    let color_arr = actor.text_color.unwrap_or([1.0, 1.0, 1.0, 1.0]);
    let rotator = actor_rotator(actor);
    let source_rotation = ue_rotator_to_bevy_quat(rotator.0, rotator.1, rotator.2);
    let rotation = ue_text_mesh_rotation_to_bevy_quat(rotator.0, rotator.1, rotator.2);
    let world_size_cm = actor.text_world_size.unwrap_or(30.0).max(0.001);
    let world_size_m = world_size_cm * CM_TO_M;
    let x_scale = actor.text_x_scale.unwrap_or(1.0);
    let y_scale = actor.text_y_scale.unwrap_or(1.0);
    let horizontal = text_horizontal_alignment(actor.text_horizontal_alignment.as_deref());
    let vertical = text_vertical_alignment(actor.text_vertical_alignment.as_deref());
    let anchor = text_anchor(horizontal, vertical);
    let (center, size) = text_target_bounds(actor, rotation);
    let source_origin = ue_point_to_bevy(Vec3::from(actor.location));
    let source_scale = Vec2::new(
        world_size_m * x_scale / TEXT_LAYOUT_FONT_SIZE,
        world_size_m * y_scale / TEXT_LAYOUT_FONT_SIZE,
    );
    let horizontal_spacing =
        actor.text_horizontal_spacing_adjust.unwrap_or(0.0) * x_scale * TEXT_LAYOUT_FONT_SIZE
            / world_size_cm;
    let line_height = TEXT_LAYOUT_FONT_SIZE
        + actor.text_vertical_spacing_adjust.unwrap_or(0.0) * y_scale * TEXT_LAYOUT_FONT_SIZE
            / world_size_cm;
    let (leading_line_breaks, leading_spaces) = leading_text_layout_whitespace(&text_str);
    commands.spawn((
        Name::new(format!("Text:{}", actor.name)),
        Text2d::new(text_str),
        TextLayout::justify(text_justify(horizontal)),
        anchor,
        TextFont::from_font_size(TEXT_LAYOUT_FONT_SIZE),
        LetterSpacing::Px(horizontal_spacing),
        LineHeight::Px(line_height),
        TextColor(text_color_linear(color_arr)),
        Transform::from_translation(center)
            .with_rotation(rotation)
            .with_scale(Vec3::new(
                world_size_m * x_scale / TEXT_LAYOUT_FONT_SIZE,
                world_size_m * y_scale / TEXT_LAYOUT_FONT_SIZE,
                world_size_m / TEXT_LAYOUT_FONT_SIZE,
            )),
        PlaygroundTextLabel,
        PlaygroundTextFacing {
            expected_front: source_rotation.mul_vec3(Vec3::X),
        },
        PlaygroundTextLayoutTarget {
            center,
            width: size.x,
            height: size.y,
            color_linear: color_arr,
            horizontal,
            vertical,
            depth_scale: world_size_m / TEXT_LAYOUT_FONT_SIZE,
            source_origin,
            source_scale,
            source_letter_spacing: horizontal_spacing,
            source_line_height: line_height,
            shares_blue_text_frame: shares_blue_text_frame(&actor.name),
            leading_line_breaks,
            leading_spaces,
        },
    ));
}

#[derive(Default)]
struct WorldTextMeshBatch {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

const WORLD_TEXT_QUAD_INDICES: [u32; 6] = [0, 2, 1, 0, 3, 2];

fn world_text_mesh_system(
    mut commands: Commands,
    mut sources: Query<
        (
            Entity,
            &Text2d,
            &TextLayoutInfo,
            &TextColor,
            &Anchor,
            &PlaygroundTextLayoutTarget,
            &mut Transform,
        ),
        (With<PlaygroundTextLabel>, Without<WorldTextMeshBuilt>),
    >,
    reference_frames: Query<
        (
            &Name,
            &Transform,
            &PlaygroundTextFit,
            &PlaygroundTextLineLayout,
        ),
        (With<PlaygroundTextLabel>, With<WorldTextMeshBuilt>),
    >,
    mut images: ResMut<Assets<Image>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    for (entity, text, layout, text_color, anchor, target, mut transform) in &mut sources {
        if layout.glyphs.is_empty() || layout.scale_factor <= 0.0 {
            continue;
        }

        let inverse_scale_factor = layout.scale_factor.recip();
        let top_left = (Anchor::TOP_LEFT.0 - anchor.as_vec()) * layout.size;
        let mut batches: HashMap<AssetId<Image>, WorldTextMeshBatch> = HashMap::new();
        let mut geometry_min = Vec2::splat(f32::INFINITY);
        let mut geometry_max = Vec2::splat(f32::NEG_INFINITY);
        let mut line_rights: Vec<(u32, f32)> = Vec::new();
        let mut layout_characters = text
            .0
            .chars()
            .filter(|character| !matches!(character, '\r' | '\n'));

        for glyph in &layout.glyphs {
            let Some(character) = layout_characters.next() else {
                continue;
            };
            if character.is_whitespace() {
                continue;
            }
            let Some(image) = images.get(glyph.atlas_info.texture) else {
                continue;
            };
            let image_size = Vec2::new(
                image.texture_descriptor.size.width as f32,
                image.texture_descriptor.size.height as f32,
            );
            if image_size.x <= 0.0 || image_size.y <= 0.0 {
                continue;
            }

            let glyph_size = glyph.atlas_info.rect.size();
            let glyph_min = glyph.position - glyph_size * 0.5;
            let glyph_max = glyph.position + glyph_size * 0.5;
            let left = top_left.x + glyph_min.x * inverse_scale_factor;
            let right = top_left.x + glyph_max.x * inverse_scale_factor;
            let top = top_left.y - glyph_min.y * inverse_scale_factor;
            let bottom = top_left.y - glyph_max.y * inverse_scale_factor;
            let uv_min = glyph.atlas_info.rect.min / image_size;
            let uv_max = glyph.atlas_info.rect.max / image_size;
            let batch = batches.entry(glyph.atlas_info.texture).or_default();
            let base = batch.positions.len() as u32;

            batch.positions.extend_from_slice(&[
                [left, top, 1.0],
                [right, top, 1.0],
                [right, bottom, 1.0],
                [left, bottom, 1.0],
            ]);
            batch.normals.extend_from_slice(&[
                [0.0, 0.0, 1.0],
                [0.0, 0.0, 1.0],
                [0.0, 0.0, 1.0],
                [0.0, 0.0, 1.0],
            ]);
            batch.uvs.extend_from_slice(&[
                uv_min.to_array(),
                [uv_max.x, uv_min.y],
                uv_max.to_array(),
                [uv_min.x, uv_max.y],
            ]);
            let indices = WORLD_TEXT_QUAD_INDICES.map(|index| base + index);
            batch.indices.extend_from_slice(&indices);
            geometry_min = geometry_min.min(Vec2::new(left.min(right), bottom.min(top)));
            geometry_max = geometry_max.max(Vec2::new(left.max(right), bottom.max(top)));
            if let Some((_, line_right)) = line_rights
                .iter_mut()
                .find(|(line_index, _)| *line_index == glyph.line_index)
            {
                *line_right = line_right.max(right);
            } else {
                line_rights.push((glyph.line_index, right));
            }
        }

        let geometry_size = geometry_max - geometry_min;
        if batches.is_empty() || geometry_size.x <= 0.0 || geometry_size.y <= 0.0 {
            continue;
        }

        line_rights.sort_by(|left, right| left.0.cmp(&right.0));
        let line_right_edges = line_rights
            .into_iter()
            .map(|(_, right)| right)
            .collect::<Vec<_>>();
        let (fit_size, local_center, shared_frame, inline_target_left, inline_frame_x) = if target
            .shares_blue_text_frame
        {
            let Some((_, reference, reference_fit, reference_lines)) = reference_frames
                .iter()
                .find(|(name, _, _, _)| name.as_str() == "Text:TextRenderActor2_4")
            else {
                continue;
            };
            let Some(reference_line_right) = reference_lines
                .right_edges
                .get(target.leading_line_breaks as usize)
            else {
                continue;
            };
            if target.leading_spaces == 0 {
                continue;
            }
            let source_space_advance = (geometry_min.x - top_left.x) / target.leading_spaces as f32;
            (
                geometry_size,
                Vec3::ZERO,
                Some(reference.clone()),
                Some(*reference_line_right + source_space_advance),
                Some((reference_fit.local_min_x, reference_fit.local_max_x)),
            )
        } else {
            (
                geometry_size,
                Vec3::new(
                    (geometry_min.x + geometry_max.x) * 0.5,
                    (geometry_min.y + geometry_max.y) * 0.5,
                    1.0,
                ),
                None,
                None,
                None,
            )
        };
        if let Some(target_left) = inline_target_left {
            let whitespace_advance_adjustment = target_left - geometry_min.x;
            for batch in batches.values_mut() {
                for position in &mut batch.positions {
                    position[0] += whitespace_advance_adjustment;
                }
            }
        }
        let actual_min_x = batches
            .values()
            .flat_map(|batch| batch.positions.iter().map(|position| position[0]))
            .fold(f32::INFINITY, f32::min);
        let actual_max_x = batches
            .values()
            .flat_map(|batch| batch.positions.iter().map(|position| position[0]))
            .fold(f32::NEG_INFINITY, f32::max);
        let inline_anchor_error = inline_target_left
            .map(|target_left| (actual_min_x - target_left).abs())
            .unwrap_or(0.0);
        let inline_frame_error = inline_frame_x
            .map(|(frame_min_x, frame_max_x)| {
                (frame_min_x - actual_min_x)
                    .max(0.0)
                    .max((actual_max_x - frame_max_x).max(0.0))
            })
            .unwrap_or(0.0);
        let (fitted_center, fitted_width, fitted_height, shared_frame_error) =
            if let Some(reference) = shared_frame {
                *transform = reference.clone();
                (
                    target.center,
                    fit_size.x * transform.scale.x,
                    fit_size.y * transform.scale.y,
                    transform.translation.distance(reference.translation).max(
                        transform
                            .scale
                            .distance(reference.scale)
                            .max(1.0 - transform.rotation.dot(reference.rotation).abs()),
                    ),
                )
            } else {
                let scale = Vec3::new(
                    target.width / geometry_size.x,
                    target.height / geometry_size.y,
                    target.depth_scale,
                );
                transform.scale = scale;
                transform.translation =
                    target.center - transform.rotation.mul_vec3(local_center * transform.scale);
                let fitted_center = transform.translation
                    + transform.rotation.mul_vec3(local_center * transform.scale);
                (
                    fitted_center,
                    fit_size.x * transform.scale.x,
                    fit_size.y * transform.scale.y,
                    0.0,
                )
            };

        commands.entity(entity).with_children(|parent| {
            for (texture, batch) in batches {
                let Some(texture_handle) = images.get_strong_handle(texture) else {
                    continue;
                };
                let mut mesh = Mesh::new(
                    bevy::render::mesh::PrimitiveTopology::TriangleList,
                    bevy::asset::RenderAssetUsages::default(),
                );
                mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, batch.positions);
                mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, batch.normals);
                mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, batch.uvs);
                mesh.insert_indices(bevy::render::mesh::Indices::U32(batch.indices));
                let material = materials.add(StandardMaterial {
                    base_color: text_color.0,
                    base_color_texture: Some(texture_handle),
                    alpha_mode: AlphaMode::Mask(0.1),
                    cull_mode: None,
                    perceptual_roughness: 1.0,
                    ..default()
                });
                parent.spawn((
                    Name::new("WorldTextGlyphMesh"),
                    Mesh3d(meshes.add(mesh)),
                    MeshMaterial3d(material),
                    NotShadowCaster,
                    Transform::IDENTITY,
                    PlaygroundWorldTextMesh,
                    PlaygroundTextGlyphColor(target.color_linear),
                ));
            }
        });
        commands.entity(entity).insert((
            WorldTextMeshBuilt,
            PlaygroundTextFit {
                width_error: (fitted_width - target.width).abs(),
                height_error: (fitted_height - target.height).abs(),
                center_error: fitted_center.distance(target.center),
                overflow_error: (fitted_width - target.width)
                    .max(0.0)
                    .max((fitted_height - target.height).max(0.0)),
                shared_frame_error,
                local_min_x: actual_min_x,
                local_max_x: actual_max_x,
                inline_anchor_error,
                inline_frame_error,
            },
            PlaygroundTextLineLayout {
                right_edges: line_right_edges,
            },
        ));
        commands.entity(entity).remove::<Text2d>();
    }
}

fn resolved_sun_ray(lighting: &SceneLighting, sun_rotator: Option<&PlaygroundRotator>) -> Vec3 {
    sun_rotator
        .map(|rot| ue_directional_light_ray_to_bevy(rot.pitch, rot.yaw))
        .or_else(|| {
            lighting
                .sun_direction
                .map(Vec3::from)
                .map(Vec3::normalize_or_zero)
        })
        .unwrap_or_else(|| Vec3::new(-0.4, -0.8, 0.3).normalize())
}

fn spawn_sun_light(
    commands: &mut Commands,
    lighting: &SceneLighting,
    sun_rotator: Option<&PlaygroundRotator>,
) {
    let ray = resolved_sun_ray(lighting, sun_rotator);
    let rotation = Quat::from_rotation_arc(Vec3::NEG_Z, ray);
    let intensity = lighting.sun_intensity.unwrap_or(10.0);
    let color_arr = lighting.sun_color.unwrap_or([1.0, 0.96, 0.9, 1.0]);
    commands.spawn((
        Name::new("SunLight"),
        DirectionalLight {
            color: Color::srgba(color_arr[0], color_arr[1], color_arr[2], color_arr[3]),
            illuminance: intensity,
            shadow_maps_enabled: true,
            contact_shadows_enabled: false,
            shadow_depth_bias: PLAYGROUND_SHADOW_DEPTH_BIAS,
            shadow_normal_bias: PLAYGROUND_SHADOW_NORMAL_BIAS,
            ..default()
        },
        CascadeShadowConfigBuilder {
            num_cascades: 4,
            maximum_distance: 400.0,
            ..default()
        }
        .build(),
        Transform::default().with_rotation(rotation),
    ));
    commands.insert_resource(GlobalAmbientLight {
        brightness: 0.0,
        ..default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actual_playground_boxes_preserve_source_visibility_responses() {
        use avian3d::prelude::{PhysicsPlugins, ShapeCastConfig, SpatialQueryFilter};
        use bevy::ecs::system::RunSystemOnce;
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            PhysicsPlugins::new(PostUpdate),
        ))
        .init_resource::<Assets<Mesh>>()
        .add_message::<bevy::asset::AssetEvent<Mesh>>()
        .add_systems(Update, playground_box_physics_bridge);
        app.finish();
        let ids = [
            "StaticMeshActor_UAID_94B86D58D760231001_1996299222",
            "StaticMeshActor_UAID_94B86D58D760231001_1287870219",
            "StaticMeshActor_UAID_94B86D58D760231001_1363458220",
        ];
        let visibility: HashMap<String, bool> = serde_json::from_str(include_str!(
            "../../../assets/ALSExtras/Levels/L_Als_Playground/visibility.runtime.json"
        ))
        .unwrap();
        let bodies = app
            .world_mut()
            .run_system_once(
                move |mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>| {
                    ids.into_iter()
                        .chain(["S_Arch_01"])
                        .enumerate()
                        .map(|(index, name)| {
                            let layers = CollisionLayers::new(
                                if visibility[name] {
                                    LayerMask::from(AlsCollisionLayer::Default)
                                        | AlsCollisionLayer::Visibility
                                } else {
                                    LayerMask::from(AlsCollisionLayer::Default)
                                },
                                LayerMask::ALL,
                            );
                            spawn_box_visual(
                                &mut commands,
                                &mut meshes,
                                PlaygroundMaterialTint::Main(Handle::default()),
                                Transform::from_xyz(index as f32 * 3.0, 0.0, 0.0),
                                Vec3::splat(0.5),
                                format!("{name}:aabb"),
                                false,
                                layers,
                            )
                        })
                        .collect::<Vec<_>>()
                },
            )
            .unwrap();
        app.update();
        app.update();
        app.world_mut().run_system_once(move |spatial:bevy_als::extensions::SceneQuery| {
            let shape=Collider::sphere(0.15);
            let config=ShapeCastConfig::from_max_distance(3.0);
            for (index,body) in bodies.iter().enumerate() {
                let start=Vec3::new(index as f32*3.0,0.0,-1.5);
                let visibility=spatial.cast_shape(&shape,start,Quat::IDENTITY,Dir3::Z,&config,
                    &SpatialQueryFilter::from_mask(AlsCollisionLayer::Visibility));
                let physical=spatial.cast_shape(&shape,start,Quat::IDENTITY,Dir3::Z,&config,
                    &SpatialQueryFilter::from_mask(AlsCollisionLayer::Default));
                assert_eq!(physical.map(|hit|hit.entity),Some(*body),"Source physical block was lost");
                assert_eq!(visibility.is_some(),index==3,"Actual Playground box ignores its source Visibility response: index={index}");
            }
        }).unwrap();
    }

    #[test]
    fn playground_camera_preserves_source_hdr_exposure_contract() {
        use bevy::ecs::system::RunSystemOnce;
        use bevy::post_process::auto_exposure::AutoExposure;
        let mut app = App::new();
        app.init_resource::<Assets<AutoExposureCompensationCurve>>()
            .init_resource::<Assets<Image>>()
            .insert_resource(TonemappingLuts {
                blender_filmic: default(),
                agx: default(),
                tony_mc_mapface: default(),
            })
            .insert_resource(imported_lighting());
        let camera = app.world_mut().spawn(Camera3d::default()).id();
        app.world_mut()
            .run_system_once(playground_camera_exposure_system)
            .unwrap();
        assert!(
            app.world().get::<bevy::camera::Hdr>(camera).is_some(),
            "Source sky and lit geometry require one HDR display chain"
        );
        let exposure = app
            .world()
            .get::<AutoExposure>(camera)
            .expect("Source camera requires histogram exposure");
        assert_eq!(exposure.range, -10.0..=20.0);
        assert_eq!(exposure.filter, 0.1..=0.9);
        assert_eq!(exposure.speed_brighten, 3.0);
        assert_eq!(exposure.speed_darken, 1.0);
        let curves = app
            .world()
            .resource::<Assets<AutoExposureCompensationCurve>>();
        let curve = curves
            .get(&exposure.compensation_curve)
            .expect("Source compensation must reach its reference asset");
        let bevy::reflect::ReflectRef::Struct(fields) = curve.reflect_ref() else {
            panic!("Native compensation must be reflected")
        };
        for name in ["min_compensation", "max_compensation"] {
            let value = *fields
                .field(name)
                .unwrap()
                .try_downcast_ref::<f32>()
                .unwrap();
            assert!(
                (value - (1.0 + 0.18_f32.log2())).abs() < 1e-6,
                "Source gray and bias were dropped: {name}={value}"
            );
        }
        assert!(
            (app.world().get::<Exposure>(camera).unwrap().exposure() - 1.0).abs() < 1e-6,
            "Source histogram receives scene luminance without the old fixed exposure scale"
        );
        assert!(
            app.world()
                .get::<ColorGrading>(camera)
                .unwrap()
                .all_sections()
                .all(|section| section.contrast == 1.0)
        );
        assert!(
            matches!(
                app.world().get::<Tonemapping>(camera),
                Some(Tonemapping::TonyMcMapface)
            ),
            "Source Filmic must use the imported LUT response"
        );
        let environment = app
            .world()
            .get::<bevy::light::EnvironmentMapLight>(camera)
            .expect("Source SH environment must be loaded directly without random generation");
        assert!(
            app.world()
                .get::<bevy::light::GeneratedEnvironmentMapLight>(camera)
                .is_none(),
            "Random reference filtering would replace the source SH response"
        );
        assert_eq!(environment.intensity, 2.0);
        assert_eq!(environment.rotation, Quat::IDENTITY);
        let images = app.world().resource::<Assets<Image>>();
        let diffuse = images.get(&environment.diffuse_map).unwrap();
        let specular = images.get(&environment.specular_map).unwrap();
        assert_eq!(diffuse.texture_descriptor.size.width, 32);
        assert_eq!(diffuse.texture_descriptor.size.depth_or_array_layers, 6);
        assert_eq!(
            diffuse.texture_descriptor.format,
            bevy::render::render_resource::TextureFormat::Rgba16Float
        );
        assert_eq!(
            diffuse.texture_view_descriptor.as_ref().unwrap().dimension,
            Some(bevy::render::render_resource::TextureViewDimension::Cube)
        );
        assert_eq!(
            diffuse.data.as_deref().unwrap(),
            include_bytes!(
                "../../../assets/ALSExtras/Environment/Sky/playground_sky_irradiance.rgba16f"
            )
        );
        assert_eq!(specular.texture_descriptor.size.width, 128);
        assert_eq!(specular.texture_descriptor.mip_level_count, 6);
        assert_eq!(
            specular.data_order,
            bevy::render::render_resource::TextureDataOrder::MipMajor
        );
        assert_eq!(
            specular.data.as_deref().unwrap(),
            include_bytes!(
                "../../../assets/ALSExtras/Environment/Sky/playground_sky_radiance.rgba16f"
            )
        );
        let image_count = app.world().resource::<Assets<Image>>().len();
        assert!(
            app.world()
                .get::<bevy::post_process::bloom::Bloom>(camera)
                .is_none(),
            "Native HDR compression must not replace source linear Gaussian bloom"
        );
        let bloom = app
            .world()
            .get::<SourceGaussianBloom>(camera)
            .expect("Source Gaussian bloom is missing");
        let gain: f32 = bloom
            .tints
            .iter()
            .map(|color| color.x * bloom.intensity / 6.0)
            .sum();
        assert!((gain - 0.08944875).abs() < 1e-6);
        let luts = app.world().resource::<TonemappingLuts>();
        let lut = images.get(&luts.tony_mc_mapface).unwrap();
        assert_eq!(
            lut.texture_descriptor.dimension,
            bevy::render::render_resource::TextureDimension::D3
        );
        assert_eq!(lut.texture_descriptor.size.width, 48);
        assert_eq!(
            lut.data.as_deref().unwrap(),
            include_bytes!("../../../assets/ALSExtras/Environment/Sky/playground_filmic.rgba16f")
        );
        app.world_mut()
            .run_system_once(playground_camera_exposure_system)
            .unwrap();
        assert_eq!(
            app.world().resource::<Assets<Image>>().len(),
            image_count,
            "Source non-realtime capture must not rebuild every frame"
        );
    }

    #[test]
    fn actual_playground_sun_preserves_current_source_lux() {
        use bevy::ecs::system::RunSystemOnce;
        let manifest: PlaygroundManifest = ron::from_str(include_str!(
            "../../../assets/ALSExtras/Levels/L_Als_Playground/playground_scene.ron"
        ))
        .unwrap();
        let source: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/render/directional_light.json"
        ))
        .unwrap();
        let mut app = App::new();
        app.world_mut()
            .run_system_once(move |mut commands: Commands| {
                spawn_sun_light(
                    &mut commands,
                    manifest.lighting.as_ref().unwrap(),
                    manifest.sun_rotator.as_ref(),
                );
            })
            .unwrap();
        let mut query = app.world_mut().query::<&DirectionalLight>();
        let light = query.single(app.world()).unwrap();
        assert_eq!(
            app.world().resource::<GlobalAmbientLight>().brightness,
            0.0,
            "Source captured sky replaces the legacy constant ambient light"
        );
        assert!(
            (light.illuminance - source["intensity_lux"].as_f64().unwrap() as f32).abs() < 1e-6,
            "Actual Playground light differs from current source lux: {}",
            light.illuminance
        );
    }

    #[test]
    fn tiles_texture_uvs_follow_the_source_shared_tiling_input() {
        let library = PlaygroundMaterialLibrary {
            tiles_grid: Some(Handle::default()),
            ..default()
        };
        for tiling in [0.5, 2.0, 5.0] {
            let material =
                make_tiles_material(&library, [1.0; 3], [0.5; 3], [0.25; 3], [0.75; 3], tiling)
                    .unwrap();
            assert_eq!(
                material
                    .base
                    .uv_transform
                    .transform_point2(Vec2::new(0.2, 0.4)),
                Vec2::new(0.2, 0.4) * tiling
            );
            assert_eq!(material.extension.settings.tiling_and_mode.x, tiling);
        }
    }

    fn map_dir(v: Vec3) -> Vec3 {
        ue_point_to_bevy(v) * 100.0
    }

    fn assert_vec3_eq(a: Vec3, b: Vec3, eps: f32) {
        assert!(a.distance(b) < eps, "vec mismatch left={a:?} right={b:?}");
    }

    fn assert_quat_eq(a: Quat, b: Quat, eps: f32) {
        let d = a.dot(b).abs();
        assert!(
            (d - 1.0).abs() < eps,
            "quat mismatch left={a:?} right={b:?} dot={d}"
        );
    }

    fn ue_source_rotator_quat(pitch_deg: f32, yaw_deg: f32, roll_deg: f32) -> Quat {
        let (sp, cp) = (pitch_deg.to_radians() * 0.5).sin_cos();
        let (sy, cy) = (yaw_deg.to_radians() * 0.5).sin_cos();
        let (sr, cr) = (roll_deg.to_radians() * 0.5).sin_cos();
        Quat::from_xyzw(
            cr * sp * sy - sr * cp * cy,
            -cr * sp * cy - sr * cp * sy,
            cr * cp * sy - sr * sp * cy,
            cr * cp * cy + sr * sp * sy,
        )
    }

    #[test]
    fn point_mapping_swaps_yz_and_scales() {
        assert_vec3_eq(
            ue_point_to_bevy(Vec3::new(100.0, 200.0, 300.0)),
            Vec3::new(1.0, 3.0, 2.0),
            1e-5,
        );
    }

    #[test]
    fn extents_mapping_matches_point_mapping() {
        assert_vec3_eq(
            ue_extents_to_bevy(Vec3::new(25.0, 1000.0, 100.0)),
            ue_point_to_bevy(Vec3::new(25.0, 1000.0, 100.0)),
            1e-6,
        );
    }

    #[test]
    fn directional_light_ray_matches_ue_forward() {
        assert_vec3_eq(
            ue_directional_light_ray_to_bevy(-45.0, 45.0),
            Vec3::new(0.5, -0.70710677, 0.5),
            1e-5,
        );
    }

    #[test]
    fn directional_light_shadow_bias_uses_bevy_calibration() {
        assert_eq!(
            PLAYGROUND_SHADOW_DEPTH_BIAS,
            DirectionalLight::DEFAULT_SHADOW_DEPTH_BIAS
        );
        assert_eq!(
            PLAYGROUND_SHADOW_NORMAL_BIAS,
            DirectionalLight::DEFAULT_SHADOW_NORMAL_BIAS
        );
    }

    #[test]
    fn quat_conjugation_preserves_mapped_rotations() {
        let samples: [(f32, f32, f32); 5] = [
            (10.0, 30.0, 0.0),
            (90.0, 90.0, 0.0),
            (0.0, 45.0, 15.0),
            (-20.0, -120.0, 60.0),
            (0.0, 0.0, 25.95),
        ];
        for (pitch, yaw, roll) in samples {
            let q_ue = ue_source_rotator_quat(pitch, yaw, roll);
            let q_bevy = ue_quat_to_bevy(q_ue);
            for v in [
                Vec3::X,
                Vec3::Y,
                Vec3::Z,
                Vec3::new(1.0, 2.0, 3.0),
                Vec3::new(-4.0, 0.5, 2.0),
            ] {
                let via_ue = map_dir(q_ue.mul_vec3(v));
                let via_bevy = q_bevy.mul_vec3(map_dir(v));
                assert_vec3_eq(via_ue, via_bevy, 1e-4);
            }
        }
    }

    #[test]
    fn rotator_conversion_matches_ue_quaternion_formula() {
        let samples = [
            (0.0_f32, 0.0, 25.95),
            (0.0, 0.0, 35.1),
            (0.0, 0.0, 44.25),
            (0.0, 90.0, -15.0),
            (90.0, 90.0, 0.0),
            (30.0, 45.0, 20.0),
        ];
        for (pitch, yaw, roll) in samples {
            let q_ue = ue_source_rotator_quat(pitch, yaw, roll);
            let expected = ue_quat_to_bevy(q_ue);
            let actual = ue_rotator_to_bevy_quat(pitch, yaw, roll);
            assert_quat_eq(actual, expected, 1e-5);
            for v in [Vec3::X, Vec3::Y, Vec3::Z] {
                assert_vec3_eq(actual.mul_vec3(map_dir(v)), map_dir(q_ue.mul_vec3(v)), 1e-4);
            }
        }
    }

    #[test]
    fn player_start_lands_on_floor() {
        let pos = ue_point_to_bevy(Vec3::new(-1500.0, -1500.0, 92.0));
        assert_vec3_eq(pos, Vec3::new(-15.0, 0.92, -15.0), 1e-4);
    }

    #[test]
    fn wall_collider_bottom_rests_on_ground() {
        let half = ue_extents_to_bevy(Vec3::new(25.0, 1000.0, 100.0));
        let offset = ue_point_to_bevy(Vec3::new(0.0, 0.0, 100.0));
        let center_y = offset.y;
        assert!((center_y - half.y).abs() < 1e-5);
    }

    #[test]
    fn moving_actor_scene_keeps_the_source_actor_anchor_separate_from_its_mesh() {
        use bevy::ecs::system::RunSystemOnce;
        use bevy::world_serialization::WorldAsset;
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<WorldAsset>()
            .init_resource::<Assets<Mesh>>()
            .init_resource::<Assets<StandardMaterial>>()
            .init_resource::<Assets<PlaygroundTilesMaterial>>()
            .init_resource::<Assets<PlaygroundSkyMaterial>>()
            .init_resource::<Assets<Image>>()
            .init_resource::<PlaygroundMaterialLibrary>()
            .insert_resource(PlaygroundManifestPath(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("assets/ALSExtras/Levels/L_Als_Playground/playground_scene.ron"),
            ));
        app.finish();
        app.world_mut()
            .run_system_once(playground_setup_system)
            .unwrap();
        let actors: Vec<_> = app
            .world_mut()
            .query_filtered::<(Entity, &Name, Option<&ChildOf>), With<PlaygroundMovingObject>>()
            .iter(app.world())
            .map(|(entity, name, parent)| (entity, name.to_string(), parent.map(ChildOf::parent)))
            .collect();
        assert_eq!(actors.len(), 3);
        for (mesh, name, parent) in &actors {
            let root = parent.expect("The moving mesh has no independent actor anchor");
            assert_eq!(
                app.world().get::<Name>(root).unwrap().as_str(),
                name.replace("Moving:", "Actor:")
                    .replace("Rotating:", "Actor:")
            );
            assert!(app.world().get::<PlaygroundKinematic>(root).is_none());
            assert!(
                app.world().get::<InheritedVisibility>(root).is_some(),
                "The actor anchor breaks inherited visibility"
            );
            assert!(app.world().get::<WorldAssetRoot>(*mesh).is_some());
            assert_eq!(
                app.world().get::<Transform>(*mesh).unwrap().translation,
                Vec3::ZERO
            );
            let before = *app.world().get::<GlobalTransform>(root).unwrap();
            let anchor = app
                .world()
                .get::<Transform>(root)
                .unwrap()
                .clone()
                .with_rotation(Quat::from_rotation_y(0.7))
                .with_scale(Vec3::new(2.0, 1.5, 0.5));
            let after = GlobalTransform::from(anchor);
            let mut kinematic = app
                .world_mut()
                .get_mut::<PlaygroundKinematic>(*mesh)
                .unwrap();
            if let PlaygroundKinematicKind::MoveSpline { start, end, .. } = &mut kinematic.kind {
                *start = after.transform_point(before.affine().inverse().transform_point3(*start));
                *end = after.transform_point(before.affine().inverse().transform_point3(*end));
            }
            app.world_mut().entity_mut(root).insert((anchor, after));
        }
        for delta in [0.25, 1.3, 0.8] {
            app.world_mut()
                .resource_mut::<Time>()
                .advance_by(std::time::Duration::from_secs_f32(delta));
            app.world_mut()
                .run_system_once(playground_moving_object_system)
                .unwrap();
            for (mesh, _, parent) in &actors {
                let parent = parent.unwrap();
                let anchor = app.world().get::<GlobalTransform>(parent).unwrap();
                let transform = app.world().get::<Transform>(*mesh).unwrap();
                let time = app.world().resource::<Time>().elapsed_secs();
                match app.world().get::<PlaygroundKinematic>(*mesh).unwrap().kind {
                    PlaygroundKinematicKind::MoveSpline {
                        duration,
                        start,
                        end,
                    } => {
                        let expected = moving_object_position(
                            app.world().resource::<PlaygroundMovingCurve>(),
                            start,
                            end,
                            time,
                            duration,
                        );
                        assert!(
                            anchor
                                .transform_point(transform.translation)
                                .abs_diff_eq(expected, 1e-5)
                        );
                        assert_eq!(transform.rotation, Quat::IDENTITY);
                    }
                    PlaygroundKinematicKind::RotateWorldTime { rate_yzx_radians } => {
                        let expected = rotating_object_rotation(rate_yzx_radians, time);
                        assert!(
                            (transform.rotation * Vec3::X).abs_diff_eq(expected * Vec3::X, 1e-5)
                        );
                        assert_eq!(transform.translation, Vec3::ZERO);
                    }
                }
                assert_eq!(
                    anchor.translation(),
                    app.world().get::<Transform>(parent).unwrap().translation
                );
            }
        }
    }

    #[test]
    fn rotating_object_rotation_preserves_source_axes_and_large_world_time() {
        use bevy::math::DVec3;
        for (pitch, yaw, roll) in [
            (90.0_f32, 0.0_f32, 0.0_f32),
            (0.0, 90.0, 0.0),
            (0.0, 0.0, 90.0),
            (23.0, 67.0, -41.0),
        ] {
            let rate = DVec3::new(
                -f64::from(yaw).to_radians(),
                f64::from(pitch).to_radians(),
                f64::from(roll).to_radians(),
            );
            for time in [0.0, 0.13, 1.0, 3.27] {
                let actual = rotating_object_rotation(rate, time);
                let expected = ue_rotator_to_bevy_quat(pitch * time, yaw * time, roll * time);
                for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                    assert!((actual * axis).abs_diff_eq(expected * axis, 1e-5));
                }
            }
        }
        for time in [10_000.13_f32, 1_000_000.25_f32] {
            let actual =
                rotating_object_rotation(DVec3::new(-100.0_f64.to_radians(), 0.0, 0.0), time);
            let angle = (100.0_f64 * f64::from(time)).rem_euclid(360.0);
            let expected = Quat::from_rotation_y(-angle.to_radians() as f32);
            assert!((actual * Vec3::X).abs_diff_eq(expected * Vec3::X, 1e-5));
        }
    }

    #[test]
    fn moving_curve_uses_the_exported_source_samples() {
        let curve = load_moving_object_curve().expect("load moving curve");
        assert!(curve.is_source_loaded());
        assert_eq!(curve.key_count(), 4);
        let samples: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/playground/moving_object_curve_samples.json"
        ))
        .unwrap();
        for sample in samples["keys"].as_array().unwrap() {
            let time = sample["t"].as_f64().unwrap() as f32;
            let value = sample["v"].as_f64().unwrap() as f32;
            assert!(
                (curve.sample(time) - value).abs() < 1e-6,
                "Native curve differs at time={time}"
            );
        }
    }

    #[test]
    fn moving_curve_ping_pongs_with_the_source_time_rule() {
        assert!((moving_object_curve_input(0.0, 1.25) - 0.0).abs() < 1e-6);
        assert!((moving_object_curve_input(0.625, 1.25) - 0.5).abs() < 1e-6);
        assert!((moving_object_curve_input(1.25, 1.25) - 1.0).abs() < 1e-6);
        assert!((moving_object_curve_input(1.5625, 1.25) - 0.75).abs() < 1e-6);
        assert!((moving_object_curve_input(2.5, 1.25) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn moving_spline_endpoints_preserve_ue_axes() {
        let curve = load_moving_object_curve().expect("load moving curve");
        let vertical_start = ue_point_to_bevy(Vec3::new(2275.0, -2875.0, 0.0));
        let vertical_end = ue_point_to_bevy(Vec3::new(2275.0, -2875.0, 275.0));
        let horizontal_start = ue_point_to_bevy(Vec3::new(1900.0, -2875.0, 0.0));
        let horizontal_end = ue_point_to_bevy(Vec3::new(1250.0, -2875.0, 0.0));
        assert_vec3_eq(
            moving_object_position(&curve, vertical_start, vertical_end, 1.25, 1.25),
            vertical_end,
            1e-5,
        );
        assert_vec3_eq(
            moving_object_position(&curve, horizontal_start, horizontal_end, 1.75, 1.75),
            horizontal_end,
            1e-5,
        );
        assert!((vertical_end.x - vertical_start.x).abs() < 1e-6);
        assert!((vertical_end.z - vertical_start.z).abs() < 1e-6);
        assert!((horizontal_end.y - horizontal_start.y).abs() < 1e-6);
        assert!((horizontal_end.z - horizontal_start.z).abs() < 1e-6);
    }

    #[test]
    fn gltf_base_extraction() {
        assert_eq!(
            gltf_asset_base("/ALS/ALSExtras/Environment/Meshes/SM_Als_Rock.SM_Als_Rock").as_deref(),
            Some("SM_Als_Rock")
        );
        assert!(gltf_asset_base("/Engine/BasicShapes/Cube.Cube").is_none());
    }

    #[test]
    fn engine_basic_detection() {
        assert!(is_engine_basic("/Engine/BasicShapes/Cube.Cube"));
        assert!(!is_engine_basic(
            "/ALS/ALSExtras/Environment/Meshes/SM_Als_Rock.SM_Als_Rock"
        ));
    }

    #[test]
    fn source_collision_categories_cover_all_exported_meshes() {
        let boxes = [
            "SM_Als_Block_2x2",
            "SM_Als_Block_3x3",
            "SM_Als_Crate",
            "SM_Als_Floor_20x20",
            "SM_Als_Pillar",
            "SM_Als_TitleBoard",
            "SM_Als_Wall_2x20",
            "SM_Als_Wall_3x5",
        ];
        let convexes = [
            ("SM_Als_Arch_01", 5),
            ("SM_Als_Arch_02", 4),
            ("SM_Als_Barrel", 1),
            ("SM_Als_Barrel_Broken", 1),
            ("SM_Als_Bench", 4),
        ];
        let trimeshes = [
            "SM_Als_Cart",
            "SM_Als_Crate_Broken",
            "SM_Als_Fountain_01",
            "SM_Als_Rock",
            "SM_Als_Stairs_01",
            "SM_Als_Stairs_02",
            "SM_Als_Stairs_03",
        ];
        for mesh in boxes {
            assert_eq!(playground_collider_kind(mesh), PlaygroundColliderKind::Box);
        }
        for (mesh, source_hulls) in convexes {
            assert_eq!(
                playground_collider_kind(mesh),
                PlaygroundColliderKind::Convex { source_hulls }
            );
        }
        for mesh in trimeshes {
            assert_eq!(
                playground_collider_kind(mesh),
                PlaygroundColliderKind::TriMesh
            );
        }
        assert_eq!(boxes.len() + convexes.len() + trimeshes.len(), 20);
    }

    #[test]
    fn ue_visual_scale_maps_axes() {
        assert_vec3_eq(
            ue_scale_to_bevy(Vec3::new(1.5, 1.5, 1.75)),
            Vec3::new(1.5, 1.75, 1.5),
            1e-6,
        );
    }

    #[test]
    fn text_alignment_maps_source_enums() {
        assert_eq!(
            text_horizontal_alignment(Some("EHTA_Left")),
            PlaygroundTextHorizontalAlignment::Left
        );
        assert_eq!(
            text_horizontal_alignment(Some("EHTA_Center")),
            PlaygroundTextHorizontalAlignment::Center
        );
        assert_eq!(
            text_vertical_alignment(Some("EVRTA_TextTop")),
            PlaygroundTextVerticalAlignment::Top
        );
        assert_eq!(
            text_anchor(
                PlaygroundTextHorizontalAlignment::Left,
                PlaygroundTextVerticalAlignment::Top,
            ),
            Anchor::TOP_LEFT
        );
    }

    #[test]
    fn text_render_color_preserves_the_source_linear_channels() {
        let color = text_color_linear([0.46274513, 0.8, 0.9568628, 1.0]).to_linear();
        assert!((color.red - 0.46274513).abs() < 1e-6);
        assert!((color.green - 0.8).abs() < 1e-6);
        assert!((color.blue - 0.9568628).abs() < 1e-6);
        assert!((color.alpha - 1.0).abs() < 1e-6);
    }

    #[test]
    fn per_project_source_layout_preserves_leading_whitespace() {
        let (line_breaks, spaces) = leading_text_layout_whitespace(
            "\r\n\r\n\r\n\r\n                                                                               PER PROJECT",
        );
        assert_eq!(line_breaks, 4);
        assert_eq!(spaces, 79);
    }

    #[test]
    fn text_mesh_front_matches_ue_text_render_axes() {
        let source_rotation = ue_rotator_to_bevy_quat(0.0, 0.0, 0.0);
        let mesh_rotation = ue_text_mesh_rotation_to_bevy_quat(0.0, 0.0, 0.0);
        assert_vec3_eq(
            mesh_rotation.mul_vec3(Vec3::Z),
            source_rotation.mul_vec3(Vec3::X),
            1e-6,
        );
        assert_vec3_eq(mesh_rotation.mul_vec3(Vec3::X), Vec3::NEG_Z, 1e-6);
        assert_vec3_eq(mesh_rotation.mul_vec3(Vec3::Y), Vec3::Y, 1e-6);
    }

    #[test]
    fn world_text_quad_winding_faces_positive_z() {
        let positions = [
            Vec3::new(-1.0, 1.0, 1.0),
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(1.0, -1.0, 1.0),
            Vec3::new(-1.0, -1.0, 1.0),
        ];
        let first = positions[WORLD_TEXT_QUAD_INDICES[0] as usize];
        let second = positions[WORLD_TEXT_QUAD_INDICES[1] as usize];
        let third = positions[WORLD_TEXT_QUAD_INDICES[2] as usize];
        assert!((second - first).cross(third - first).z > 0.0);
    }

    #[test]
    fn skip_classification() {
        assert!(!is_skipped("TextRenderActor"));
        assert!(!is_skipped("Landscape"));
        assert!(is_skipped("LandscapeGizmoActiveActor"));
        assert!(!is_skipped("B_Als_StaticObject_C"));
        assert!(!is_skipped("StaticMeshActor"));
        assert!(is_skipped("Brush"));
    }

    #[test]
    fn manifest_deserializes_ron_snippet() {
        let src = "#![enable(implicit_some)]\n(actors: [(class: \"Brush\", name: \"Brush_0\", location: (1.0, 2.0, 3.0), rotation_yaw: 45.0, rotation: (pitch: 0.0, yaw: 45.0, roll: 0.0), scale: (1.0, 1.0, 1.0), bounds_origin: (1.0, 2.0, 103.0), bounds_extent: (150.0, 100.0, 100.0))])";
        let m: PlaygroundManifest = from_str(src).expect("parse");
        assert_eq!(m.actors.len(), 1);
        assert_eq!(m.actors[0].class, "Brush");
        assert_eq!(m.actors[0].bounds_extent.unwrap(), [150.0, 100.0, 100.0]);
        assert!(m.actors[0].components.is_empty());
    }
}
