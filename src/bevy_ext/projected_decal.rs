use bevy::{
    light::ClusteredDecal,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline},
    prelude::*,
    render::{
        RenderApp,
        render_resource::{
            AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
        },
        storage::ShaderBuffer,
    },
    shader::ShaderRef,
};
use std::collections::{HashMap, HashSet};

pub const PACKED_DECAL_TAG: u32 = 1 << 31;
pub(crate) const DECAL_PROBE_SAMPLES: usize = 16;
pub(crate) const DECAL_PROBE_WORDS: usize = 12 + DECAL_PROBE_SAMPLES * 40;

#[derive(SystemSet, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PackedDecalPrepareSet;

#[derive(Component)]
pub struct NoDecalReceiver;

#[derive(Asset, TypePath, Clone, Debug)]
pub struct PackedDecalMaterial {
    pub color: Vec4,
    pub roughness_scale: f32,
    pub metallic: f32,
    pub specular: f32,
    pub texture: Handle<Image>,
}

#[derive(Component, Clone, Debug)]
#[require(Transform, Visibility, ClusteredDecal)]
pub struct PackedDecal(pub Handle<PackedDecalMaterial>);

#[derive(Clone, Default, ShaderType)]
pub struct PackedDecalData {
    pub local_from_world: Mat4,
    pub color: Vec4,
    pub material: Vec4,
}

#[derive(Resource, Clone, Default, bevy::render::extract_resource::ExtractResource)]
#[extract_app(RenderApp)]
pub struct PackedDecalBuffers {
    pub materials: Handle<ShaderBuffer>,
    pub probe: Handle<ShaderBuffer>,
    capacity: usize,
    /// Enables frame markers for an external shader probe.
    pub(crate) probe_enabled: bool,
}

impl PackedDecalBuffers {
    fn new(buffers: &mut Assets<ShaderBuffer>) -> Self {
        Self {
            materials: buffers.add(ShaderBuffer::from(vec![[Vec4::ZERO; 6]])),
            // Match the layout of DecalMaterialProbe in packed_decal.wesl.
            probe: buffers.add(ShaderBuffer::from(vec![0_u32; DECAL_PROBE_WORDS])),
            capacity: 1,
            probe_enabled: false,
        }
    }
}

/// Rebuilds custom receiver bindings when the shared decal buffer grows.
pub(crate) fn install_decal_receiver_bindings<M: bevy::pbr::Material>(app: &mut App)
where
    M::Data: PartialEq + Eq + std::hash::Hash + Clone,
{
    app.add_systems(
        PostUpdate,
        refresh_decal_receiver_bindings::<M>
            .after(PackedDecalPrepareSet)
            .before(bevy::asset::AssetEventSystems),
    );
    if let Some(render) = app.get_sub_app_mut(bevy::render::RenderApp) {
        render.add_systems(
            bevy::render::Render,
            ApplyDeferred
                .in_set(bevy::render::RenderSystems::PrepareAssets)
                .after(
                    bevy::render::render_asset::prepare_assets::<
                        bevy::render::storage::GpuShaderBuffer,
                    >,
                )
                .before(
                    bevy::render::erased_render_asset::prepare_erased_assets::<MeshMaterial3d<M>>,
                ),
        );
    }
}

fn refresh_decal_receiver_bindings<M: Asset>(
    buffers: Res<PackedDecalBuffers>,
    mut materials: ResMut<Assets<M>>,
    mut capacity: Local<usize>,
) {
    if *capacity == buffers.capacity {
        return;
    }
    *capacity = buffers.capacity;
    materials.iter_mut().for_each(drop);
}

pub type PackedDecalReceiverMaterial =
    ExtendedMaterial<StandardMaterial, PackedDecalReceiverExtension>;

#[derive(Component, Clone)]
pub struct StandardMaterialSource(pub Handle<StandardMaterial>);

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct PackedDecalReceiverExtension {
    #[storage(110, read_only, visibility(fragment))]
    pub decals: Handle<ShaderBuffer>,
    #[storage(111, visibility(fragment))]
    pub probe: Handle<ShaderBuffer>,
}

impl MaterialExtension for PackedDecalReceiverExtension {
    fn fragment_shader() -> ShaderRef {
        "ALSExtras/Environment/Materials/packed_decal_receiver.wesl".into()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        "ALSExtras/Environment/Materials/packed_decal_receiver.wesl".into()
    }
    fn specialize(
        _: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _: &MeshVertexBufferLayoutRef,
        _: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.label = Some(
            format!(
                "als_packed_receiver_{}",
                descriptor.label.as_deref().unwrap_or_default()
            )
            .into(),
        );
        Ok(())
    }
}

pub struct PackedDecalPlugin;

