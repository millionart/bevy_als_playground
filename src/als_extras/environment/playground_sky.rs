use crate::playground::{
    MovingTransformDefinition, PlaygroundRotator, PlaygroundSky, SceneLighting,
    ue_directional_light_ray_to_bevy,
};
use bevy_als::extensions::{RichCurveKey, sample_rich_curve};
use bevy::asset::RenderAssetUsages;
use bevy::image::{
    CompressedImageFormats, ImageAddressMode, ImageFilterMode, ImageSampler,
    ImageSamplerDescriptor, ImageType,
};
use bevy::light::NotShadowCaster;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::pbr::{Material, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, ShaderType};
use bevy::shader::ShaderRef;
use serde::Deserialize;

const SKY_DATA: &str =
    include_str!("../../../assets/ALSExtras/Environment/Sky/playground_sky.runtime.json");
const SKY_MESH: &str =
    include_str!("../../../assets/ALSExtras/Environment/Sky/sky_mesh.runtime.json");

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct PlaygroundSkyMaterial {
    #[uniform(0)]
    pub settings: PlaygroundSkySettings,
    #[texture(1)]
    #[sampler(2)]
    pub clouds: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    pub blue: Handle<Image>,
    #[texture(5)]
    #[sampler(6)]
    pub stars: Handle<Image>,
}

#[derive(Clone, Debug, ShaderType)]
pub struct PlaygroundSkySettings {
    pub zenith: Vec4,
    pub horizon: Vec4,
    pub cloud: Vec4,
    pub overall: Vec4,
    pub sun_color: Vec4,
    pub light_direction: Vec4,
    pub animation: Vec4,
    pub shape: Vec4,
    pub origin: Vec4,
    pub constants: Vec4,
    pub texture_settings: Vec4,
    pub intensity: Vec4,
}

impl Material for PlaygroundSkyMaterial {
    fn fragment_shader() -> ShaderRef {
        "ALSExtras/Environment/Sky/playground_sky.wesl".into()
    }
}

pub struct PlaygroundSkyPlugin;

impl Plugin for PlaygroundSkyPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(MaterialPlugin::<PlaygroundSkyMaterial>::default())
            .add_systems(Update, playground_sky_time_system);
    }
}

pub fn playground_sky_time_system(
    time: Res<Time>,
    mut materials: ResMut<Assets<PlaygroundSkyMaterial>>,
) {
    for (_, material) in materials.iter_mut() {
        material.settings.animation.w = time.elapsed_secs();
    }
}

#[derive(Deserialize, Clone)]
struct SkySun {
    pitch: f32,
    yaw: f32,
    color: Vec4,
}

#[derive(Deserialize)]
struct SkyCurves {
    zenith: [Vec<RichCurveKey>; 3],
    horizon: [Vec<RichCurveKey>; 3],
    cloud: [Vec<RichCurveKey>; 3],
}

#[derive(Deserialize)]
struct SkyConstants {
    noise_power_1: f32,
    noise_power_2: f32,
    sun_radius: f32,
    blue_speed: f32,
    cloud_speed: f32,
    noise_tiling: f32,
    stars_tiling: f32,
    emissive_scale: f32,
    rim_scale: f32,
    height_radius_scale: f32,
    glow_radius: f32,
    glow_exponent: f32,
}

#[derive(Deserialize)]
struct SkyDefinition {
    colors_by_sun: bool,
    sun_height: f32,
    actor_yaw: f32,
    sun: Option<SkySun>,
    transform: MovingTransformDefinition,
    zenith: Vec4,
    horizon: Vec4,
    cloud: Vec4,
    overall: Vec4,
    sun_brightness: f32,
    horizon_falloff: f32,
    cloud_speed: f32,
    cloud_opacity: f32,
    stars_brightness: f32,
    material_constants: SkyConstants,
    curves: SkyCurves,
}

#[derive(Deserialize)]
struct SkyMesh {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    uv0: Vec<[f32; 2]>,
    uv1: Vec<[f32; 2]>,
    indices: Vec<u32>,
    bounds_origin: Vec3,
    bounds_radius: f32,
}

fn curve_color(curves: &[Vec<RichCurveKey>; 3], height: f32) -> Vec4 {
    let rgb = Vec3::new(
        sample_rich_curve(&curves[0], height),
        sample_rich_curve(&curves[1], height),
        sample_rich_curve(&curves[2], height),
    );
    let maximum = rgb.max_element();
    let minimum = rgb.min_element();
    let rgb = if maximum <= 0.0 {
        Vec3::ZERO
    } else if minimum < 0.0 {
        (rgb - Vec3::splat(minimum)) * maximum.min(1.0) / (maximum - minimum)
    } else {
        rgb / maximum.max(1.0)
    };
    rgb.extend(1.0)
}

