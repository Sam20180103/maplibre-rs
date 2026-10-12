use std::marker::PhantomData;

use image::RgbaImage;
use thiserror::Error;

use crate::{
    coords::WorldTileCoords,
    io::apc::Context,
    raster::transferables::{LayerRaster, RasterTransferables},
};

#[derive(Error, Debug)]
pub enum ProcessRasterError {
    /// Error during processing of the pipeline
    #[error("processing data in pipeline failed")]
    Processing(Box<dyn std::error::Error>),
}

pub struct RasterTileRequest {
    pub coords: WorldTileCoords,
    /// Id of the style layer the tile is for.
    pub layer: String,
    /// Zoom level of the image when it belongs to an ancestor of `coords` (overzoom); the tile
    /// is then cropped out of it.
    pub data_zoom: Option<u8>,
}

pub fn process_raster_tile<T: RasterTransferables, C: Context>(
    data: &[u8],
    tile_request: RasterTileRequest,
    context: &mut ProcessRasterContext<T, C>,
) -> Result<(), ProcessRasterError> {
    let coords = &tile_request.coords;
    let img =
        image::load_from_memory(data).map_err(|e| ProcessRasterError::Processing(Box::new(e)))?;
    let mut rgba = img.to_rgba8();

    if let Some(data_zoom) = tile_request.data_zoom {
        let scale = 1 << (u8::from(coords.z) - data_zoom);
        let (width, height) = (
            (rgba.width() / scale).max(1),
            (rgba.height() / scale).max(1),
        );
        let (cx, cy) = (coords.x as u32 % scale, coords.y as u32 % scale);
        rgba = image::imageops::crop_imm(&rgba, cx * width, cy * height, width, height).to_image();
    }

    context.layer_raster_finished(coords, tile_request.layer, rgba)?;

    Ok(())
}
pub struct ProcessRasterContext<T: RasterTransferables, C: Context> {
    context: C,
    phantom_t: PhantomData<T>,
}

impl<T: RasterTransferables, C: Context> ProcessRasterContext<T, C> {
    pub fn new(context: C) -> Self {
        Self {
            context,
            phantom_t: Default::default(),
        }
    }
}

impl<T: RasterTransferables, C: Context> ProcessRasterContext<T, C> {
    fn layer_raster_finished(
        &mut self,
        coords: &WorldTileCoords,
        layer_name: String,
        image_data: RgbaImage,
    ) -> Result<(), ProcessRasterError> {
        self.context
            .send_back(T::LayerRaster::build_from(*coords, layer_name, image_data))
            .map_err(|e| ProcessRasterError::Processing(Box::new(e)))
    }
}

#[cfg(test)]
mod tests {
    use super::process_raster_tile;
    use crate::{
        coords::ZoomLevel,
        io::apc::tests::DummyContext,
        raster::{
            process_raster::{ProcessRasterContext, RasterTileRequest},
            DefaultRasterTransferables,
        },
    };

    #[test] // TODO: Add proper tile byte array
    #[ignore]
    fn test() {
        let _output = process_raster_tile(
            &[0],
            RasterTileRequest {
                coords: (0, 0, ZoomLevel::default()).into(),
                layer: "raster".to_string(),
                data_zoom: None,
            },
            &mut ProcessRasterContext::<DefaultRasterTransferables, _>::new(DummyContext),
        );
    }
}

#[cfg(test)]
mod crop_tests {
    use image::{codecs::png::PngEncoder, ImageEncoder, Rgba, RgbaImage};

    use super::*;
    use crate::{coords::ZoomLevel, io::apc::Context, io::apc::IntoMessage, io::apc::SendError};

    #[derive(Clone)]
    struct Capture(std::rc::Rc<std::cell::RefCell<Vec<crate::io::apc::Message>>>);
    impl Context for Capture {
        fn send_back<T: IntoMessage>(&self, message: T) -> Result<(), SendError> {
            self.0.borrow_mut().push(message.into());
            Ok(())
        }
    }

    #[test]
    fn overzoomed_tile_is_cropped_from_ancestor() {
        // 4x4 image whose quadrants have distinct colors; the tile two levels down at (1, 2)
        // within the ancestor is pixel (1, 2).
        let image = RgbaImage::from_fn(4, 4, |x, y| Rgba([x as u8, y as u8, 0, 255]));
        let mut png = Vec::new();
        PngEncoder::new(&mut png)
            .write_image(image.as_raw(), 4, 4, image::ExtendedColorType::Rgba8)
            .unwrap();
        let messages = Capture(Default::default());
        let mut context = ProcessRasterContext::<crate::raster::DefaultRasterTransferables, _>::new(
            messages.clone(),
        );
        process_raster_tile(
            &png,
            RasterTileRequest {
                coords: (4 * 7 + 1, 4 * 3 + 2, ZoomLevel::new(5)).into(),
                layer: "l".to_string(),
                data_zoom: Some(3),
            },
            &mut context,
        )
        .unwrap();
        let message = messages.0.borrow_mut().pop().unwrap();
        let layer = message
            .into_transferable::<<crate::raster::DefaultRasterTransferables as crate::raster::RasterTransferables>::LayerRaster>();
        let data = crate::raster::LayerRaster::to_layer(*layer);
        assert_eq!(data.image.dimensions(), (1, 1));
        assert_eq!(data.image.get_pixel(0, 0), &Rgba([1, 2, 0, 255]));
    }
}
