//! Minimal reader for [PMTiles v3](https://github.com/protomaps/PMTiles/blob/main/spec/v3/spec.md)
//! archives: single-file tile pyramids read with byte-range requests.
//!
//! Storage-agnostic: a [`RangeReader`] provides the bytes (HTTP range requests, local files,
//! browser blobs, ...).

use std::{cell::RefCell, collections::HashMap, io::Read, rc::Rc};

use thiserror::Error;

pub const HEADER_LEN: u64 = 127;

/// Directories nest at most this deep (root plus leaves).
const MAX_DIRECTORY_DEPTH: usize = 4;

#[derive(Error, Debug)]
pub enum PmTilesError {
    #[error("not a PMTiles v3 archive")]
    InvalidHeader,
    #[error("invalid PMTiles directory")]
    InvalidDirectory,
    #[error("unsupported PMTiles compression {0:?}")]
    UnsupportedCompression(Compression),
    #[error("decompressing PMTiles data failed")]
    Decompression(#[from] std::io::Error),
    #[error("reading the PMTiles archive failed: {0}")]
    Read(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compression {
    Unknown,
    None,
    Gzip,
    Brotli,
    Zstd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileType {
    Unknown,
    Mvt,
    Png,
    Jpeg,
    Webp,
    Avif,
}

#[derive(Clone, Debug)]
pub struct Header {
    pub root_offset: u64,
    pub root_length: u64,
    /// JSON metadata, compressed with `internal_compression`.
    pub metadata_offset: u64,
    pub metadata_length: u64,
    pub leaf_offset: u64,
    pub data_offset: u64,
    pub internal_compression: Compression,
    pub tile_compression: Compression,
    pub tile_type: TileType,
    pub min_zoom: u8,
    pub max_zoom: u8,
    /// `[min_lon, min_lat, max_lon, max_lat]` in degrees.
    pub bounds: [f64; 4],
    pub center_zoom: u8,
    /// `[lon, lat]` in degrees.
    pub center: [f64; 2],
}

impl Header {
    pub fn parse(bytes: &[u8]) -> Result<Self, PmTilesError> {
        if bytes.len() < HEADER_LEN as usize || &bytes[..7] != b"PMTiles" || bytes[7] != 3 {
            return Err(PmTilesError::InvalidHeader);
        }
        let u64_at = |i: usize| u64::from_le_bytes(bytes[i..i + 8].try_into().unwrap());
        let degrees_at =
            |i: usize| i32::from_le_bytes(bytes[i..i + 4].try_into().unwrap()) as f64 / 1e7;
        let compression = |b: u8| match b {
            1 => Compression::None,
            2 => Compression::Gzip,
            3 => Compression::Brotli,
            4 => Compression::Zstd,
            _ => Compression::Unknown,
        };
        Ok(Header {
            root_offset: u64_at(8),
            root_length: u64_at(16),
            metadata_offset: u64_at(24),
            metadata_length: u64_at(32),
            leaf_offset: u64_at(40),
            data_offset: u64_at(56),
            internal_compression: compression(bytes[97]),
            tile_compression: compression(bytes[98]),
            tile_type: match bytes[99] {
                1 => TileType::Mvt,
                2 => TileType::Png,
                3 => TileType::Jpeg,
                4 => TileType::Webp,
                5 => TileType::Avif,
                _ => TileType::Unknown,
            },
            min_zoom: bytes[100],
            max_zoom: bytes[101],
            bounds: [
                degrees_at(102),
                degrees_at(106),
                degrees_at(110),
                degrees_at(114),
            ],
            center_zoom: bytes[118],
            center: [degrees_at(119), degrees_at(123)],
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    tile_id: u64,
    offset: u64,
    length: u64,
    /// 0 marks a leaf directory instead of tile data.
    run_length: u64,
}

fn parse_directory(bytes: &[u8]) -> Result<Vec<Entry>, PmTilesError> {
    let mut bytes = bytes.iter().copied();
    let mut varint = || {
        let mut value = 0u64;
        for shift in (0..64).step_by(7) {
            let byte = bytes.next().ok_or(PmTilesError::InvalidDirectory)?;
            value |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Ok(value);
            }
        }
        Err(PmTilesError::InvalidDirectory)
    };

    let n = varint()? as usize;
    let mut entries = vec![
        Entry {
            tile_id: 0,
            offset: 0,
            length: 0,
            run_length: 0
        };
        n
    ];
    let mut tile_id = 0;
    for entry in &mut entries {
        tile_id += varint()?;
        entry.tile_id = tile_id;
    }
    for entry in &mut entries {
        entry.run_length = varint()?;
    }
    for entry in &mut entries {
        entry.length = varint()?;
    }
    for i in 0..n {
        entries[i].offset = match varint()? {
            // 0: directly after the previous entry's data.
            0 if i > 0 => entries[i - 1].offset + entries[i - 1].length,
            0 => return Err(PmTilesError::InvalidDirectory),
            offset => offset - 1,
        };
    }
    Ok(entries)
}

/// The entry for `tile_id`: an exact match, a run of tiles containing it, or the leaf directory
/// that may contain it.
fn find(entries: &[Entry], tile_id: u64) -> Option<&Entry> {
    match entries.binary_search_by_key(&tile_id, |entry| entry.tile_id) {
        Ok(i) => Some(&entries[i]),
        Err(0) => None,
        Err(i) => {
            let previous = &entries[i - 1];
            (previous.run_length == 0 || tile_id - previous.tile_id < previous.run_length)
                .then_some(previous)
        }
    }
}

/// Position of a tile on the Hilbert curve across all zoom levels.
pub fn tile_id(z: u8, x: u32, y: u32) -> u64 {
    let n = 1u64 << z;
    let (mut x, mut y) = (u64::from(x), u64::from(y));
    let mut d = 0;
    let mut s = n / 2;
    while s > 0 {
        let rx = u64::from(x & s > 0);
        let ry = u64::from(y & s > 0);
        d += s * s * ((3 * rx) ^ ry);
        if ry == 0 {
            if rx == 1 {
                x = n - 1 - x;
                y = n - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s /= 2;
    }
    // Tiles in all lower zoom levels: (4^z - 1) / 3.
    (n * n - 1) / 3 + d
}

pub fn decompress(compression: Compression, data: Vec<u8>) -> Result<Vec<u8>, PmTilesError> {
    match compression {
        Compression::None | Compression::Unknown => Ok(data),
        Compression::Gzip => {
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(data.as_slice()).read_to_end(&mut out)?;
            Ok(out)
        }
        other => Err(PmTilesError::UnsupportedCompression(other)),
    }
}

/// Reads byte ranges of an archive.
#[allow(async_fn_in_trait)]
pub trait RangeReader {
    async fn read(&self, offset: u64, length: u64) -> Result<Vec<u8>, PmTilesError>;
}

/// An open archive.
pub struct PmTiles<R> {
    reader: R,
    pub header: Header,
    root: Vec<Entry>,
    // ponytail: unbounded leaf directory cache; fine for regional archives, add LRU for planet-scale.
    leaves: RefCell<HashMap<u64, Rc<Vec<Entry>>>>,
}

impl<R: RangeReader> PmTiles<R> {
    /// The archive's JSON metadata (e.g. `vector_layers`).
    pub async fn metadata(&self) -> Result<String, PmTilesError> {
        let bytes = self
            .reader
            .read(self.header.metadata_offset, self.header.metadata_length)
            .await?;
        String::from_utf8(decompress(self.header.internal_compression, bytes)?)
            .map_err(|e| PmTilesError::Read(e.to_string()))
    }

    pub async fn open(reader: R) -> Result<Self, PmTilesError> {
        let header = Header::parse(&reader.read(0, HEADER_LEN).await?)?;
        let root =
            Self::read_directory(&reader, &header, header.root_offset, header.root_length).await?;
        Ok(PmTiles {
            reader,
            header,
            root,
            leaves: Default::default(),
        })
    }

    async fn read_directory(
        reader: &R,
        header: &Header,
        offset: u64,
        length: u64,
    ) -> Result<Vec<Entry>, PmTilesError> {
        let bytes = reader.read(offset, length).await?;
        parse_directory(&decompress(header.internal_compression, bytes)?)
    }

    /// The tile at z/x/y, decompressed, or `None` if the archive does not contain it.
    pub async fn tile(&self, z: u8, x: u32, y: u32) -> Result<Option<Vec<u8>>, PmTilesError> {
        let tile_id = tile_id(z, x, y);
        let mut entry = find(&self.root, tile_id).cloned();

        for _ in 0..MAX_DIRECTORY_DEPTH {
            let Some(leaf) = entry.as_ref().filter(|entry| entry.run_length == 0) else {
                break;
            };
            let offset = self.header.leaf_offset + leaf.offset;
            let cached = self.leaves.borrow().get(&offset).cloned();
            let directory = match cached {
                Some(directory) => directory,
                None => {
                    let directory = Rc::new(
                        Self::read_directory(&self.reader, &self.header, offset, leaf.length)
                            .await?,
                    );
                    self.leaves.borrow_mut().insert(offset, directory.clone());
                    directory
                }
            };
            entry = find(&directory, tile_id).cloned();
        }

        let Some(entry) = entry.filter(|entry| entry.run_length > 0) else {
            return Ok(None);
        };
        let data = self
            .reader
            .read(self.header.data_offset + entry.offset, entry.length)
            .await?;
        decompress(self.header.tile_compression, data).map(Some)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        future::Future,
        io::Cursor,
        pin::pin,
        task::{Context, Poll, Waker},
    };

    use super::*;

    /// Polls a future that completes without waiting, which in-memory readers do.
    fn ready<F: Future>(future: F) -> F::Output {
        match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(output) => output,
            Poll::Pending => panic!("future is not ready"),
        }
    }

    impl RangeReader for Vec<u8> {
        async fn read(&self, offset: u64, length: u64) -> Result<Vec<u8>, PmTilesError> {
            self.get(offset as usize..(offset + length) as usize)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| PmTilesError::Read("out of range".to_string()))
        }
    }

    fn tile_data(z: u8, x: u32, y: u32) -> Vec<u8> {
        format!("{z}/{x}/{y}").into_bytes()
    }

    /// Writes all tiles up to `max_zoom` with the reference implementation.
    fn archive(max_zoom: u8) -> Vec<u8> {
        let mut buffer = Cursor::new(Vec::new());
        let mut writer = pmtiles::PmTilesWriter::new(pmtiles::TileType::Mvt)
            .max_zoom(max_zoom)
            .metadata(r#"{"vector_layers":[{"id":"test"}]}"#)
            .bounds(-10.0, -20.0, 30.0, 40.5)
            .create(&mut buffer)
            .unwrap();
        // The writer expects tiles in tile id order.
        let mut tiles: Vec<_> = (0..=max_zoom)
            .flat_map(|z| (0..1u32 << z).flat_map(move |x| (0..1u32 << z).map(move |y| (z, x, y))))
            .collect();
        tiles.sort_by_key(|&(z, x, y)| tile_id(z, x, y));
        for (z, x, y) in tiles {
            let coord = pmtiles::TileCoord::new(z, x, y).unwrap();
            writer.add_tile(coord, &tile_data(z, x, y)).unwrap();
        }
        writer.finalize().unwrap();
        buffer.into_inner()
    }

    #[test]
    fn tile_id_matches_reference() {
        for z in 0..=8u8 {
            for x in (0..1u32 << z).step_by(3) {
                for y in (0..1u32 << z).step_by(5) {
                    let reference =
                        pmtiles::TileId::from(pmtiles::TileCoord::new(z, x, y).unwrap());
                    assert_eq!(tile_id(z, x, y), reference.value(), "{z}/{x}/{y}");
                }
            }
        }
    }

    #[test]
    fn reads_tiles_written_by_reference() {
        let archive = ready(PmTiles::open(archive(3))).unwrap();
        assert_eq!(archive.header.tile_type, TileType::Mvt);
        assert_eq!(archive.header.max_zoom, 3);
        assert_eq!(archive.header.bounds, [-10.0, -20.0, 30.0, 40.5]);
        assert_eq!(
            ready(archive.metadata()).unwrap(),
            r#"{"vector_layers":[{"id":"test"}]}"#
        );
        for (z, x, y) in [(0, 0, 0), (1, 1, 0), (3, 5, 2), (3, 7, 7)] {
            assert_eq!(
                ready(archive.tile(z, x, y)).unwrap(),
                Some(tile_data(z, x, y))
            );
        }
        assert_eq!(ready(archive.tile(4, 0, 0)).unwrap(), None);
    }

    #[test]
    fn reads_through_leaf_directories() {
        // Enough tiles that the root directory overflows into leaves.
        let bytes = archive(7);
        let archive = ready(PmTiles::open(bytes)).unwrap();
        assert!(archive.root.iter().any(|entry| entry.run_length == 0));
        for (z, x, y) in [(7, 0, 0), (7, 127, 127), (7, 64, 33), (6, 12, 40)] {
            assert_eq!(
                ready(archive.tile(z, x, y)).unwrap(),
                Some(tile_data(z, x, y))
            );
        }
    }
}
