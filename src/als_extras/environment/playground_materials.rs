use bevy::asset::{AssetServer, RenderAssetUsages};
use bevy::image::{
    CompressedImageFormats, Image, ImageSampler, ImageSamplerDescriptor, ImageType,
    TextureFormatPixelInfo,
};
use bevy::pbr::{ExtendedMaterial, MaterialExtension, MaterialPlugin};
use bevy::prelude::*;
use bevy::reflect::TypePath;
use bevy::render::render_resource::{AsBindGroup, ShaderType, TextureFormat};
use bevy::shader::ShaderRef;

pub type PlaygroundTilesMaterial = ExtendedMaterial<StandardMaterial, PlaygroundTilesExtension>;

#[derive(Asset, TypePath, AsBindGroup, Debug, Clone)]
pub struct PlaygroundTilesExtension {
    #[storage(110, read_only, visibility(fragment))]
    pub decals: Handle<bevy::render::storage::ShaderBuffer>,
    #[storage(111, visibility(fragment))]
    pub probe: Handle<bevy::render::storage::ShaderBuffer>,
    #[uniform(100)]
    pub settings: PlaygroundTilesSettings,
    #[texture(101)]
    #[sampler(102)]
    pub grid_texture: Handle<Image>,
}

#[derive(Clone, Debug, ShaderType)]
pub struct PlaygroundTilesSettings {
    pub side_color_a: Vec4,
    pub top_color_a: Vec4,
    pub side_color_b: Vec4,
    pub top_color_b: Vec4,
    pub tiling_and_mode: Vec4,
}

impl MaterialExtension for PlaygroundTilesExtension {
    fn fragment_shader() -> ShaderRef {
        "ALSExtras/Environment/Materials/playground_tiles.wesl".into()
    }
    fn deferred_fragment_shader() -> ShaderRef {
        "ALSExtras/Environment/Materials/playground_tiles.wesl".into()
    }
    fn specialize(
        _: &bevy::pbr::MaterialExtensionPipeline,
        descriptor: &mut bevy::render::render_resource::RenderPipelineDescriptor,
        _: &bevy::mesh::MeshVertexBufferLayoutRef,
        _: bevy::pbr::MaterialExtensionKey<Self>,
    ) -> Result<(), bevy::render::render_resource::SpecializedMeshPipelineError> {
        descriptor.label = Some(
            format!(
                "als_playground_receiver_{}",
                descriptor.label.as_deref().unwrap_or_default()
            )
            .into(),
        );
        Ok(())
    }
}

pub struct PlaygroundMaterialsPlugin;

impl Plugin for PlaygroundMaterialsPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<crate::projected_decal::PackedDecalPlugin>() {
            app.add_plugins(crate::projected_decal::PackedDecalPlugin);
        }
        app.add_plugins(MaterialPlugin::<PlaygroundTilesMaterial>::default())
            .init_resource::<PlaygroundMaterialLibrary>()
            .add_systems(PreStartup, load_playground_textures);
        crate::projected_decal::install_decal_receiver_bindings::<
            PlaygroundTilesMaterial,
        >(app);
    }
}

#[derive(Resource, Default, Clone)]
pub struct PlaygroundMaterialLibrary {
    pub decal_buffers: crate::projected_decal::PackedDecalBuffers,
    pub tiles_grid: Option<Handle<Image>>,
    pub tiles_normal: Option<Handle<Image>>,
    pub tiles_occlusion: Option<Handle<Image>>,
}

fn repeating_sampler_descriptor() -> ImageSamplerDescriptor {
    ImageSamplerDescriptor {
        address_mode_u: bevy::image::ImageAddressMode::Repeat,
        address_mode_v: bevy::image::ImageAddressMode::Repeat,
        mag_filter: bevy::image::ImageFilterMode::Linear,
        min_filter: bevy::image::ImageFilterMode::Linear,
        mipmap_filter: bevy::image::ImageFilterMode::Linear,
        ..Default::default()
    }
}

fn generate_mip_chain(img: &mut Image) {
    let width = img.width();
    let height = img.height();
    if width == 1 || height == 1 {
        return;
    }
    let format_size = img.texture_descriptor.format.pixel_size().unwrap_or(4);
    let mut levels: Vec<Vec<u8>> = Vec::new();
    let mut cur = img.data.clone().expect("image data");
    let (mut w, mut h) = (width as usize, height as usize);
    loop {
        let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
        let mut next = vec![0u8; nw * nh * format_size];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..format_size {
                    let mut sum = 0u32;
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let sx = (x * 2 + dx).min(w - 1);
                            let sy = (y * 2 + dy).min(h - 1);
                            let idx = (sy * w + sx) * format_size + c;
                            sum += cur[idx] as u32;
                        }
                    }
                    next[(y * nw + x) * format_size + c] = (sum / 4) as u8;
                }
            }
        }
        cur = next.clone();
        levels.push(next);
        w = nw;
        h = nh;
        if w == 1 && h == 1 {
            break;
        }
    }

    let mut total = img.data.as_ref().map(|d| d.len()).unwrap_or(0);
    for level in &levels {
        total += level.len();
    }
    let mut all = Vec::with_capacity(total);
    all.extend_from_slice(img.data.as_deref().unwrap_or(&[]));
    for level in &levels {
        all.extend_from_slice(level);
    }
    img.data = Some(all);
    img.texture_descriptor.mip_level_count = levels.len() as u32 + 1;
}

