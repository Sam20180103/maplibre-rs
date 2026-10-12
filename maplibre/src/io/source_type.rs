use crate::{
    coords::WorldTileCoords,
    style::source::{TileAddressingScheme, VectorSource},
};

/// Represents a source from which the vector tile are fetched.
#[derive(Clone)]
pub struct TessellateSource {
    pub url: String,
    pub filetype: String,
}

impl TessellateSource {
    pub fn new(url: &str, filetype: &str) -> Self {
        Self {
            url: url.to_string(),
            filetype: filetype.to_string(),
        }
    }

    pub fn format(&self, coords: &WorldTileCoords) -> String {
        let tile_coords = coords.into_tile(TileAddressingScheme::XYZ).unwrap();
        format!(
            "{url}/{z}/{x}/{y}.{filetype}",
            url = self.url,
            z = tile_coords.z,
            x = tile_coords.x,
            y = tile_coords.y,
            filetype = self.filetype,
        )
    }
}

impl Default for TessellateSource {
    fn default() -> Self {
        Self::new("https://maps.tuerantuer.org/europe_germany", "pbf")
        // Self::new("https://demotiles.maplibre.org/tiles", "pbf")
    }
}

/// Represents a source from which the raster tile are fetched.
#[derive(Clone)]
pub struct RasterSource {
    pub url: String,
    pub filetype: String,
    pub key: String,
}

impl RasterSource {
    pub fn new(url: &str, filetype: &str, key: &str) -> Self {
        Self {
            url: url.to_string(),
            filetype: filetype.to_string(),
            key: key.to_string(),
        }
    }

    pub fn format(&self, coords: &WorldTileCoords) -> String {
        let tile_coords = coords.into_tile(TileAddressingScheme::XYZ).unwrap();
        format!(
            "{url}/{z}/{x}/{y}.{filetype}?key={key}",
            url = self.url,
            z = tile_coords.z,
            x = tile_coords.x,
            y = tile_coords.y,
            filetype = self.filetype,
            key = self.key,
        )
    }
}

impl Default for RasterSource {
    fn default() -> Self {
        Self::new(
            "https://api.maptiler.com/tiles/satellite-v2",
            "jpg",
            "qnePkfbGpMsLCi3KFBs3",
        )
    }
}

/// Represents the tiles' different types of source.
#[derive(Clone)]
pub enum SourceType {
    Raster(RasterSource),
    Tessellate(TessellateSource),
    /// Tile URL templates from a style source's `tiles`, e.g. `https://example.com/{z}/{x}/{y}.pbf`.
    Template {
        tiles: Vec<String>,
        scheme: TileAddressingScheme,
    },
}

impl SourceType {
    /// A template source from a style source, or `None` if it lists no tile URLs.
    pub fn from_style_source(source: &VectorSource) -> Option<Self> {
        let tiles = source.tiles.clone().filter(|tiles| !tiles.is_empty())?;
        Some(SourceType::Template {
            tiles,
            scheme: source.scheme.clone().unwrap_or_default(),
        })
    }

    pub fn format(&self, coords: &WorldTileCoords) -> String {
        match self {
            SourceType::Raster(raster_source) => raster_source.format(coords),
            SourceType::Tessellate(tessellate_source) => tessellate_source.format(coords),
            SourceType::Template { tiles, scheme } => {
                let tile = coords.into_tile(scheme.clone()).unwrap();
                // Spread requests over the listed hosts, like MapLibre GL JS.
                let template = &tiles[(tile.x + tile.y) as usize % tiles.len()];
                template
                    .replace("{z}", &tile.z.to_string())
                    .replace("{x}", &tile.x.to_string())
                    .replace("{y}", &tile.y.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::ZoomLevel;

    #[test]
    fn template_fills_placeholders_and_spreads_hosts() {
        let source = SourceType::Template {
            tiles: vec![
                "https://a.example/{z}/{x}/{y}.pbf".to_string(),
                "https://b.example/vt?x={x}&y={y}&z={z}".to_string(),
            ],
            scheme: TileAddressingScheme::XYZ,
        };
        let coords: WorldTileCoords = (2, 1, ZoomLevel::new(3)).into();
        assert_eq!(source.format(&coords), "https://b.example/vt?x=2&y=1&z=3");
        let coords: WorldTileCoords = (2, 2, ZoomLevel::new(3)).into();
        assert_eq!(source.format(&coords), "https://a.example/3/2/2.pbf");
    }

    #[test]
    fn template_flips_y_for_tms() {
        let source = SourceType::Template {
            tiles: vec!["{z}/{x}/{y}".to_string()],
            scheme: TileAddressingScheme::TMS,
        };
        let coords: WorldTileCoords = (0, 0, ZoomLevel::new(1)).into();
        assert_eq!(source.format(&coords), "1/0/1");
    }
}
