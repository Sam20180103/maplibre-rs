//! Requests tiles which are currently in view

use std::{borrow::Cow, marker::PhantomData, rc::Rc};

use crate::{
    context::MapContext,
    environment::{Environment, OffscreenKernel},
    io::{
        apc::{AsyncProcedureCall, AsyncProcedureFuture, Context, Input, ProcedureError},
        source_type::{RasterSource, SourceType},
    },
    kernel::Kernel,
    raster::{
        is_raster_layer,
        process_raster::{process_raster_tile, ProcessRasterContext, RasterTileRequest},
        transferables::{LayerRasterMissing, RasterTransferables},
        RasterLayersDataComponent,
    },
    render::{tile_view_pattern::DEFAULT_TILE_SIZE, view_state::ViewStatePadding},
    style::source::Source,
    tcs::system::{System, SystemResult},
};

pub struct RequestSystem<E: Environment, T: RasterTransferables> {
    kernel: Rc<Kernel<E>>,
    phantom_t: PhantomData<T>,
}

impl<E: Environment, T: RasterTransferables> RequestSystem<E, T> {
    pub fn new(kernel: &Rc<Kernel<E>>) -> Self {
        Self {
            kernel: kernel.clone(),
            phantom_t: Default::default(),
        }
    }
}

impl<E: Environment, T: RasterTransferables> System for RequestSystem<E, T> {
    fn name(&self) -> Cow<'static, str> {
        "raster_request".into()
    }

    fn run(
        &mut self,
        MapContext {
            style,
            view_state,
            world,
            ..
        }: &mut MapContext,
    ) -> SystemResult {
        let view_region = view_state.create_view_region(
            view_state.zoom().zoom_level(DEFAULT_TILE_SIZE),
            ViewStatePadding::Loose,
        );

        if !style.layers.iter().any(is_raster_layer) {
            return Ok(());
        }

        if view_state.did_camera_change() || view_state.did_zoom_change() {
            if let Some(view_region) = &view_region {
                // TODO: We also need to request tiles from layers above if we are over the maximum zoom level

                for coords in view_region.iter() {
                    if coords.build_quad_key().is_none() {
                        continue;
                    }

                    // TODO: Make tessellation depend on style? So maybe we need to request even if it exists
                    if world
                        .tiles
                        .query::<&RasterLayersDataComponent>(coords)
                        .is_some()
                    {
                        continue;
                    }

                    world
                        .tiles
                        .spawn_mut(coords)
                        .expect("unable to spawn a raster tile")
                        .insert(RasterLayersDataComponent::default());

                    tracing::event!(tracing::Level::ERROR, %coords, "tile request started: {coords}");
                    log::info!("tile request started: {coords}");

                    self.kernel
                        .apc()
                        .call(
                            Input::TileRequest {
                                coords,
                                style: style.clone(), // TODO: Avoid cloning whole style
                            },
                            fetch_raster_apc::<
                                E::OffscreenKernelEnvironment,
                                T,
                                <E::AsyncProcedureCall as AsyncProcedureCall<
                                    E::OffscreenKernelEnvironment,
                                >>::Context,
                            >,
                        )
                        .expect("unable to call APC"); // TODO: Remove unwrap
                }
            }
        }

        Ok(())
    }
}
pub fn fetch_raster_apc<K: OffscreenKernel, T: RasterTransferables, C: Context + Clone + Send>(
    input: Input,
    context: C,
    kernel: K,
) -> AsyncProcedureFuture {
    Box::pin(async move {
        let Input::TileRequest { coords, style } = input else {
            return Err(ProcedureError::IncompatibleInput);
        };

        let client = kernel.source_client();

        for layer in style.layers.iter().filter(|layer| is_raster_layer(layer)) {
            let source = match &layer.source {
                None => Some(SourceType::Raster(RasterSource::default())),
                Some(id) => match style.sources.get(id) {
                    Some(Source::Raster(source)) => SourceType::from_style_source(source),
                    _ => None,
                },
            };
            let Some(source) = source else {
                log::warn!("raster layer {} has no raster source with tiles", layer.id);
                continue;
            };

            let result = match client.fetch(&coords, &source).await {
                Ok(data) => process_raster_tile(
                    &data,
                    RasterTileRequest {
                        coords,
                        layer: layer.id.clone(),
                    },
                    &mut ProcessRasterContext::<T, C>::new(context.clone()),
                )
                .map_err(|e| format!("{e:?}")),
                Err(e) => Err(format!("{e:?}")),
            };
            if let Err(e) = result {
                log::error!("raster tile {coords} of layer {} failed: {e}", layer.id);
                context
                    .send_back(<T as RasterTransferables>::LayerRasterMissing::build_from(
                        coords,
                    ))
                    .map_err(ProcedureError::Send)?;
            }
        }

        Ok(())
    })
}