fn copy_green_to_red(data: &mut [u8]) {
    for pixel in data.chunks_exact_mut(4) {
        pixel[0] = pixel[1];
    }
}

fn decode_image(bytes: &[u8], extension: &str, green_to_red: bool) -> Option<Image> {
    let mut image = Image::from_buffer(
        bytes,
        ImageType::Extension(extension),
        CompressedImageFormats::NONE,
        false,
        ImageSampler::Descriptor(repeating_sampler_descriptor()),
        RenderAssetUsages::default(),
    )
    .ok()?;
    if image.texture_descriptor.format == TextureFormat::Rgba8UnormSrgb {
        image.texture_descriptor.format = TextureFormat::Rgba8Unorm;
    }
    if green_to_red {
        let data = image.data.as_mut()?;
        copy_green_to_red(data);
    }
    generate_mip_chain(&mut image);
    Some(image)
}

fn add_image(
    server: &AssetServer,
    bytes: &[u8],
    extension: &str,
    green_to_red: bool,
) -> Option<Handle<Image>> {
    decode_image(bytes, extension, green_to_red).map(|image| server.add(image))
}

fn load_playground_textures(
    mut commands: Commands,
    server: Res<AssetServer>,
    buffers: Res<crate::projected_decal::PackedDecalBuffers>,
) {
    let grid =
        std::fs::read("assets/ALSExtras/Environment/Materials/T_Default_Material_Grid_M.tga");
    let normal = std::fs::read("assets/ALSExtras/Environment/Materials/T_Als_Tiles_N.tga");
    let masks = std::fs::read("assets/ALSExtras/Environment/Materials/T_Als_Tiles_M.tga");

    let library = PlaygroundMaterialLibrary {
        decal_buffers: buffers.clone(),
        tiles_grid: grid
            .ok()
            .and_then(|bytes| add_image(&server, &bytes, "tga", false)),
        tiles_normal: normal
            .ok()
            .and_then(|bytes| add_image(&server, &bytes, "tga", false)),
        tiles_occlusion: masks
            .ok()
            .and_then(|bytes| add_image(&server, &bytes, "tga", true)),
    };
    info!(
        "[PGMAT] textures loaded grid={} normal={} occlusion={}",
        library.tiles_grid.is_some(),
        library.tiles_normal.is_some(),
        library.tiles_occlusion.is_some()
    );
    commands.insert_resource(library);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_grid_tga_preserves_every_rgba_pixel() {
        for (bytes, size) in [
            (
                include_bytes!(
                    "../../../assets/ALSExtras/Environment/Materials/T_Default_Material_Grid_M.tga"
                )
                .as_slice(),
                512usize,
            ),
            (
                include_bytes!("../../../assets/ALSExtras/Environment/Materials/T_Als_Tiles_N.tga")
                    .as_slice(),
                2048usize,
            ),
            (
                include_bytes!("../../../assets/ALSExtras/Environment/Materials/T_Als_Tiles_M.tga")
                    .as_slice(),
                2048usize,
            ),
        ] {
            assert_eq!(&bytes[16..18], [32, 8]);
            let image = decode_image(bytes, "tga", false).unwrap();
            assert_eq!(image.size(), UVec2::splat(size as u32));
            let pixels = image.data.unwrap();
            for (index, bgra) in bytes[18..18 + size * size * 4].chunks_exact(4).enumerate() {
                let target = ((size - 1 - index / size) * size + index % size) * 4;
                assert_eq!(
                    &pixels[target..target + 4],
                    [bgra[2], bgra[1], bgra[0], bgra[3]]
                );
            }
        }
    }

    #[test]
    fn green_mask_becomes_occlusion_red_channel() {
        let mut pixels = vec![3, 17, 29, 255, 41, 53, 67, 255];
        copy_green_to_red(&mut pixels);
        assert_eq!(pixels, vec![17, 17, 29, 255, 53, 53, 67, 255]);
    }

    #[test]
    fn repeating_sampler_is_trilinear() {
        let sampler = repeating_sampler_descriptor();
        assert_eq!(sampler.mag_filter, bevy::image::ImageFilterMode::Linear);
        assert_eq!(sampler.min_filter, bevy::image::ImageFilterMode::Linear);
        assert_eq!(sampler.mipmap_filter, bevy::image::ImageFilterMode::Linear);
    }
}