impl Plugin for PackedDecalPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy::pbr::MaterialPlugin::<PackedDecalReceiverMaterial>::default())
            .init_asset::<PackedDecalMaterial>();
        if !app.world().contains_resource::<Assets<ShaderBuffer>>() {
            app.init_asset::<ShaderBuffer>();
        }
        let buffers =
            PackedDecalBuffers::new(&mut app.world_mut().resource_mut::<Assets<ShaderBuffer>>());
        app.insert_resource(buffers)
            .add_plugins(bevy::render::extract_resource::ExtractResourcePlugin::<
                PackedDecalBuffers,
            >::default())
            .add_systems(
                Startup,
                |settings: Option<ResMut<bevy::light::cluster::GlobalClusterSettings>>| {
                    if let Some(mut settings) = settings {
                        settings.gpu_clustering = None;
                    }
                },
            )
            .add_systems(Update, prepare_receivers.in_set(PackedDecalPrepareSet))
            .add_systems(
                PostUpdate,
                prepare_decals
                    .after(bevy::transform::TransformSystems::Propagate)
                    .in_set(PackedDecalPrepareSet),
            );
        install_decal_receiver_bindings::<PackedDecalReceiverMaterial>(app);
    }
}

fn prepare_decals(
    mut decals: Query<(Entity, &PackedDecal, &GlobalTransform, &mut ClusteredDecal)>,
    materials: Res<Assets<PackedDecalMaterial>>,
    mut handles: ResMut<PackedDecalBuffers>,
    mut buffers: ResMut<Assets<ShaderBuffer>>,
    frame: Res<bevy::diagnostic::FrameCount>,
) {
    let mut entities: Vec<_> = decals.iter().map(|(entity, _, _, _)| entity).collect();
    entities.sort_unstable();
    let mut values = Vec::with_capacity(entities.len().max(1));
    for entity in entities {
        let (_, handle, transform, mut decal) = decals.get_mut(entity).unwrap();
        let Some(material) = materials.get(&handle.0) else {
            decal.metallic_roughness_texture = None;
            continue;
        };
        let index = values.len() as u32;
        decal.tag = PACKED_DECAL_TAG | index;
        decal.base_color_texture = None;
        decal.normal_map_texture = None;
        decal.emissive_texture = None;
        decal.metallic_roughness_texture = Some(material.texture.clone());
        values.push(PackedDecalData {
            local_from_world: transform.affine().inverse().into(),
            color: material.color,
            material: Vec4::new(
                material.roughness_scale,
                material.metallic,
                material.specular,
                0.0,
            ),
        });
    }
    let required = values.len().max(1);
    if required > handles.capacity {
        handles.capacity = required.next_power_of_two();
    }
    values.resize(handles.capacity, default());
    if handles.probe_enabled {
        for value in &mut values {
            value.material.w = frame.0 as f32;
        }
    }
    let mut materials = buffers.get_mut(&handles.materials).unwrap();
    materials.clear();
    materials.extend(values.into_iter().map(|value| {
        [
            value.local_from_world.x_axis,
            value.local_from_world.y_axis,
            value.local_from_world.z_axis,
            value.local_from_world.w_axis,
            value.color,
            value.material,
        ]
    }));
}

