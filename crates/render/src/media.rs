//! Where pictures come from.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use reelforge_renderspec::{Asset, AssetKind};
use reelforge_time::Tick;

use crate::lut::Lut;
use crate::{Image, Yuv};

/// Supplies decoded pictures to the planner.
///
/// Implementations: [`StillImages`] for image assets; a video decoder (FFmpeg-free, WebCodecs in
/// the browser, the platform's hardware decoder natively) implements this same trait.
pub trait MediaSource {
    /// The picture of `asset` at source time `at`. `Ok(None)` means "not available yet" (offline,
    /// still loading, or a kind this source does not decode).
    fn picture(&self, asset_id: &str, asset: &Asset, at: Tick) -> Result<Option<Arc<Image>>, String>;

    /// [`picture`](Self::picture), but it may be `shrink` (2, 4 or 8) times smaller each way: what a
    /// draft needs where the picture is drawn that much smaller. The default ignores `shrink`.
    fn picture_shrunk(
        &self,
        asset_id: &str,
        asset: &Asset,
        at: Tick,
        shrink: u32,
    ) -> Result<Option<Arc<Image>>, String> {
        let _ = shrink;
        self.picture(asset_id, asset, at)
    }

    /// The picture of a video `asset` at source time `at` as the decoder made it, for a compositor
    /// that converts Y'CbCr itself (the GPU's). `Ok(None)` means "use [`picture`](Self::picture)":
    /// the default, and what a source answers for stills or pictures in other forms.
    fn yuv(&self, asset_id: &str, asset: &Asset, at: Tick) -> Result<Option<Arc<Yuv>>, String> {
        let _ = (asset_id, asset, at);
        Ok(None)
    }

    /// The bytes of a `font` asset (TrueType or OpenType). `Ok(None)` means not available; the
    /// default implementation has no fonts.
    fn font(&self, asset_id: &str, asset: &Asset) -> Result<Option<Arc<[u8]>>, String> {
        let _ = (asset_id, asset);
        Ok(None)
    }

    /// A `lut` asset, read. `Ok(None)` means not available; the default implementation has none.
    fn lut(&self, asset_id: &str, asset: &Asset) -> Result<Option<Arc<Lut>>, String> {
        let _ = (asset_id, asset);
        Ok(None)
    }
}

/// A source with nothing in it: every picture is offline.
pub struct NoMedia;

impl MediaSource for NoMedia {
    fn picture(&self, _: &str, _: &Asset, _: Tick) -> Result<Option<Arc<Image>>, String> {
        Ok(None)
    }
}

/// Still images, decoded once and kept by content hash.
#[derive(Default)]
pub struct StillImages {
    cache: Mutex<HashMap<String, Arc<Image>>>,
}

impl StillImages {
    /// An empty cache.
    pub fn new() -> StillImages {
        StillImages::default()
    }

    /// Decodes PNG or JPEG bytes for the asset with this hash (the browser hands bytes over this
    /// way; there is no file system there).
    pub fn insert_bytes(&self, sha256: &str, bytes: &[u8]) -> Result<(), String> {
        let decoded = ::image::load_from_memory(bytes).map_err(|e| e.to_string())?.to_rgba8();
        let img = Image::from_srgba8(decoded.width(), decoded.height(), decoded.as_raw());
        self.cache.lock().expect("cache lock").insert(sha256.to_string(), Arc::new(img));
        Ok(())
    }

    /// Keeps a picture that was made, not read, for the asset with this hash.
    pub fn insert(&self, sha256: &str, img: Image) {
        self.cache.lock().expect("cache lock").insert(sha256.to_string(), Arc::new(img));
    }

    /// Whether the picture of the asset with this hash is here.
    pub fn has(&self, sha256: &str) -> bool {
        self.cache.lock().expect("cache lock").contains_key(sha256)
    }

    fn load(&self, asset: &Asset) -> Result<Option<Arc<Image>>, String> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let path = asset.uri.strip_prefix("file://").unwrap_or(&asset.uri);
            if asset.uri.contains("://") && !asset.uri.starts_with("file://") {
                return Ok(None);
            }
            match std::fs::read(path) {
                Ok(bytes) => {
                    self.insert_bytes(&asset.sha256, &bytes)?;
                    Ok(self.cache.lock().expect("cache lock").get(&asset.sha256).cloned())
                }
                Err(_) => Ok(None),
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = asset;
            Ok(None)
        }
    }
}

impl MediaSource for StillImages {
    fn picture(&self, _: &str, asset: &Asset, _: Tick) -> Result<Option<Arc<Image>>, String> {
        if asset.kind != AssetKind::Image {
            return Ok(None);
        }
        if let Some(img) = self.cache.lock().expect("cache lock").get(&asset.sha256) {
            return Ok(Some(img.clone()));
        }
        self.load(asset)
    }
}
