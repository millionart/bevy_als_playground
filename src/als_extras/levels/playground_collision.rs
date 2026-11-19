use bevy_als::als::character::AlsCollisionLayer;
use bevy_als::extensions::cooked_mesh_raycast::{CookedMeshRay, CookedMeshRayData};
use std::collections::HashMap;
use std::sync::{Arc, LazyLock};

#[cfg(test)]
use bevy_als::extensions::SceneQuery;
use crate::playground::{
    PlaygroundBoxCollider, PlaygroundColliderKind, PlaygroundCollisionSource, PlaygroundKinematic,
    PlaygroundKinematicKind, PlaygroundMovingCurve, PlaygroundMovingObject,
    PlaygroundPhysicsDriven, playground_moving_object_system,
};
use avian3d::prelude::*;
use bevy::prelude::*;

pub struct PlaygroundCollisionPlugin;

static SOURCE_MESH_RAY_CULLING: LazyLock<HashMap<String, bool>> = LazyLock::new(|| {
    serde_json::from_str(include_str!(
        "../../../assets/ALSExtras/Environment/Meshes/playground_mesh_ray_culling.runtime.json"
    ))
    .expect("Source mesh ray culling must parse")
});

static SOURCE_MESH_RAY_DATA: LazyLock<HashMap<String, Arc<CookedMeshRay>>> = LazyLock::new(|| {
    serde_json::from_str::<HashMap<String, CookedMeshRay>>(include_str!(
        "../../../assets/ALSExtras/Environment/Meshes/playground_mesh_ray_data.runtime.json"
    ))
    .expect("Cooked source mesh rays must parse")
    .into_iter()
    .map(|(name, mesh)| (name, Arc::new(mesh)))
    .collect()
});

impl Plugin for PlaygroundCollisionPlugin {
    fn build(&self, app: &mut App) {
        let [dynamic_friction, static_friction, restitution]: [f32;3] = serde_json::from_str(
            include_str!("../../../assets/ALSExtras/Levels/L_Als_Playground/default_physical_material.runtime.json"),
        ).expect("The imported source default physical material must parse");
        let schedule = app
            .world()
            .get_resource::<bevy_als::AlsPhysicsSchedule>()
            .copied();
        app.insert_resource(DefaultFriction(
            Friction::new(dynamic_friction).with_static_coefficient(static_friction),
        ))
        .insert_resource(DefaultRestitution(Restitution::new(restitution)))
        .init_resource::<PlaygroundMeshColliderCache>()
        .init_resource::<PlaygroundMovingCurve>()
        .add_systems(Update, (playground_moving_box_physics_bridge, playground_mesh_physics_bridge));
        if let Some(bevy_als::AlsPhysicsSchedule::Avian(schedule)) = schedule {
            app.add_systems(
                schedule,
                playground_kinematic_velocity_bridge
                    .after(PhysicsSystems::Prepare)
                    .before(PhysicsSystems::StepSimulation),
            );
        } else {
            app.add_systems(
                Update,
                playground_kinematic_velocity_bridge
                    .after(playground_moving_object_system)
                    .before(bevy_tnua::TnuaPipelineSystems::Sensors),
            )
            .add_systems(
                PostUpdate,
                playground_kinematic_velocity_bridge
                    .after(PhysicsSystems::Prepare)
                    .before(PhysicsSystems::StepSimulation),
            );
        }
    }
}

#[derive(Component, Debug, Clone)]
pub struct PlaygroundMeshColliderBuilt {
    pub mesh_base: String,
    pub kind: PlaygroundColliderKind,
    pub vertices: usize,
    pub triangles: usize,
    pub surface_probes: Vec<Vec3>,
}

#[derive(Clone)]
struct PlaygroundMeshColliderTemplate {
    collider: Collider,
    complex: Option<Collider>,
    vertices: usize,
    triangles: usize,
    surface_probes: Vec<Vec3>,
}

#[derive(Resource, Default)]
struct PlaygroundMeshColliderCache(HashMap<String, PlaygroundMeshColliderTemplate>);

struct PlaygroundMeshGeometry {
    vertices: Vec<Vec3>,
    triangles: Vec<[u32; 3]>,
}

pub fn playground_kinematic_velocity_bridge(
    time: Res<Time>,
    virtual_time: Res<Time<Virtual>>,
    schedule: Option<Res<bevy_als::AlsPhysicsSchedule>>,
    curve: Res<PlaygroundMovingCurve>,
    transforms: bevy::transform::helper::TransformHelper,
    mut platforms: Query<
        (
            &PlaygroundKinematic,
            &Position,
            &mut Rotation,
            Option<&ChildOf>,
            &mut LinearVelocity,
            &mut AngularVelocity,
        ),
        With<PlaygroundMovingObject>,
    >,
) {
    let (elapsed, delta) = if matches!(schedule.as_deref(), Some(bevy_als::AlsPhysicsSchedule::Avian(_))) {
        (time.elapsed_secs(), time.delta_secs())
    } else {
        (
            virtual_time.elapsed_secs(),
            virtual_time
                .delta()
                .min(bevy_als::source_physics_timing::SOURCE_MAX_PHYSICS_DELTA)
                .as_secs_f32(),
        )
    };
    for (kinematic, position, mut rotation, parent, mut linear, mut angular) in &mut platforms {
        match kinematic.kind {
            PlaygroundKinematicKind::MoveSpline {
                duration,
                start,
                end,
            } => {
                let target = crate::playground::moving_object_position(
                    &curve,
                    start,
                    end,
                    elapsed,
                    duration,
                );
                linear.0 = if delta > 0.0 {
                    (target - position.0) / delta
                } else {
                    Vec3::ZERO
                };
                angular.0 = Vec3::ZERO;
            }
            PlaygroundKinematicKind::RotateWorldTime { rate_yzx_radians } => {
                linear.0 = Vec3::ZERO;
                let parent_rotation = parent
                    .and_then(|parent| transforms.compute_global_transform(parent.parent()).ok())
                    .map_or(kinematic.base_transform.rotation, |parent| {
                        parent.rotation()
                    });
                let target = parent_rotation
                    * crate::playground::rotating_object_rotation(
                        rate_yzx_radians,
                        elapsed,
                    );
                let mut difference = (target * rotation.0.conjugate()).normalize();
                if difference.w < 0.0 {
                    difference = -difference;
                }
                if delta > 0.0 {
                    angular.0 = difference.to_scaled_axis() / delta;
                } else {
                    angular.0 = Vec3::ZERO;
                    rotation.0 = target;
                }
            }
        }
    }
}

