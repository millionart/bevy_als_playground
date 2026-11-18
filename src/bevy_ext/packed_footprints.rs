use std::f32::consts::FRAC_PI_2;

use bevy::{
    pbr::decal::{ForwardDecal, ForwardDecalMaterial},
    prelude::*,
};
use bevy_als::{
    character_assembly::AlsCharacterAvatar,
    extensions::overlay_materials::{AvatarMaterialRoot, AvatarMaterialSet},
    footprint::{Footprint, FootprintLibrary, FootprintSet, footprint_opacity},
};

use crate::projected_decal::{
    NoDecalReceiver, PackedDecal, PackedDecalMaterial, PackedDecalPrepareSet,
};

type ForwardFootprintMaterial = ForwardDecalMaterial<StandardMaterial>;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PackedFootprintSet;

/// Use the Playground's packed material renderer for ALS footprints.
pub struct PackedFootprintsPlugin;

impl Plugin for PackedFootprintsPlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(Update, PackedDecalPrepareSet.after(AvatarMaterialSet))
            .add_systems(
                Update,
                (exclude_character_receivers, inherit_receiver_exclusion)
                    .chain()
                    .before(PackedDecalPrepareSet),
            )
            .add_systems(
                PostUpdate,
                (fade_packed_footprints, use_packed_footprints)
                    .chain()
                    .in_set(PackedFootprintSet)
                    .after(FootprintSet)
                    .before(PackedDecalPrepareSet),
            );
    }
}

fn exclude_character_receivers(
    mut commands: Commands,
    avatars: Query<Entity, (With<AlsCharacterAvatar>, Without<NoDecalReceiver>)>,
    server: Res<AssetServer>,
    libraries: Res<Assets<FootprintLibrary>>,
) {
    let Some(library) = server
        .get_handle("ALS/Data/Footstep/footprint.runtime.json")
        .and_then(|handle| libraries.get(&handle))
    else {
        return;
    };
    if !library.character_receives_decals {
        for avatar in &avatars {
            commands.entity(avatar).insert(NoDecalReceiver);
        }
    }
}

fn inherit_receiver_exclusion(
    mut commands: Commands,
    roots: Query<Entity, (With<AvatarMaterialRoot>, With<NoDecalReceiver>)>,
    material_roots: Query<(), With<AvatarMaterialRoot>>,
    children: Query<&Children>,
    meshes: Query<Has<NoDecalReceiver>, With<Mesh3d>>,
) {
    for root in &roots {
        let mut stack = vec![root];
        while let Some(entity) = stack.pop() {
            if entity != root && material_roots.contains(entity) {
                continue;
            }
            if meshes.get(entity) == Ok(false) {
                commands.entity(entity).insert(NoDecalReceiver);
            }
            if let Ok(children) = children.get(entity) {
                stack.extend(children.iter());
            }
        }
    }
}

fn use_packed_footprints(
    mut commands: Commands,
    footprints: Query<
        (
            Entity,
            &Transform,
            &GlobalTransform,
            &MeshMaterial3d<ForwardFootprintMaterial>,
        ),
        (With<Footprint>, Without<PackedDecal>),
    >,
    forward: Res<Assets<ForwardFootprintMaterial>>,
    mut packed: ResMut<Assets<PackedDecalMaterial>>,
) {
    let to_packed = Transform::from_rotation(Quat::from_rotation_x(-FRAC_PI_2));
    for (entity, local, global, handle) in &footprints {
        let Some(material) = forward.get(&handle.0) else {
            continue;
        };
        let Some(texture) = material.base.base_color_texture.clone() else {
            continue;
        };
        let handle = packed.add(PackedDecalMaterial {
            color: material.base.base_color.to_linear().to_vec4(),
            roughness_scale: material.base.perceptual_roughness,
            metallic: material.base.metallic,
            specular: material.base.reflectance,
            texture,
        });
        commands
            .entity(entity)
            .remove::<(
                ForwardDecal,
                Mesh3d,
                MeshMaterial3d<ForwardFootprintMaterial>,
            )>()
            .insert((
                PackedDecal(handle),
                GlobalTransform::from(*local)
                    .mul_transform(to_packed)
                    .compute_transform(),
                global.mul_transform(to_packed),
            ));
    }
}

