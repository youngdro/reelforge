//! Where pictures come from.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use reelforge_renderspec::{Asset, AssetKind};
use reelforge_time::Tick;

use crate::Image;

/// Supplies decoded pictures to the planner.
///
/// Implementations: [`StillImages`] for image assets; a video decoder (FFmpeg-free, WebCodecs in
/// the browser, the platform's hardware decoder natively) implements this same trait.
pub trait MediaSource {
    /// The picture of `asset` at source time `at`. `Ok(None)` means "not available yet" (offline,
    /// still loading, or a kind this source does not decode).
    fn picture(&self, asset_id: &str, asset: &Asset, at: Tick) -> Result<Option<Arc<Image>>, String>;

    /// The bytes of a `font` asset (TrueType or OpenType). `Ok(None)` means not available; the
    /// default implementation has no fonts.
    fn font(&self, asset_id: &str, asset: &Asset) -> Result<Option<Arc<[u8]>>, String> {
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