fn playground_moving_box_physics_bridge(
    mut commands: Commands,
    bodies: Query<(Entity, &PlaygroundBoxCollider, Option<&ChildOf>), Added<PlaygroundBoxCollider>>,
    moving_roots: Query<(), With<PlaygroundMovingObject>>,
) {
    for (entity, collider, parent) in &bodies {
        let Some(&ChildOf(root)) = parent else {
            continue;
        };
        if !moving_roots.contains(root) {
            continue;
        }
        commands.entity(entity).insert((
            Collider::cuboid(
                collider.half_extents.x * 2.0,
                collider.half_extents.y * 2.0,
                collider.half_extents.z * 2.0,
            ),
            CollisionLayers::new(
                [AlsCollisionLayer::Default, AlsCollisionLayer::Visibility],
                LayerMask::ALL,
            ),
        ));
        commands
            .entity(root)
            .insert((RigidBody::Kinematic, PlaygroundPhysicsDriven));
    }
}

fn playground_mesh_physics_bridge(
    mut commands: Commands,
    mut cache: ResMut<PlaygroundMeshColliderCache>,
    roots: Query<
        (Entity, &PlaygroundCollisionSource, &GlobalTransform),
        Without<PlaygroundMeshColliderBuilt>,
    >,
    scene_children: Query<&Children>,
    mesh_parts: Query<(&Mesh3d, &GlobalTransform)>,
    meshes: Res<Assets<Mesh>>,
) {
    for (root, source, root_transform) in &roots {
        if source.kind.is_box() {
            continue;
        }
        let template = if let Some(template) = cache.0.get(&source.mesh_base) {
            template.clone()
        } else {
            let Some(template) = build_mesh_collider_template(
                source,
                root_transform.to_matrix().inverse(),
                root,
                &scene_children,
                &mesh_parts,
                &meshes,
            ) else {
                continue;
            };
            cache.0.insert(source.mesh_base.clone(), template.clone());
            template
        };
        info!(
            "[PGCOL] built source={} kind={:?} verts={} triangles={}",
            source.mesh_base,
            template_kind(source.kind),
            template.vertices,
            template.triangles
        );
        if let Some(complex) = template.complex {
            commands
                .entity(root)
                .insert(bevy_als::extensions::ComplexCollider(complex));
        }
        if let Some(data) = SOURCE_MESH_RAY_DATA.get(&source.mesh_base) {
            commands.entity(root).insert(CookedMeshRayData::new(
                data.clone(),
                SOURCE_MESH_RAY_CULLING[&source.mesh_base],
            ));
        }
        commands.entity(root).insert((
            RigidBody::Static,
            template.collider,
            CollisionLayers::new(
                [AlsCollisionLayer::Default, AlsCollisionLayer::Visibility],
                LayerMask::ALL,
            ),
            PlaygroundMeshColliderBuilt {
                mesh_base: source.mesh_base.clone(),
                kind: source.kind,
                vertices: template.vertices,
                triangles: template.triangles,
                surface_probes: template.surface_probes,
            },
        ));
    }
}

fn template_kind(kind: PlaygroundColliderKind) -> &'static str {
    match kind {
        PlaygroundColliderKind::Box => "box",
        PlaygroundColliderKind::Convex { .. } => "convex",
        PlaygroundColliderKind::TriMesh => "trimesh",
    }
}

fn build_mesh_collider_template(
    source: &PlaygroundCollisionSource,
    root_inverse: Mat4,
    root: Entity,
    scene_children: &Query<&Children>,
    mesh_parts: &Query<(&Mesh3d, &GlobalTransform)>,
    meshes: &Assets<Mesh>,
) -> Option<PlaygroundMeshColliderTemplate> {
    let mut geometry = PlaygroundMeshGeometry {
        vertices: Vec::new(),
        triangles: Vec::new(),
    };
    let mut vertex_indices = HashMap::new();
    append_mesh_geometry(
        root,
        root_inverse,
        scene_children,
        mesh_parts,
        meshes,
        &mut geometry,
        &mut vertex_indices,
    );
    if geometry.vertices.is_empty() || geometry.triangles.is_empty() {
        return None;
    }
    let vertices = geometry.vertices.len();
    let triangles = geometry.triangles.len();
    let surface_probes = if source.mesh_base.starts_with("SM_Als_Stairs_") {
        select_stair_surface_probes(&geometry.vertices, &geometry.triangles)
    } else {
        Vec::new()
    };
    let complex = matches!(source.kind, PlaygroundColliderKind::Convex { .. }).then(|| {
        Collider::try_trimesh(geometry.vertices.clone(), geometry.triangles.clone())
            .expect("Source complex collision surface is invalid")
    });
    let collider = match source.kind {
        PlaygroundColliderKind::Box => return None,
        PlaygroundColliderKind::Convex { source_hulls } => {
            let definitions: HashMap<String, Vec<Vec<[f32; 3]>>> = serde_json::from_str(include_str!(
                "../../../assets/ALSExtras/Environment/Meshes/static_convex_collision.runtime.json"
            )).expect("Generated static convex collision must be valid");
            let hulls = definitions
                .get(&source.mesh_base)
                .expect("Authored static convex collision is missing");
            assert_eq!(
                hulls.len(),
                source_hulls as usize,
                "Authored convex element count differs"
            );
            Collider::compound(
                hulls
                    .iter()
                    .map(|points| {
                        (
                            Vec3::ZERO,
                            Quat::IDENTITY,
                            Collider::convex_hull(
                                &points
                                    .iter()
                                    .copied()
                                    .map(Vec3::from_array)
                                    .collect::<Vec<_>>(),
                            )
                            .expect("Authored static convex hull is invalid"),
                        )
                    })
                    .collect(),
            )
        }
        PlaygroundColliderKind::TriMesh => {
            match Collider::try_trimesh(geometry.vertices, geometry.triangles) {
                Ok(collider) => collider,
                Err(error) => {
                    error!(
                        "[PGCOL] trimesh failed source={} error={error:?}",
                        source.mesh_base
                    );
                    return None;
                }
            }
        }
    };
    Some(PlaygroundMeshColliderTemplate {
        collider,
        complex,
        vertices,
        triangles,
        surface_probes,
    })
}