fn fade_packed_footprints(
    time: Res<Time>,
    footprints: Query<(&Footprint, &PackedDecal)>,
    mut materials: ResMut<Assets<PackedDecalMaterial>>,
) {
    for (footprint, decal) in &footprints {
        let age = time.elapsed_secs_f64() - footprint.created_at;
        if age > footprint.lifetime.x as f64
            && let Some(mut material) = materials.get_mut(&decal.0)
        {
            material.color.w = footprint.opacity * footprint_opacity(age, footprint.lifetime);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy_als::extensions::overlay_materials::{
        AvatarMaterialApplied, AvatarMaterialOverride, OverlayMaterialsPlugin,
    };

    fn material_world() -> App {
        let mut app = App::new();
        app.add_plugins(OverlayMaterialsPlugin)
            .add_systems(Update, inherit_receiver_exclusion);
        app
    }

    #[test]
    fn no_decal_policy_reaches_meshes_without_material_overrides() {
        for empty_override in [false, true] {
            let mut app = material_world();
            let world = app.world_mut();
            let mut materials = Assets::<StandardMaterial>::default();
            let source = materials.add(StandardMaterial::default());
            world.insert_resource(materials);
            let character = world.spawn_empty().id();
            let root = world
                .spawn(bevy_als::character_assembly::AlsCharacterAvatar { character })
                .id();
            if empty_override {
                world
                    .entity_mut(root)
                    .insert(AvatarMaterialOverride(Vec::new()));
            }
            let body = world
                .spawn((
                    ChildOf(root),
                    Mesh3d::default(),
                    MeshMaterial3d(source.clone()),
                ))
                .id();
            let pending = world.spawn((ChildOf(root), Mesh3d::default())).id();
            let nested_root = world
                .spawn((ChildOf(root), AvatarMaterialOverride(vec![source.clone()])))
                .id();
            let nested = world
                .spawn((
                    ChildOf(nested_root),
                    Mesh3d::default(),
                    MeshMaterial3d(source.clone()),
                ))
                .id();
            let environment = world
                .spawn((Mesh3d::default(), MeshMaterial3d(source.clone())))
                .id();
            let camera = world
                .spawn((
                    ChildOf(root),
                    bevy_als::character_assembly::AlsCharacterCamera { character },
                ))
                .id();
            let camera_mesh = world
                .spawn((
                    ChildOf(camera),
                    Mesh3d::default(),
                    MeshMaterial3d(source.clone()),
                ))
                .id();
            world.run_schedule(Update);
            assert!(world.get::<NoDecalReceiver>(body).is_none());
            world.entity_mut(root).insert(NoDecalReceiver);
            world.run_schedule(Update);
            assert!(
                world.get::<NoDecalReceiver>(body).is_some(),
                "Decal policy must not depend on optional overrides"
            );
            assert!(
                world.get::<NoDecalReceiver>(pending).is_some(),
                "The policy must reach a mesh before its material arrives"
            );
            assert!(world.get::<NoDecalReceiver>(nested).is_none());
            assert!(world.get::<NoDecalReceiver>(environment).is_none());
            assert!(
                world.get::<NoDecalReceiver>(camera_mesh).is_none(),
                "The source Camera keeps its own decal policy without an override"
            );
            assert_eq!(
                world
                    .get::<MeshMaterial3d<StandardMaterial>>(body)
                    .unwrap()
                    .0,
                source
            );
            assert!(world.get::<AvatarMaterialApplied>(root).is_none());
            world.clear_trackers();
            world.increment_change_tick();
            world.run_schedule(Update);
            assert!(
                !world
                    .entity(body)
                    .get_ref::<NoDecalReceiver>()
                    .unwrap()
                    .is_changed()
            );
            assert_eq!(
                world
                    .get::<MeshMaterial3d<StandardMaterial>>(body)
                    .unwrap()
                    .0,
                source
            );
        }
    }

    #[test]
    fn non_receiving_avatar_preserves_material_values_and_nested_receivers() {
        let mut app = material_world();
        let world = app.world_mut();
        let mut materials = Assets::<StandardMaterial>::default();
        let source = materials.add(StandardMaterial::default());
        let color = Color::linear_rgb(0.2, 0.4, 0.8);
        let target = materials.add(StandardMaterial {
            base_color: color,
            metallic: 0.25,
            perceptual_roughness: 0.7,
            ..default()
        });
        world.insert_resource(materials);
        let root = world
            .spawn((
                AvatarMaterialOverride(vec![target.clone(), target.clone()]),
                NoDecalReceiver,
            ))
            .id();
        let body = world
            .spawn((
                ChildOf(root),
                Mesh3d::default(),
                MeshMaterial3d(source.clone()),
            ))
            .id();
        let second = world
            .spawn((
                ChildOf(root),
                Mesh3d::default(),
                MeshMaterial3d(source.clone()),
            ))
            .id();
        let other = world
            .spawn((ChildOf(root), AvatarMaterialOverride(vec![target.clone()])))
            .id();
        let nested = world
            .spawn((ChildOf(other), Mesh3d::default(), MeshMaterial3d(source)))
            .id();
        world.run_schedule(Update);
        assert!(world.get::<NoDecalReceiver>(body).is_some());
        assert!(world.get::<NoDecalReceiver>(second).is_some());
        assert!(world.get::<NoDecalReceiver>(nested).is_none());
        assert_eq!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(body)
                .unwrap()
                .0,
            target
        );
        assert_eq!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(second)
                .unwrap()
                .0,
            target
        );
        assert_eq!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(nested)
                .unwrap()
                .0,
            target
        );
        world.clear_trackers();
        world.increment_change_tick();
        world.run_schedule(Update);
        assert!(
            !world
                .entity(body)
                .get_ref::<NoDecalReceiver>()
                .unwrap()
                .is_changed()
        );
        assert!(
            !world
                .entity(body)
                .get_ref::<MeshMaterial3d<StandardMaterial>>()
                .unwrap()
                .is_changed()
        );
    }

    #[test]
    fn late_no_decal_marker_reaches_already_processed_meshes() {
        let mut app = material_world();
        let world = app.world_mut();
        let mut materials = Assets::<StandardMaterial>::default();
        let source = materials.add(StandardMaterial::default());
        let target = materials.add(StandardMaterial::default());
        world.insert_resource(materials);
        let root = world
            .spawn(AvatarMaterialOverride(vec![target.clone()]))
            .id();
        let body = world
            .spawn((
                ChildOf(root),
                Mesh3d::default(),
                MeshMaterial3d(source.clone()),
            ))
            .id();
        let nested_root = world
            .spawn((ChildOf(root), AvatarMaterialOverride(vec![target.clone()])))
            .id();
        let nested = world
            .spawn((
                ChildOf(nested_root),
                Mesh3d::default(),
                MeshMaterial3d(source),
            ))
            .id();
        world.run_schedule(Update);
        assert!(world.get::<AvatarMaterialApplied>(root).is_some());
        assert!(world.get::<NoDecalReceiver>(body).is_none());

        world.entity_mut(root).insert(NoDecalReceiver);
        world.run_schedule(Update);
        assert!(world.get::<NoDecalReceiver>(body).is_some());
        assert!(world.get::<NoDecalReceiver>(nested).is_none());
        assert_eq!(
            world
                .get::<MeshMaterial3d<StandardMaterial>>(body)
                .unwrap()
                .0,
            target
        );
    }
}
