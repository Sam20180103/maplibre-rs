//! `pmtiles://<file>/{z}/{x}/{y}` tile URLs: tiles from PMTiles archives stored in the origin
//! private file system (OPFS), e.g. downloaded for offline use.

use std::{cell::RefCell, collections::HashMap, rc::Rc};

use js_sys::Uint8Array;
use maplibre::io::pmtiles::{PmTiles, PmTilesError, RangeReader};
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{Blob, File, FileSystemDirectoryHandle, FileSystemFileHandle, WorkerGlobalScope};

use crate::error::WebError;

pub const SCHEME: &str = "pmtiles://";

struct BlobReader(Blob);

fn js_error(e: JsValue) -> PmTilesError {
    PmTilesError::Read(format!("{e:?}"))
}

impl RangeReader for BlobReader {
    async fn read(&self, offset: u64, length: u64) -> Result<Vec<u8>, PmTilesError> {
        let slice = self
            .0
            .slice_with_f64_and_f64(offset as f64, (offset + length) as f64)
            .map_err(js_error)?;
        let buffer = JsFuture::from(slice.array_buffer())
            .await
            .map_err(js_error)?;
        Ok(Uint8Array::new(&buffer).to_vec())
    }
}

thread_local! {
    /// Archives opened by this worker, by file name.
    static ARCHIVES: RefCell<HashMap<String, Rc<PmTiles<BlobReader>>>> = Default::default();
}

async fn open(name: &str) -> Result<PmTiles<BlobReader>, WebError> {
    let scope: WorkerGlobalScope = js_sys::global().unchecked_into();
    let root: FileSystemDirectoryHandle =
        JsFuture::from(scope.navigator().storage().get_directory())
            .await?
            .unchecked_into();
    let handle: FileSystemFileHandle = JsFuture::from(root.get_file_handle(name))
        .await?
        .unchecked_into();
    let file: File = JsFuture::from(handle.get_file()).await?.unchecked_into();
    PmTiles::open(BlobReader(file.into()))
        .await
        .map_err(|e| WebError::GenericError(e.to_string().into()))
}

/// Fetches `<file>/<z>/<x>/<y>`. A tile missing from the archive yields no bytes.
pub async fn fetch(path: &str) -> Result<Vec<u8>, WebError> {
    let invalid = || WebError::GenericError(format!("invalid PMTiles URL {SCHEME}{path}").into());
    let mut parts = path.rsplitn(4, '/');
    let (Some(y), Some(x), Some(z), Some(name)) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(invalid());
    };
    let (z, x, y) = match (z.parse(), x.parse(), y.parse()) {
        (Ok(z), Ok(x), Ok(y)) => (z, x, y),
        _ => return Err(invalid()),
    };

    let cached = ARCHIVES.with_borrow(|archives| archives.get(name).cloned());
    let archive = match cached {
        Some(archive) => archive,
        None => {
            let archive = Rc::new(open(name).await?);
            ARCHIVES.with_borrow_mut(|archives| archives.insert(name.to_string(), archive.clone()));
            archive
        }
    };

    match archive.tile(z, x, y).await {
        Ok(tile) => Ok(tile.unwrap_or_default()),
        Err(e) => {
            // The file may have been replaced since it was opened; reopen on the next request.
            ARCHIVES.with_borrow_mut(|archives| archives.remove(name));
            Err(WebError::GenericError(e.to_string().into()))
        }
    }
}