impl SkyDefinition {
    fn settings(&self, mesh: &SkyMesh) -> PlaygroundSkySettings {
        let transform = self.transform.transform();
        let (height, direction, sun_color) = if let Some(sun) = &self.sun {
            (
                sun.pitch / -90.0,
                ue_directional_light_ray_to_bevy(sun.pitch, sun.yaw),
                sun.color,
            )
        } else {
            let alpha = (self.sun_height + 0.2).clamp(0.0, 1.0);
            (
                self.sun_height,
                ue_directional_light_ray_to_bevy(-90.0 * self.sun_height, self.actor_yaw),
                Vec4::new(1.0, 0.221, 0.04, 1.0).lerp(Vec4::new(0.954, 0.901, 0.74412, 1.0), alpha),
            )
        };
        let (zenith, horizon, cloud, falloff) = if self.colors_by_sun {
            (
                curve_color(&self.curves.zenith, height),
                curve_color(&self.curves.horizon, height),
                curve_color(&self.curves.cloud, height),
                3.0 + 4.0 * height.abs(),
            )
        } else {
            (self.zenith, self.horizon, self.cloud, self.horizon_falloff)
        };
        let c = &self.material_constants;
        PlaygroundSkySettings {
            zenith,
            horizon,
            cloud,
            overall: self.overall,
            sun_color,
            light_direction: direction.extend(if height < 0.0 { height.abs() } else { 0.0 }),
            animation: Vec4::new(
                self.cloud_speed,
                self.cloud_opacity,
                self.stars_brightness,
                0.0,
            ),
            shape: Vec4::new(
                falloff,
                self.sun_brightness,
                c.sun_radius,
                mesh.bounds_radius * transform.scale.abs().max_element(),
            ),
            origin: transform.transform_point(mesh.bounds_origin).extend(0.0),
            constants: Vec4::new(
                c.noise_power_1,
                c.noise_power_2,
                c.glow_radius,
                c.glow_exponent,
            ),
            texture_settings: Vec4::new(
                c.blue_speed,
                c.cloud_speed,
                c.noise_tiling,
                c.stars_tiling,
            ),
            intensity: Vec4::new(c.emissive_scale, c.rim_scale, c.height_radius_scale, 0.0),
        }
    }
}

fn texture(bytes: &[u8], extension: &str, srgb: bool) -> Image {
    Image::from_buffer(
        bytes,
        ImageType::Extension(extension),
        CompressedImageFormats::NONE,
        srgb,
        ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::Repeat,
            address_mode_v: ImageAddressMode::Repeat,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            ..default()
        }),
        RenderAssetUsages::default(),
    )
    .expect("The imported source sky texture must decode")
}