fn append_mesh_geometry(
    entity: Entity,
    root_inverse: Mat4,
    scene_children: &Query<&Children>,
    mesh_parts: &Query<(&Mesh3d, &GlobalTransform)>,
    meshes: &Assets<Mesh>,
    geometry: &mut PlaygroundMeshGeometry,
    vertex_indices: &mut HashMap<[u32; 3], u32>,
) {
    if let Ok((mesh_handle, transform)) = mesh_parts.get(entity)
        && let Some(mesh) = meshes.get(&mesh_handle.0)
        && let Some(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        && let Some(values) = positions.as_float3()
    {
        let relative = root_inverse * transform.to_matrix();
        let indices = match mesh.indices() {
            Some(bevy::render::mesh::Indices::U32(indices)) => indices.clone(),
            Some(bevy::render::mesh::Indices::U16(indices)) => {
                indices.iter().map(|index| *index as u32).collect()
            }
            None => (0..values.len() as u32).collect(),
        };
        for triangle in indices.chunks_exact(3) {
            let Some(first) = values.get(triangle[0] as usize) else {
                continue;
            };
            let Some(second) = values.get(triangle[1] as usize) else {
                continue;
            };
            let Some(third) = values.get(triangle[2] as usize) else {
                continue;
            };
            let triangle = [
                deduplicated_vertex(
                    relative.transform_point3(Vec3::from(*first)),
                    geometry,
                    vertex_indices,
                ),
                deduplicated_vertex(
                    relative.transform_point3(Vec3::from(*second)),
                    geometry,
                    vertex_indices,
                ),
                deduplicated_vertex(
                    relative.transform_point3(Vec3::from(*third)),
                    geometry,
                    vertex_indices,
                ),
            ];
            if triangle[0] != triangle[1]
                && triangle[1] != triangle[2]
                && triangle[2] != triangle[0]
            {
                geometry.triangles.push(triangle);
            }
        }
    }
    if let Ok(children) = scene_children.get(entity) {
        for child in children.iter() {
            append_mesh_geometry(
                child,
                root_inverse,
                scene_children,
                mesh_parts,
                meshes,
                geometry,
                vertex_indices,
            );
        }
    }
}

fn deduplicated_vertex(
    position: Vec3,
    geometry: &mut PlaygroundMeshGeometry,
    vertex_indices: &mut HashMap<[u32; 3], u32>,
) -> u32 {
    let key = [
        position.x.to_bits(),
        position.y.to_bits(),
        position.z.to_bits(),
    ];
    if let Some(index) = vertex_indices.get(&key) {
        return *index;
    }
    let index = geometry.vertices.len() as u32;
    geometry.vertices.push(position);
    vertex_indices.insert(key, index);
    index
}

fn select_stair_surface_probes(vertices: &[Vec3], triangles: &[[u32; 3]]) -> Vec<Vec3> {
    let mut candidates = triangles
        .iter()
        .filter_map(|triangle| {
            let first = *vertices.get(triangle[0] as usize)?;
            let second = *vertices.get(triangle[1] as usize)?;
            let third = *vertices.get(triangle[2] as usize)?;
            let normal = (second - first).cross(third - first);
            (normal.y > 1e-4).then_some((first + second + third) / 3.0)
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| left.y.total_cmp(&right.y));
    let Some(low) = candidates.first().copied() else {
        return Vec::new();
    };
    let Some(high) = candidates
        .iter()
        .rev()
        .copied()
        .find(|candidate| candidate.y - low.y > 0.2)
    else {
        return Vec::new();
    };
    vec![low, high]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn environment_default_material_reaches_real_shape_contacts() {
        for kind in 0..5 {
            for overridden in [false, true] {
                let mut app = App::new();
                app.add_plugins((
                    MinimalPlugins,
                    AssetPlugin::default(),
                    TransformPlugin,
                    PhysicsPlugins::new(PostUpdate),
                    PlaygroundCollisionPlugin,
                ))
                .init_resource::<Assets<Mesh>>()
                .add_message::<AssetEvent<Mesh>>()
                .insert_resource(Gravity(Vec3::ZERO))
                .insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
                    std::time::Duration::from_secs_f32(1.0 / 60.0),
                ));
                app.finish();
                app.update();
                let shape = match kind {
                    1 => Collider::convex_hull(
                        &[-4.0, 4.0]
                            .into_iter()
                            .flat_map(|x| {
                                [-0.5, 0.5].into_iter().flat_map(move |y| {
                                    [-4.0, 4.0].into_iter().map(move |z| Vec3::new(x, y, z))
                                })
                            })
                            .collect::<Vec<_>>(),
                    )
                    .unwrap(),
                    2 | 4 => Collider::trimesh(
                        vec![
                            Vec3::new(-4.0, 0.5, -4.0),
                            Vec3::new(4.0, 0.5, -4.0),
                            Vec3::new(4.0, 0.5, 4.0),
                            Vec3::new(-4.0, 0.5, 4.0),
                        ],
                        vec![[0, 2, 1], [0, 3, 2]],
                    ),
                    _ => Collider::cuboid(8.0, 1.0, 8.0),
                };
                let ground = app
                    .world_mut()
                    .spawn((
                        if kind == 3 {
                            RigidBody::Kinematic
                        } else {
                            RigidBody::Static
                        },
                        shape,
                        Transform::from_xyz(0.0, -0.5, 0.0),
                    ))
                    .id();
                if kind == 4 {
                    app.world_mut()
                        .entity_mut(ground)
                        .insert(crate::playground::PlaygroundLandscape);
                }
                if overridden {
                    app.world_mut()
                        .entity_mut(ground)
                        .insert((Friction::new(0.2), Restitution::new(0.6)));
                }
                let body = app
                    .world_mut()
                    .spawn((
                        RigidBody::Dynamic,
                        Collider::sphere(0.25),
                        Transform::from_xyz(0.0, 0.2, 0.0),
                        LinearVelocity(Vec3::new(3.0, -4.0, 0.0)),
                        Friction::new(0.7),
                        Restitution::new(0.3),
                    ))
                    .id();
                let mut material = None;
                for _ in 0..4 {
                    app.update();
                    material = app
                        .world_mut()
                        .run_system_once(move |collisions: Collisions| {
                            collisions.get(ground, body).and_then(|pair| {
                                pair.manifolds.first().map(|m| {
                                    (
                                        m.friction,
                                        m.restitution,
                                        pair.total_normal_impulse_magnitude(),
                                    )
                                })
                            })
                        })
                        .unwrap();
                    if material.is_some() {
                        break;
                    }
                }
                let (friction, restitution, impulse) =
                    material.expect("The material fixture did not create a real contact manifold");
                let expected = if overridden { (0.45, 0.45) } else { (0.7, 0.3) };
                println!(
                    "Environment contact kind={kind} override={overridden}: friction={friction} restitution={restitution}"
                );
                assert!(
                    (friction - expected.0).abs() < 1e-6 && (restitution - expected.1).abs() < 1e-6,
                    "Source default and explicit environment materials must reach the contact solver"
                );
                assert_eq!(
                    app.world()
                        .resource::<DefaultFriction>()
                        .0
                        .static_coefficient,
                    0.7
                );
                assert!(impulse > 0.0, "The contact solver did not apply an impulse");
            }
        }
    }
    use bevy_als::als::{
        character::{AlsCharacterBundle, AlsCharacterPlugin},
        control_scheme::{AlsControlScheme, AlsControlSchemeConfig},
        gait::AlsDesiredGait,
        input::AlsInputCommand,
        motion::{AlsMotionPlugin, initial_als_control_scheme_config},
        params::AlsMovementParams,
        state::{Gait, LocomotionState},
        transition::AlsStateTransitionPlugin,
    };
    use bevy::{asset::AssetEvent, ecs::system::RunSystemOnce, time::TimeUpdateStrategy};

    #[test]
    fn current_arch_simple_collision_matches_authored_source_hulls() {
        use avian3d::parry::{
            math::Pose as PhysicsPose,
            query::{ShapeCastOptions, cast_shapes},
        };
        let model = bevy_gltf::gltf::binary::Glb::from_slice(include_bytes!(
            "../../../assets/ALSExtras/Environment/Meshes/SM_Als_Arch_01.glb"
        ))
        .unwrap();
        let data: serde_json::Value = serde_json::from_slice(&model.json).unwrap();
        let bytes = model.bin.unwrap();
        let primitive = &data["meshes"][0]["primitives"][0];
        let accessor_bytes = |index: usize, width: usize| {
            let accessor = &data["accessors"][index];
            let view = &data["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
            let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize
                + accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
            assert_eq!(
                view["byteStride"].as_u64().unwrap_or(width as u64) as usize,
                width
            );
            &bytes[offset..offset + accessor["count"].as_u64().unwrap() as usize * width]
        };
        let positions = accessor_bytes(
            primitive["attributes"]["POSITION"].as_u64().unwrap() as usize,
            12,
        )
        .chunks_exact(12)
        .map(|p| {
            std::array::from_fn::<_, 3, _>(|i| {
                f32::from_le_bytes(p[i * 4..i * 4 + 4].try_into().unwrap())
            })
        })
        .collect::<Vec<_>>();
        let indices: Vec<u32> = accessor_bytes(primitive["indices"].as_u64().unwrap() as usize, 2)
            .chunks_exact(2)
            .map(|i| u16::from_le_bytes(i.try_into().unwrap()) as u32)
            .collect();
        let mut mesh = Mesh::new(
            bevy::render::render_resource::PrimitiveTopology::TriangleList,
            default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin, PlaygroundCollisionPlugin))
            .init_resource::<Assets<Mesh>>()
            .add_message::<AssetEvent<Mesh>>();
        app.finish();
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
        let root = app
            .world_mut()
            .spawn((
                PlaygroundCollisionSource::new("SM_Als_Arch_01"),
                Transform::IDENTITY,
                GlobalTransform::IDENTITY,
            ))
            .id();
        app.world_mut().spawn((
            ChildOf(root),
            Mesh3d(mesh),
            Transform::IDENTITY,
            GlobalTransform::IDENTITY,
        ));
        app.update();
        let actual = app.world().get::<Collider>(root).unwrap();
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/physics/arch_convex_shapes.json"
        ))
        .unwrap();
        let source = Collider::compound(
            fixture["convexes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|hull| {
                    let points = hull
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(|p| {
                            Vec3::new(
                                p[0].as_f64().unwrap() as f32,
                                p[1].as_f64().unwrap() as f32,
                                p[2].as_f64().unwrap() as f32,
                            )
                        })
                        .collect::<Vec<_>>();
                    (
                        Vec3::ZERO,
                        Quat::IDENTITY,
                        Collider::convex_hull(&points).unwrap(),
                    )
                })
                .collect(),
        );
        let capsule = Collider::capsule(0.3, 1.2);
        let options = ShapeCastOptions {
            max_time_of_impact: 3.0,
            ..default()
        };
        for position in [
            Vec3::new(0.0, 1.1, -1.5),
            Vec3::new(-0.7, 0.9, -1.5),
            Vec3::new(0.7, 0.9, -1.5),
        ] {
            let pose = PhysicsPose::from_translation(position);
            let expected = cast_shapes(
                &PhysicsPose::IDENTITY,
                Vec3::ZERO,
                source.shape().as_ref(),
                &pose,
                Vec3::Z,
                capsule.shape().as_ref(),
                options,
            )
            .unwrap()
            .map(|h| h.time_of_impact);
            let observed = cast_shapes(
                &PhysicsPose::IDENTITY,
                Vec3::ZERO,
                actual.shape().as_ref(),
                &pose,
                Vec3::Z,
                capsule.shape().as_ref(),
                options,
            )
            .unwrap()
            .map(|h| h.time_of_impact);
            assert_eq!(
                observed.is_some(),
                expected.is_some(),
                "Source blocking state differs at {position:?}"
            );
            if let (Some(expected), Some(observed)) = (expected, observed) {
                assert!(
                    (observed - expected).abs() < 0.001,
                    "Current Arch capsule sweep differs at {position:?}: source={expected}, target={observed}"
                );
            }
        }
    }

    #[test]
    fn current_convex_foot_queries_use_the_source_complex_surface() {
        use avian3d::parry::{
            query::{Ray, RayCast},
            shape::TypedShape,
        };
        for name in [
            "SM_Als_Arch_01",
            "SM_Als_Arch_02",
            "SM_Als_Barrel",
            "SM_Als_Barrel_Broken",
            "SM_Als_Bench",
        ] {
            let folder = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("assets/ALSExtras/Environment/Meshes");
            let model_bytes = std::fs::read(folder.join(format!("{name}.glb"))).unwrap();
            let model = bevy_gltf::gltf::binary::Glb::from_slice(&model_bytes).unwrap();
            let data: serde_json::Value = serde_json::from_slice(&model.json).unwrap();
            let bytes = model.bin.unwrap();
            let primitive = &data["meshes"][0]["primitives"][0];
            let accessor = |index: usize, width: usize| {
                let item = &data["accessors"][index];
                let view = &data["bufferViews"][item["bufferView"].as_u64().unwrap() as usize];
                let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize
                    + item["byteOffset"].as_u64().unwrap_or(0) as usize;
                assert_eq!(
                    view["byteStride"].as_u64().unwrap_or(width as u64) as usize,
                    width
                );
                &bytes[offset..offset + item["count"].as_u64().unwrap() as usize * width]
            };
            let positions = accessor(
                primitive["attributes"]["POSITION"].as_u64().unwrap() as usize,
                12,
            )
            .chunks_exact(12)
            .map(|p| {
                std::array::from_fn::<_, 3, _>(|i| {
                    f32::from_le_bytes(p[i * 4..i * 4 + 4].try_into().unwrap())
                })
            })
            .collect::<Vec<_>>();
            let index = primitive["indices"].as_u64().unwrap() as usize;
            let indices: Vec<u32> =
                match data["accessors"][index]["componentType"].as_u64().unwrap() {
                    5123 => accessor(index, 2)
                        .chunks_exact(2)
                        .map(|p| u16::from_le_bytes(p.try_into().unwrap()) as u32)
                        .collect(),
                    5125 => accessor(index, 4)
                        .chunks_exact(4)
                        .map(|p| u32::from_le_bytes(p.try_into().unwrap()))
                        .collect(),
                    _ => panic!("Unsupported source mesh index type"),
                };
            let complex = Collider::try_trimesh(
                positions.iter().copied().map(Vec3::from_array).collect(),
                indices
                    .chunks_exact(3)
                    .map(|p| [p[0], p[1], p[2]])
                    .collect(),
            )
            .unwrap();
            let bounds = complex.aabb(Vec3::ZERO, Quat::IDENTITY, 0.0);
            let mut mesh = Mesh::new(
                bevy::render::render_resource::PrimitiveTopology::TriangleList,
                default(),
            );
            mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
            mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));
            let (mut app, character, avatar) = crate::test_support::loaded_character();
            PlaygroundCollisionPlugin.build(&mut app);
            app.world_mut()
                .entity_mut(character)
                .insert((RigidBody::Kinematic, bevy_tnua::TnuaToggle::Disabled));
            let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
            let transform = Transform::from_xyz(0.0, 10.0, 0.0);
            let root = app
                .world_mut()
                .spawn((
                    PlaygroundCollisionSource::new(name),
                    transform,
                    GlobalTransform::from(transform),
                ))
                .id();
            app.world_mut().spawn((
                ChildOf(root),
                Mesh3d(mesh),
                Transform::IDENTITY,
                GlobalTransform::from(transform),
            ));
            app.update();
            app.world_mut()
                .get_mut::<bevy_als::control_rig_apply::AlsControlRigInput>(character)
                .unwrap()
                .foot_transforms_valid = true;
            app.world_mut()
                .run_system_once(bevy_als::control_rig_apply::publish_control_rig_inputs)
                .unwrap();
            let player = app
                .world()
                .get::<bevy_animation_graph::core::animated_scene::AnimatedSceneInstance>(avatar)
                .unwrap()
                .player_entity();
            let query = app
                .world()
                .get::<bevy_als::extensions::SourceAnimationInputs>(player)
                .unwrap()
                .rig_query_context()
                .expect("Actual source rig query context is missing");
            assert!(query.world.mesh_ray_data[&root].cull_back_faces);
            let TypedShape::TriMesh(surface) = complex.shape().as_typed_shape() else {
                panic!("Expected source mesh");
            };
            let offset = |hit: Option<(f32, Vec3)>, root_y: f32| {
                hit.filter(|(_, normal)| normal.y >= 45.0_f32.to_radians().cos())
                    .map_or((0.0, Vec3::Y), |(height, normal)| {
                        (height - root_y + 0.135 / normal.y - 0.135, normal)
                    })
            };
            for x in 1..20 {
                for z in 1..20 {
                    let px = bounds.min.x + (bounds.max.x - bounds.min.x) * x as f32 / 20.0;
                    let pz = bounds.min.z + (bounds.max.z - bounds.min.z) * z as f32 / 20.0;
                    let probe = Vec3::new(px, bounds.max.y + 10.5, pz);
                    let simple = app.world().get::<Collider>(root).unwrap().cast_ray(
                        Vec3::Y * 10.0,
                        Quat::IDENTITY,
                        probe,
                        Vec3::NEG_Y,
                        4.0,
                        true,
                    );
                    let Some((distance, _)) = simple else {
                        continue;
                    };
                    let root_y = probe.y - distance;
                    let start = Vec3::new(px, root_y + 0.5, pz);
                    let ray = Ray::new(start - Vec3::Y * 10.0, Vec3::NEG_Y);
                    let expected = surface
                        .triangles()
                        .filter_map(|face| {
                            let normal = (face.b - face.a).cross(face.c - face.a);
                            if ray.dir.dot(normal) > 0.0 {
                                return None;
                            }
                            face.cast_local_ray_and_get_normal(&ray, 1.3, true)
                        })
                        .min_by(|left, right| left.time_of_impact.total_cmp(&right.time_of_impact))
                        .map(|hit| (start.y - hit.time_of_impact, hit.normal));
                    let observed = query
                        .trace(
                            start.as_dvec3(),
                            (start - Vec3::Y * 1.3).as_dvec3(),
                            "ECC_Visibility",
                            true,
                        )
                        .unwrap()
                        .map(|(point, normal)| (point.y as f32, normal.as_vec3()));
                    let expected = offset(expected, root_y);
                    let observed = offset(observed, root_y);
                    assert!(
                        (expected.0 - observed.0).abs() < 0.01
                            && expected.1.angle_between(observed.1) < 1.0_f32.to_radians(),
                        "Source complex foot offset differs: mesh={name}, x={px}, z={pz}, root={root_y}, source={expected:?}, target={observed:?}"
                    );
                }
            }
            if name == "SM_Als_Barrel" {
                use bevy::math::DVec3;
                let inside = DVec3::new(0.025, 10.8, -0.035);
                let end = inside.with_y(9.5);
                let trace = |query: &bevy_als::control_rig_query::RigQueryContext| {
                    assert!(
                        query
                            .trace(inside, end, "ECC_Visibility", true)
                            .unwrap()
                            .is_none()
                    );
                    assert!(
                        query
                            .trace(inside.with_y(12.8), end, "ECC_Visibility", true)
                            .unwrap()
                            .is_some()
                    );
                };
                trace(&query);
                app.world_mut().get_mut::<Transform>(root).unwrap().scale = Vec3::splat(1.5);
                app.update();
                app.update();
                app.world_mut()
                    .get_mut::<bevy_als::control_rig_apply::AlsControlRigInput>(character)
                    .unwrap()
                    .foot_transforms_valid = true;
                let publish = |app: &mut App| {
                    app.world_mut()
                        .run_system_once(bevy_als::control_rig_apply::publish_control_rig_inputs)
                        .unwrap();
                    app.world()
                        .get::<bevy_als::extensions::SourceAnimationInputs>(player)
                        .unwrap()
                        .rig_query_context()
                        .unwrap()
                };
                let scaled = publish(&mut app);
                assert_eq!(scaled.world.colliders[&root].2.scale(), Vec3::splat(1.5));
                trace(&scaled);
                app.world_mut()
                    .get_mut::<CookedMeshRayData>(root)
                    .unwrap()
                    .cull_back_faces = false;
                assert!(
                    publish(&mut app)
                        .trace(inside, end, "ECC_Visibility", true)
                        .unwrap()
                        .is_some()
                );
                app.world_mut()
                    .get_mut::<CookedMeshRayData>(root)
                    .unwrap()
                    .cull_back_faces = true;
                trace(&publish(&mut app));
            }
        }
    }

    #[test]
    fn rotating_actor_world_time_and_yaw_match_source() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            PhysicsPlugins::new(PostUpdate),
            PlaygroundCollisionPlugin,
            bevy_als::source_physics_timing::SourcePhysicsTimingPlugin,
        ))
        .init_resource::<Assets<Mesh>>()
        .add_message::<AssetEvent<Mesh>>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(0.025),
        ));
        app.finish();
        for _ in 0..40 {
            app.update();
        }
        let transform = Transform::from_xyz(13.5, -0.5, -24.0)
            .with_rotation(Quat::from_rotation_y(0.3) * Quat::from_rotation_z(0.2))
            .with_scale(Vec3::new(1.5, 0.8, 2.0));
        let anchor = app
            .world_mut()
            .spawn((transform, GlobalTransform::from(transform)))
            .id();
        let mesh = app
            .world_mut()
            .spawn((
                PlaygroundMovingObject,
                PlaygroundPhysicsDriven,
                RigidBody::Kinematic,
                PlaygroundKinematic {
                    base_transform: transform,
                    kind: PlaygroundKinematicKind::RotateWorldTime {
                        rate_yzx_radians: bevy::math::DVec3::new(-100.0_f64.to_radians(), 0.0, 0.0),
                    },
                },
                ChildOf(anchor),
                Transform::IDENTITY,
                GlobalTransform::from(transform),
            ))
            .id();
        for frame in 0..240 {
            let delta = [1.0 / 120.0, 1.0 / 60.0, 0.05, 0.013, 0.0][frame % 5];
            if frame == 50 {
                let mut rotation = app.world_mut().get_mut::<Rotation>(mesh).unwrap();
                rotation.0 = -rotation.0;
            }
            app.insert_resource(TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f32(delta),
            ));
            app.update();
            let time = app.world().resource::<Time>().elapsed_secs();
            let expected =
                transform.rotation * Quat::from_rotation_y(-100.0_f32.to_radians() * time);
            let actual = app.world().get::<Rotation>(mesh).unwrap().0;
            let angular_step = app.world().get::<AngularVelocity>(mesh).unwrap().length()
                * delta.min(bevy_als::source_physics_timing::SOURCE_MAX_PHYSICS_DELTA.as_secs_f32());
            assert!(
                angular_step <= std::f32::consts::PI + 1e-4,
                "Rotation did not use the shortest quaternion arc"
            );
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                assert!(
                    (actual * axis).abs_diff_eq(expected * axis, 1e-4),
                    "Rotating mesh differs from source world time: time={time} actual={actual:?} expected={expected:?}"
                );
            }
            assert_eq!(
                app.world().get::<Transform>(anchor).unwrap().rotation,
                transform.rotation
            );
        }
    }

    #[test]
    fn moving_platform_reaches_the_source_position_after_late_start_and_variable_steps() {
        use crate::playground::{PlaygroundKinematic, PlaygroundKinematicKind};
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            PhysicsPlugins::new(PostUpdate),
            PlaygroundCollisionPlugin,
            bevy_als::source_physics_timing::SourcePhysicsTimingPlugin,
        ))
        .init_resource::<Assets<Mesh>>()
        .add_message::<AssetEvent<Mesh>>()
        .insert_resource(crate::playground::load_moving_object_curve().unwrap())
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(0.025),
        ))
        .add_systems(Update, playground_moving_object_system);
        app.finish();
        for _ in 0..40 {
            app.update();
        }
        let start = Vec3::new(19.0, 1.0, -28.75);
        let end = start - Vec3::X * 6.5;
        let transform = Transform::from_translation(start);
        let anchor = app
            .world_mut()
            .spawn((transform, GlobalTransform::from(transform)))
            .id();
        let platform = app
            .world_mut()
            .spawn((
                PlaygroundMovingObject,
                PlaygroundPhysicsDriven,
                PlaygroundKinematic {
                    base_transform: transform,
                    kind: PlaygroundKinematicKind::MoveSpline {
                        duration: 1.75,
                        start,
                        end,
                    },
                },
                RigidBody::Kinematic,
                ChildOf(anchor),
                Transform::IDENTITY,
                GlobalTransform::from(transform),
            ))
            .id();
        for frame in 0..240 {
            let delta = [1.0 / 120.0, 1.0 / 60.0, 0.05, 0.013, 0.0][frame % 5];
            *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
                TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f32(delta));
            app.update();
            let t = app.world().resource::<Time>().elapsed_secs();
            let expected = crate::playground::moving_object_position(
                app.world()
                    .resource::<crate::playground::PlaygroundMovingCurve>(),
                start,
                end,
                t,
                1.75,
            );
            let actual = app.world().get::<Position>(platform).unwrap().0;
            assert!(
                actual.abs_diff_eq(expected, 1e-4),
                "Platform drifted at frame={frame}: actual={actual:?} expected={expected:?}"
            );
            assert_eq!(
                app.world().get::<Transform>(anchor).unwrap().translation,
                start
            );
        }
    }

    #[test]
    fn imported_stair_descent_exits_initial_contact_without_entering_the_stair() {
        let model = bevy_gltf::gltf::binary::Glb::from_slice(include_bytes!(
            "../../../assets/ALSExtras/Environment/Meshes/SM_Als_Stairs_03.glb"
        ))
        .unwrap();
        let data: serde_json::Value = serde_json::from_slice(&model.json).unwrap();
        let bytes = model.bin.unwrap();
        let primitive = &data["meshes"][0]["primitives"][0];
        let accessor_bytes = |index: usize, width: usize| {
            let accessor = &data["accessors"][index];
            let view = &data["bufferViews"][accessor["bufferView"].as_u64().unwrap() as usize];
            let offset = view["byteOffset"].as_u64().unwrap_or(0) as usize
                + accessor["byteOffset"].as_u64().unwrap_or(0) as usize;
            assert_eq!(
                view["byteStride"].as_u64().unwrap_or(width as u64) as usize,
                width
            );
            &bytes[offset..offset + accessor["count"].as_u64().unwrap() as usize * width]
        };
        let positions = accessor_bytes(
            primitive["attributes"]["POSITION"].as_u64().unwrap() as usize,
            12,
        )
        .chunks_exact(12)
        .map(|point| {
            std::array::from_fn::<_, 3, _>(|i| {
                f32::from_le_bytes(point[i * 4..i * 4 + 4].try_into().unwrap())
            })
        })
        .collect::<Vec<_>>();
        let indices = accessor_bytes(primitive["indices"].as_u64().unwrap() as usize, 2)
            .chunks_exact(2)
            .map(|index| u16::from_le_bytes(index.try_into().unwrap()) as u32)
            .collect();
        let mut mesh = Mesh::new(
            bevy::render::render_resource::PrimitiveTopology::TriangleList,
            default(),
        );
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_indices(bevy::render::mesh::Indices::U32(indices));
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            TransformPlugin,
            PhysicsPlugins::new(PostUpdate),
            bevy_tnua_avian3d::TnuaAvian3dPlugin::new(Update),
            bevy_tnua::TnuaControllerPlugin::<AlsControlScheme>::new(Update),
            AlsStateTransitionPlugin,
            AlsCharacterPlugin,
            AlsMotionPlugin,
            PlaygroundCollisionPlugin,
            bevy_als::source_physics_timing::SourcePhysicsTimingPlugin,
        ))
        .init_resource::<Assets<Mesh>>()
        .add_message::<AssetEvent<Mesh>>()
        .insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(0.0170989),
        ));
        app.finish();
        let source = Transform::from_xyz(-30.0, 0.36, -30.0);
        let mesh = app.world_mut().resource_mut::<Assets<Mesh>>().add(mesh);
        let root = app
            .world_mut()
            .spawn((
                PlaygroundCollisionSource::new("SM_Als_Stairs_03"),
                source,
                GlobalTransform::from(source),
            ))
            .id();
        app.world_mut().spawn((
            ChildOf(root),
            Mesh3d(mesh),
            Transform::IDENTITY,
            GlobalTransform::from(source),
        ));
        app.world_mut().spawn((
            RigidBody::Static,
            Collider::cuboid(80.0, 0.2, 80.0),
            Transform::from_xyz(0.0, -0.1, -30.0),
        ));
        let barrier = app
            .world_mut()
            .spawn((
                RigidBody::Static,
                Collider::cuboid(6.67, 3.0, 0.2),
                Transform::from_xyz(-26.665, 6.4, -34.5),
            ))
            .id();
        let params = AlsMovementParams::default();
        let config = initial_als_control_scheme_config(&params);
        let params = app
            .world_mut()
            .resource_mut::<Assets<AlsMovementParams>>()
            .add(params);
        let start = Vec3::new(-26.665, 6.321502, -35.25028);
        let mut bundle = AlsCharacterBundle::new(
            params,
            &mut app
                .world_mut()
                .resource_mut::<Assets<AlsControlSchemeConfig>>(),
            config,
        );
        bundle.transform.translation = start;
        let actor = app.world_mut().spawn(bundle).id();
        for _ in 0..30 {
            app.update();
        }
        assert!(
            app.world()
                .get::<PlaygroundMeshColliderBuilt>(root)
                .is_some()
        );
        let capsule = app.world().get::<Collider>(actor).unwrap().clone();
        app.world_mut()
            .run_system_once(move |spatial: SceneQuery| {
                let filter = SpatialQueryFilter::from_excluded_entities([actor]);
                let config = ShapeCastConfig::from_max_distance(6.5 * 0.012236);
                let hit = spatial
                    .cast_shape(&capsule, start, Quat::IDENTITY, Dir3::Z, &config, &filter)
                    .unwrap();
                assert_eq!(hit.distance, 0.0);
                assert!(hit.normal1.z > 0.99);
                assert!(((hit.point1 - hit.point2).dot(hit.normal1) - 0.000278).abs() < 1e-5);
                let outward = ShapeCastConfig {
                    ignore_origin_penetration: true,
                    ..config
                };
                assert!(
                    spatial
                        .cast_shape(&capsule, start, Quat::IDENTITY, Dir3::Z, &outward, &filter)
                        .is_none()
                );
                assert!(
                    spatial
                        .cast_shape(
                            &capsule,
                            start,
                            Quat::IDENTITY,
                            Dir3::NEG_Z,
                            &outward,
                            &filter
                        )
                        .is_some()
                );
                let onward = ShapeCastConfig {
                    max_distance: 1.0,
                    ..outward
                };
                let hit = spatial
                    .cast_shape(&capsule, start, Quat::IDENTITY, Dir3::Z, &onward, &filter)
                    .unwrap();
                assert_eq!(hit.entity, barrier);
                assert!(hit.distance > 0.3 && hit.distance < 0.4);
            })
            .unwrap();
        app.world_mut()
            .get_mut::<Transform>(actor)
            .unwrap()
            .translation = start;
        app.world_mut().get_mut::<Position>(actor).unwrap().0 = start;
        app.world_mut().get_mut::<LinearVelocity>(actor).unwrap().0 = Vec3::Z * 6.5;
        *app.world_mut().get_mut::<LocomotionState>(actor).unwrap() = LocomotionState::Grounded;
        app.world_mut().get_mut::<AlsDesiredGait>(actor).unwrap().0 = Gait::Sprinting;
        app.world_mut()
            .get_mut::<AlsInputCommand>(actor)
            .unwrap()
            .move_dir = Vec2::Y;
        app.world_mut()
            .get_mut::<AlsInputCommand>(actor)
            .unwrap()
            .sprint_held = true;
        app.world_mut()
            .get_mut::<bevy_als::als::motion::AlsViewYaw>(actor)
            .unwrap()
            .0 = std::f32::consts::PI;
        *app.world_mut().resource_mut::<TimeUpdateStrategy>() =
            TimeUpdateStrategy::ManualDuration(std::time::Duration::from_secs_f32(0.012236));
        app.update();
        let velocity = app.world().get::<LinearVelocity>(actor).unwrap().0;
        let position = app.world().get::<Position>(actor).unwrap().0;
        assert_eq!(
            *app.world().get::<LocomotionState>(actor).unwrap(),
            LocomotionState::Grounded
        );
        assert!(
            (velocity.z - 6.5).abs() < 0.1 && position.z - start.z > 0.075,
            "Recorded overlap stopped motion: position={position:?} velocity={velocity:?}"
        );
    }
}