fn prepare_receivers(
    mut commands: Commands,
    source: Res<Assets<StandardMaterial>>,
    mut targets: ResMut<Assets<PackedDecalReceiverMaterial>>,
    handles: Res<PackedDecalBuffers>,
    mut changes: MessageReader<AssetEvent<StandardMaterial>>,
    meshes: Query<(Entity, &MeshMaterial3d<StandardMaterial>), With<Mesh3d>>,
    converted: Query<
        (Entity, &StandardMaterialSource),
        With<MeshMaterial3d<PackedDecalReceiverMaterial>>,
    >,
    excluded: Query<(), With<NoDecalReceiver>>,
    mut cache: Local<HashMap<AssetId<StandardMaterial>, Handle<PackedDecalReceiverMaterial>>>,
) {
    let ignored = |entity| excluded.contains(entity);
    let eligible = |base: &StandardMaterial| {
        matches!(
            base.alpha_mode,
            AlphaMode::Opaque | AlphaMode::Mask(_) | AlphaMode::AlphaToCoverage
        )
    };
    let changed: HashSet<_> = changes
        .read()
        .filter_map(|event| match event {
            AssetEvent::Modified { id } => Some(*id),
            _ => None,
        })
        .collect();
    let mut active = HashSet::new();
    for (entity, material) in &converted {
        if ignored(entity)
            || meshes.contains(entity)
            || source.get(&material.0).is_some_and(|base| !eligible(base))
        {
            commands.entity(entity).remove::<(
                MeshMaterial3d<PackedDecalReceiverMaterial>,
                StandardMaterialSource,
            )>();
            if meshes.get(entity).is_err() {
                commands
                    .entity(entity)
                    .insert(MeshMaterial3d(material.0.clone()));
            }
        } else {
            active.insert(material.0.id());
        }
    }
    for (entity, material) in &meshes {
        if ignored(entity) {
            continue;
        }
        let Some(base) = source.get(&material.0) else {
            continue;
        };
        if !eligible(base) {
            continue;
        }
        active.insert(material.0.id());
        let target = cache.entry(material.0.id()).or_insert_with(|| {
            targets.add(PackedDecalReceiverMaterial {
                base: base.clone(),
                extension: PackedDecalReceiverExtension {
                    decals: handles.materials.clone(),
                    probe: handles.probe.clone(),
                },
            })
        });
        commands
            .entity(entity)
            .remove::<MeshMaterial3d<StandardMaterial>>()
            .insert((
                MeshMaterial3d(target.clone()),
                StandardMaterialSource(material.0.clone()),
            ));
    }
    cache.retain(|id, _| active.contains(id));
    for id in changed {
        if let Some(handle) = cache.get(&id)
            && let Some(base) = source.get(id)
            && let Some(mut target) = targets.get_mut(handle)
        {
            target.base = base.clone();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearby_decals_use_native_cpu_clustering_for_degenerate_depth_ranges() {
        use bevy::light::cluster::{GlobalClusterGpuSettings, GlobalClusterSettings};
        let factors =
            bevy::light::cluster::assign::calculate_cluster_factors(5.0, 5.0, 24.0, false);
        assert!(!(factors.y / factors.x).exp().is_finite());
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<Mesh>()
            .init_asset::<Image>()
            .init_asset::<StandardMaterial>()
            .insert_resource(GlobalClusterSettings {
                supports_storage_buffers: true,
                clustered_decals_are_usable: true,
                gpu_clustering: Some(GlobalClusterGpuSettings {
                    initial_z_slice_list_capacity: 1024,
                    initial_index_list_capacity: 1024,
                }),
                max_uniform_buffer_clusterable_objects: 256,
                view_cluster_bindings_max_indices: 16384,
            })
            .add_plugins(PackedDecalPlugin);
        app.update();
        assert!(
            app.world()
                .resource::<GlobalClusterSettings>()
                .gpu_clustering
                .is_none()
        );
    }

    #[test]
    fn receivers_recheck_in_place_material_alpha_mode() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<StandardMaterial>()
            .init_asset::<PackedDecalReceiverMaterial>()
            .init_resource::<PackedDecalBuffers>()
            .add_systems(Update, prepare_receivers);
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial::default());
        let first = app
            .world_mut()
            .spawn((Mesh3d::default(), MeshMaterial3d(material.clone())))
            .id();
        let second = app
            .world_mut()
            .spawn((Mesh3d::default(), MeshMaterial3d(material.clone())))
            .id();
        app.update();
        assert!(
            app.world()
                .get::<MeshMaterial3d<PackedDecalReceiverMaterial>>(first)
                .is_some()
        );
        app.world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .get_mut(&material)
            .unwrap()
            .alpha_mode = AlphaMode::Blend;
        app.update();
        for entity in [first, second] {
            assert!(
                app.world()
                    .get::<MeshMaterial3d<PackedDecalReceiverMaterial>>(entity)
                    .is_none()
            );
            assert!(app.world().get::<StandardMaterialSource>(entity).is_none());
            assert_eq!(
                app.world()
                    .get::<MeshMaterial3d<StandardMaterial>>(entity)
                    .unwrap()
                    .0,
                material
            );
        }
        {
            let mut materials = app.world_mut().resource_mut::<Assets<StandardMaterial>>();
            let mut base = materials.get_mut(&material).unwrap();
            base.alpha_mode = AlphaMode::Opaque;
            base.metallic = 0.7;
        }
        app.update();
        let restored = app
            .world()
            .get::<MeshMaterial3d<PackedDecalReceiverMaterial>>(first)
            .unwrap();
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<PackedDecalReceiverMaterial>>(second)
                .unwrap()
                .0,
            restored.0
        );
        assert_eq!(
            app.world()
                .resource::<Assets<PackedDecalReceiverMaterial>>()
                .get(&restored.0)
                .unwrap()
                .base
                .metallic,
            0.7
        );
        for entity in [first, second] {
            assert!(
                app.world()
                    .get::<MeshMaterial3d<StandardMaterial>>(entity)
                    .is_none()
            );
            assert_eq!(
                app.world().get::<StandardMaterialSource>(entity).unwrap().0,
                material
            );
        }
    }

    #[test]
    fn receiver_exit_preserves_same_frame_replacement_material() {
        for excluded in [true, false] {
            let mut app = App::new();
            app.add_plugins((MinimalPlugins, AssetPlugin::default()))
                .init_asset::<StandardMaterial>()
                .init_asset::<PackedDecalReceiverMaterial>()
                .init_resource::<PackedDecalBuffers>()
                .add_systems(Update, prepare_receivers);
            let old = app
                .world_mut()
                .resource_mut::<Assets<StandardMaterial>>()
                .add(StandardMaterial::default());
            let replacement = app
                .world_mut()
                .resource_mut::<Assets<StandardMaterial>>()
                .add(StandardMaterial {
                    base_color: Color::srgba(0.2, 0.7, 0.4, 0.5),
                    alpha_mode: if excluded {
                        AlphaMode::Opaque
                    } else {
                        AlphaMode::Blend
                    },
                    ..default()
                });
            let receiver = app
                .world_mut()
                .spawn((Mesh3d::default(), MeshMaterial3d(old.clone())))
                .id();
            app.update();
            assert_eq!(
                app.world()
                    .get::<StandardMaterialSource>(receiver)
                    .unwrap()
                    .0,
                old
            );
            app.world_mut()
                .entity_mut(receiver)
                .insert(MeshMaterial3d(replacement.clone()));
            if excluded {
                app.world_mut().entity_mut(receiver).insert(NoDecalReceiver);
            }
            app.update();
            assert_eq!(
                app.world()
                    .get::<MeshMaterial3d<StandardMaterial>>(receiver)
                    .unwrap()
                    .0,
                replacement
            );
            assert!(
                app.world()
                    .get::<MeshMaterial3d<PackedDecalReceiverMaterial>>(receiver)
                    .is_none(),
                "Old receiver proxy must be removed: excluded={excluded}"
            );
            assert!(
                app.world()
                    .get::<StandardMaterialSource>(receiver)
                    .is_none()
            );
        }
    }

    #[test]
    fn receivers_preserve_updates_exclusions_and_release_unused_materials() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, AssetPlugin::default()))
            .init_asset::<StandardMaterial>()
            .init_asset::<PackedDecalReceiverMaterial>()
            .init_resource::<PackedDecalBuffers>()
            .add_systems(Update, prepare_receivers);
        let material = app
            .world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                base_color: Color::linear_rgb(0.2, 0.3, 0.4),
                metallic: 0.7,
                perceptual_roughness: 0.8,
                reflectance: 0.9,
                ..default()
            });
        let first = app
            .world_mut()
            .spawn((Mesh3d::default(), MeshMaterial3d(material.clone())))
            .id();
        let second = app
            .world_mut()
            .spawn((Mesh3d::default(), MeshMaterial3d(material.clone())))
            .id();
        let excluded = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                MeshMaterial3d(material.clone()),
                NoDecalReceiver,
            ))
            .id();
        app.update();
        let handle = app
            .world()
            .get::<MeshMaterial3d<PackedDecalReceiverMaterial>>(first)
            .unwrap()
            .0
            .clone();
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<PackedDecalReceiverMaterial>>(second)
                .unwrap()
                .0,
            handle
        );
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<StandardMaterial>>(excluded)
                .unwrap()
                .0,
            material
        );
        assert!(
            app.world()
                .get::<MeshMaterial3d<PackedDecalReceiverMaterial>>(excluded)
                .is_none()
        );
        assert_eq!(
            app.world().get::<StandardMaterialSource>(first).unwrap().0,
            material
        );
        let read = |app: &App| {
            app.world()
                .resource::<Assets<PackedDecalReceiverMaterial>>()
                .get(&handle)
                .unwrap()
                .base
                .clone()
        };
        assert_eq!(read(&app).metallic, 0.7);
        assert_eq!(read(&app).reflectance, 0.9);
        app.world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .get_mut(&material)
            .unwrap()
            .metallic = 0.4;
        app.update();
        app.update();
        assert_eq!(read(&app).metallic, 0.4);
        app.world_mut().entity_mut(first).insert(NoDecalReceiver);
        app.update();
        assert_eq!(
            app.world()
                .get::<MeshMaterial3d<StandardMaterial>>(first)
                .unwrap()
                .0,
            material
        );
        assert!(app.world().get::<StandardMaterialSource>(first).is_none());
        assert!(
            app.world()
                .get::<MeshMaterial3d<PackedDecalReceiverMaterial>>(first)
                .is_none()
        );
        app.world_mut().despawn(second);
        drop(handle);
        for _ in 0..4 {
            app.update();
        }
        assert!(
            app.world()
                .resource::<Assets<PackedDecalReceiverMaterial>>()
                .is_empty()
        );
    }
}