pub fn spawn_playground_sky(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    images: &mut Assets<Image>,
    materials: &mut Assets<PlaygroundSkyMaterial>,
    lighting: &SceneLighting,
    sun_rotator: Option<&PlaygroundRotator>,
) {
    let mut definition: SkyDefinition =
        serde_json::from_str(SKY_DATA).expect("The imported source sky settings must parse");
    if let (Some(sun), Some(rotation)) = (&mut definition.sun, sun_rotator) {
        sun.pitch = rotation.pitch;
        sun.yaw = rotation.yaw;
        if let Some(color) = lighting.sun_color {
            sun.color = Vec4::from_array(
                Color::srgba(color[0], color[1], color[2], color[3])
                    .to_linear()
                    .to_f32_array(),
            );
        }
    }
    let source: SkyMesh =
        serde_json::from_str(SKY_MESH).expect("The imported source sky geometry must parse");
    let settings = definition.settings(&source);
    let mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, source.positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, source.normals)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, source.uv0)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_1, source.uv1)
    .with_inserted_indices(Indices::U32(source.indices));
    let material = materials.add(PlaygroundSkyMaterial {
        settings,
        clouds: images.add(texture(
            include_bytes!("../../../assets/ALSExtras/Environment/Sky/T_Sky_Clouds_M.tga"),
            "tga",
            true,
        )),
        blue: images.add(texture(
            include_bytes!("../../../assets/ALSExtras/Environment/Sky/T_Sky_Blue.png"),
            "png",
            false,
        )),
        stars: images.add(texture(
            include_bytes!("../../../assets/ALSExtras/Environment/Sky/T_Sky_Stars.tga"),
            "tga",
            true,
        )),
    });
    commands.spawn((
        Name::new("SkySphere"),
        Mesh3d(meshes.add(mesh)),
        MeshMaterial3d(material),
        definition.transform.transform(),
        NotShadowCaster,
        PlaygroundSky,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn sky_parameters_follow_source_construction_branches() {
        let mut definition: SkyDefinition = serde_json::from_str(SKY_DATA).unwrap();
        let mesh: SkyMesh = serde_json::from_str(SKY_MESH).unwrap();
        for (pitch, zenith, horizon, cloud, falloff, night) in [
            (
                -90.0,
                [0.0228, 0.048994, 0.12],
                [0.46, 0.661171, 1.0],
                [0.929289, 0.946967, 1.0],
                7.0,
                0.0,
            ),
            (
                -45.0,
                [0.0228, 0.048994, 0.12],
                [0.46, 0.661171, 1.0],
                [0.8804982, 0.8603378, 0.8862433],
                5.0,
                0.0,
            ),
            (
                0.0,
                [0.22482864, 0.09873475, 0.09363559],
                [1.0, 0.2999968, 0.10006614],
                [0.770948, 0.47746366, 0.354407],
                3.0,
                0.0,
            ),
            (
                45.0,
                [0.05267175, 0.047506526, 0.06698028],
                [0.59671295, 0.1495698, 0.11564817],
                [0.19794796, 0.13910462, 0.12890059],
                5.0,
                0.5,
            ),
            (
                90.0,
                [0.0, 0.0, 0.0],
                [0.048, 0.052, 0.075],
                [0.05395, 0.059475, 0.065],
                7.0,
                1.0,
            ),
        ] {
            definition.sun.as_mut().unwrap().pitch = pitch;
            let actual = definition.settings(&mesh);
            assert!(
                actual
                    .zenith
                    .truncate()
                    .abs_diff_eq(Vec3::from_array(zenith), 2e-6),
                "Source zenith at pitch {pitch}: {:?}",
                actual.zenith
            );
            assert!(
                actual
                    .horizon
                    .truncate()
                    .abs_diff_eq(Vec3::from_array(horizon), 2e-6),
                "Source clamped horizon at pitch {pitch}: {:?}",
                actual.horizon
            );
            assert!(
                actual
                    .cloud
                    .truncate()
                    .abs_diff_eq(Vec3::from_array(cloud), 2e-6),
                "Source cloud at pitch {pitch}: {:?}",
                actual.cloud
            );
            assert_eq!(actual.shape.x, falloff);
            assert_eq!(actual.light_direction.w, night);
        }
        definition.colors_by_sun = false;
        let manual = definition.settings(&mesh);
        assert_eq!(manual.zenith, definition.zenith);
        assert_eq!(manual.horizon, definition.horizon);
        assert_eq!(manual.cloud, definition.cloud);
        assert_eq!(manual.shape.x, 3.0);
        assert!(manual.horizon.z > 1.0);
        definition.sun = None;
        definition.sun_height = 0.5;
        definition.actor_yaw = 90.0;
        let unbound = definition.settings(&mesh);
        assert!(
            unbound
                .sun_color
                .truncate()
                .abs_diff_eq(Vec3::new(0.9678, 0.697, 0.532884), 1e-6)
        );
        assert!(unbound.light_direction.truncate().abs_diff_eq(
            Vec3::new(
                0.0,
                -std::f32::consts::FRAC_1_SQRT_2,
                std::f32::consts::FRAC_1_SQRT_2
            ),
            1e-6
        ));
    }

    #[test]
    fn sky_source_geometry_faces_the_inside_of_the_mesh() {
        let mesh: SkyMesh = serde_json::from_str(SKY_MESH).unwrap();
        assert_eq!(mesh.uv0.len(), mesh.positions.len());
        assert_eq!(mesh.uv1.len(), mesh.positions.len());
        assert_ne!(mesh.uv0, mesh.uv1);
        for face in mesh.indices.chunks_exact(3) {
            let [a, b, c] = [face[0], face[1], face[2]]
                .map(|index| Vec3::from_array(mesh.positions[index as usize]));
            let normal = Vec3::from_array(mesh.normals[face[0] as usize]);
            assert!(a.dot(normal) < 0.0);
            assert!(
                (b - a).cross(c - a).dot(normal) > 0.0,
                "Source inward faces must survive back-face culling"
            );
        }
    }

    #[test]
    fn sky_cloud_time_uses_the_unwrapped_game_clock() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .init_resource::<Assets<PlaygroundSkyMaterial>>();
        let definition: SkyDefinition = serde_json::from_str(SKY_DATA).unwrap();
        let mesh: SkyMesh = serde_json::from_str(SKY_MESH).unwrap();
        let material = app
            .world_mut()
            .resource_mut::<Assets<PlaygroundSkyMaterial>>()
            .add(PlaygroundSkyMaterial {
                settings: definition.settings(&mesh),
                clouds: Handle::default(),
                blue: Handle::default(),
                stars: Handle::default(),
            });
        for seconds in [0.0, 1.0, 60.0, 3601.0] {
            let mut time = Time::<()>::default();
            time.advance_by(std::time::Duration::from_secs_f32(seconds));
            app.insert_resource(time);
            app.world_mut()
                .run_system_once(playground_sky_time_system)
                .unwrap();
            let settings = &app
                .world()
                .resource::<Assets<PlaygroundSkyMaterial>>()
                .get(&material)
                .unwrap()
                .settings;
            assert_eq!(settings.animation.w, seconds);
            let offset = settings.animation.w * settings.animation.x * settings.texture_settings.y;
            assert!(
                (offset - seconds * 0.002).abs() < 1e-6,
                "Source cloud panner offset at {seconds}: {offset}"
            );
        }
    }
}
