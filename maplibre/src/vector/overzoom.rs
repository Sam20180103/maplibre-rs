//! Overzooming: beyond a source's max zoom, a tile is cut out of its ancestor at max zoom by
//! scaling the ancestor's geometry. Geometry outside the tile is kept; the tile mask clips it.

use geozero::mvt::{Message, Tile};

use crate::{coords::WorldTileCoords, vector::ProcessVectorError};

/// Rewrites MVT geometry commands of a feature from ancestor to descendant tile space:
/// `p' = p * scale - offset`. The cursor starts at (0, 0) for each feature, so only the first
/// position is absolute; all later deltas just scale.
fn overzoom_geometry(geometry: &mut [u32], scale: i64, offset: (i64, i64)) {
    let decode = |v: u32| i64::from(v >> 1) ^ -i64::from(v & 1);
    let encode = |n: i64| ((n << 1) ^ (n >> 63)) as u32;
    let mut first = true;
    let mut i = 0;
    while i < geometry.len() {
        let (command, count) = (geometry[i] & 7, (geometry[i] >> 3) as usize);
        i += 1;
        if command == 7 {
            continue; // ClosePath has no parameters.
        }
        for _ in 0..count {
            let Some(pair) = geometry.get_mut(i..i + 2) else {
                return;
            };
            let (mut dx, mut dy) = (decode(pair[0]) * scale, decode(pair[1]) * scale);
            if first {
                (dx, dy) = (dx - offset.0, dy - offset.1);
                first = false;
            }
            pair[0] = encode(dx);
            pair[1] = encode(dy);
            i += 2;
        }
    }
}

/// Cuts tile `coords` out of `data`, the encoded tile of its ancestor at zoom level `max_zoom`.
pub fn overzoom_mvt(
    data: &[u8],
    coords: WorldTileCoords,
    max_zoom: u8,
) -> Result<Vec<u8>, ProcessVectorError> {
    let mut tile =
        Tile::decode(data).map_err(|e| ProcessVectorError::Decoding(e.to_string().into()))?;
    let levels = u8::from(coords.z) - max_zoom;
    let scale = 1i64 << levels;
    // Position of the tile within its ancestor, in descendant tiles.
    let (cx, cy) = (
        i64::from(coords.x) & (scale - 1),
        i64::from(coords.y) & (scale - 1),
    );
    for layer in &mut tile.layers {
        let extent = i64::from(layer.extent.unwrap_or(4096));
        for feature in &mut layer.features {
            overzoom_geometry(&mut feature.geometry, scale, (cx * extent, cy * extent));
        }
    }
    Ok(tile.encode_to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::ZoomLevel;

    fn command(id: u32, count: u32) -> u32 {
        id | count << 3
    }
    fn zigzag(n: i32) -> u32 {
        ((n << 1) ^ (n >> 31)) as u32
    }

    #[test]
    fn line_in_bottom_right_child() {
        // Line from (2048, 2048) to (3072, 3072) in a 4096 tile; in the bottom-right child it runs
        // from (0, 0) to (2048, 2048).
        let mut geometry = vec![
            command(1, 1),
            zigzag(2048),
            zigzag(2048),
            command(2, 1),
            zigzag(1024),
            zigzag(1024),
        ];
        overzoom_geometry(&mut geometry, 2, (4096, 4096));
        assert_eq!(
            geometry,
            [
                command(1, 1),
                zigzag(0),
                zigzag(0),
                command(2, 1),
                zigzag(2048),
                zigzag(2048)
            ]
        );
    }

    #[test]
    fn polygon_close_path_and_second_ring() {
        // Two rings: deltas after the first position only scale; ClosePath is kept.
        let mut geometry = vec![
            command(1, 1),
            zigzag(100),
            zigzag(200),
            command(2, 2),
            zigzag(10),
            zigzag(0),
            zigzag(0),
            zigzag(10),
            command(7, 1),
            command(1, 1),
            zigzag(-5),
            zigzag(-5),
        ];
        overzoom_geometry(&mut geometry, 4, (0, 4096));
        assert_eq!(
            geometry,
            [
                command(1, 1),
                zigzag(400),
                zigzag(800 - 4096),
                command(2, 2),
                zigzag(40),
                zigzag(0),
                zigzag(0),
                zigzag(40),
                command(7, 1),
                command(1, 1),
                zigzag(-20),
                zigzag(-20)
            ]
        );
    }

    #[test]
    fn ancestor_offset() {
        let coords = WorldTileCoords {
            x: 13,
            y: 6,
            z: ZoomLevel::new(5),
        };
        assert_eq!(coords.ancestor(3), (3, 1, ZoomLevel::new(3)).into());
        // 13 % 4 = 1, 6 % 4 = 2 within the ancestor.
        let tile = Tile {
            layers: vec![geozero::mvt::tile::Layer {
                version: 2,
                name: "l".into(),
                features: vec![geozero::mvt::tile::Feature {
                    geometry: vec![command(1, 1), zigzag(1024), zigzag(2048)],
                    ..Default::default()
                }],
                extent: Some(4096),
                ..Default::default()
            }],
        };
        let out = Tile::decode(
            overzoom_mvt(&tile.encode_to_vec(), coords, 3)
                .unwrap()
                .as_slice(),
        )
        .unwrap();
        assert_eq!(
            out.layers[0].features[0].geometry,
            [command(1, 1), zigzag(1024 * 4 - 4096), zigzag(2048 * 4 - 2 * 4096)]
        );
    }
}
