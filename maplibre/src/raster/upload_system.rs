//! Uploads data to the GPU which is needed for rendering.
use crate::{
    context::MapContext,
    coords::ViewRegion,
    raster::{
        is_raster_layer, resource::RasterResources, AvailableRasterLayerData, RasterLayerData,
        RasterLayersDataComponent,
    },
    render::{
        eventually::{Eventually, Eventually::Initialized},
        tile_view_pattern::DEFAULT_TILE_SIZE,
        view_state::ViewStatePadding,
        Renderer,
    },
    style::Style,
    tcs::{
        system::{SystemError, SystemResult},
        tiles::Tiles,
    },
};

pub fn upload_system(
    MapContext {
        world,
        style,
        view_state,
        renderer: Renderer { device, queue, .. },
        ..
    }: &mut MapContext,
) -> SystemResult {
    let Some(Initialized(raster_resources)) = world
        .resources
        .query_mut::<&mut Eventually<RasterResources>>()
    else {
        return Err(SystemError::Dependencies);
    };
    raster_resources.active = style.layers.iter().any(is_raster_layer);
    let view_region = view_state.create_view_region(
        view_state.zoom().zoom_level(DEFAULT_TILE_SIZE),
        ViewStatePadding::Loose,
    );

    if let Some(view_region) = &view_region {
        upload_raster_layer(
            raster_resources,
            device,
            queue,
            &world.tiles,
            style,
            view_region,
        );
    }

    Ok(())
}

#[tracing::instrument(skip_all)]
fn upload_raster_layer(
    raster_resources: &mut RasterResources,
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    tiles: &Tiles,
    style: &Style,
    view_region: &ViewRegion,
) {
    for coords in view_region.iter() {
        if raster_resources.get_bound_texture(&coords).is_some() {
            continue;
        }

        let Some(raster_layers) = tiles.query::<&RasterLayersDataComponent>(coords) else {
            continue;
        };

        for style_layer in style.layers.iter().filter(|layer| is_raster_layer(layer)) {
            let Some(AvailableRasterLayerData { coords, image, .. }) = raster_layers
                .layers
                .iter()
                .flat_map(|data| match data {
                    RasterLayerData::Available(data) => Some(data),
                    RasterLayerData::Missing(_) => None,
                })
                .find(|data| data.source_layer == style_layer.id)
            else {
                continue;
            };

            let (width, height) = image.dimensions();

            let texture = raster_resources.create_texture(
                None,
                device,
                // Not sRGB: tiles are drawn to a non-sRGB view, so the bytes must pass through
                // unconverted, like the CSS colors of vector layers.
                wgpu::TextureFormat::Rgba8Unorm,
                width,
                height,
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            );

            queue.write_texture(
                wgpu::ImageCopyTexture {
                    aspect: wgpu::TextureAspect::All,
                    texture: &texture.texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                },
                image,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * width),
                    rows_per_image: Some(height),
                },
                texture.size,
            );

            raster_resources.bind_texture(device, coords, texture);
        }
    }
}
