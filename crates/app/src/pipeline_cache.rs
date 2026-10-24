//! A disk-backed wgpu pipeline cache, so a second launch reuses the driver's
//! compiled pipelines instead of compiling them all again (startup spends
//! most of its time there). Needs `Features::PIPELINE_CACHE` (Vulkan); without
//! it `get()` is `None` and pipelines are built as before.
//! `PLANET_PIPELINE_CACHE=0` turns it off.

use std::sync::OnceLock;

static PATH: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
static CACHE: OnceLock<Option<wgpu::PipelineCache>> = OnceLock::new();

fn path(adapter_info: &wgpu::AdapterInfo) -> Option<std::path::PathBuf> {
    let key = wgpu::util::pipeline_cache_key(adapter_info)?;
    Some(std::path::PathBuf::from("test-runs/pipeline_cache").join(key))
}

pub fn enabled() -> bool {
    !matches!(
        std::env::var("PLANET_PIPELINE_CACHE")
            .ok()
            .as_deref()
            .map(str::trim),
        Some("0" | "false" | "off")
    )
}

pub fn init(device: &wgpu::Device, adapter_info: &wgpu::AdapterInfo) {
    let _ = PATH.set(path(adapter_info));
    let cache =
        (enabled() && device.features().contains(wgpu::Features::PIPELINE_CACHE)).then(|| {
            let data = path(adapter_info).and_then(|p| std::fs::read(p).ok());
            tracing::info!(
                target: "planet::startup",
                cached_bytes = data.as_ref().map_or(0, Vec::len),
                "pipeline cache opened"
            );
            // SAFETY: the data is only ever written by `save` from this
            // device's own cache; `fallback` discards it if the driver rejects it.
            unsafe {
                device.create_pipeline_cache(&wgpu::PipelineCacheDescriptor {
                    label: Some("planet pipeline cache"),
                    data: data.as_deref(),
                    fallback: true,
                })
            }
        });
    let _ = CACHE.set(cache);
}

pub fn get() -> Option<&'static wgpu::PipelineCache> {
    CACHE.get().and_then(Option::as_ref)
}

/// Writes the cache out (call once startup has built its pipelines).
pub fn save() {
    let (Some(cache), Some(Some(path))) = (get(), PATH.get()) else {
        return;
    };
    if let Some(data) = cache.get_data() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("tmp");
        if std::fs::write(&tmp, &data).is_ok() {
            let _ = std::fs::rename(&tmp, path);
        }
    }
}
