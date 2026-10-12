#![deny(unused_imports)]

use std::fmt::Debug;

use maplibre::{
    environment::{OffscreenKernel, OffscreenKernelConfig},
    event_loop::{EventLoop, MapEvent},
    io::source_client::{HttpSourceClient, SourceClient},
    kernel::{Kernel, KernelBuilder},
    map::Map,
    render::builder::RendererBuilder,
    style::Style,
};
use maplibre_winit::{WinitEnvironment, WinitEventLoopProxy, WinitMapWindowConfig};
use wasm_bindgen::prelude::*;

use crate::{
    error::JSError,
    platform::{http_client::WHATWGFetchHttpClient, UsedOffscreenKernelEnvironment},
};

mod error;
mod platform;

#[cfg(not(target_feature = "atomics"))]
pub use platform::singlethreaded::wasm_entries::worker_factory;

#[cfg(not(any(no_pendantic_os_check, target_arch = "wasm32")))]
compile_error!("web works only on wasm32.");

#[cfg(feature = "trace")]
fn enable_tracing() {
    use tracing_subscriber::{layer::SubscriberExt, Registry};

    let mut builder = tracing_wasm::WASMLayerConfigBuilder::new();
    builder.set_report_logs_in_timings(true);
    builder.set_console_config(tracing_wasm::ConsoleConfig::NoReporting);

    tracing::subscriber::set_global_default(
        Registry::default().with(tracing_wasm::WASMLayer::new(builder.build())),
    );
}

/// Sets up logging and the panic hook. Runs automatically with the default `start` feature;
/// embedders that provide their own start function disable it and call this instead.
#[cfg_attr(feature = "start", wasm_bindgen(start))]
pub fn wasm_bindgen_start() {
    if console_log::init_with_level(log::Level::Info).is_err() {
        // Failed to initialize logging. No need to log a message.
    }
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));

    #[cfg(any(feature = "trace"))]
    enable_tracing();
}

pub struct WHATWGOffscreenKernelEnvironment;

impl OffscreenKernel for WHATWGOffscreenKernelEnvironment {
    type HttpClient = WHATWGFetchHttpClient;

    fn create(config: OffscreenKernelConfig) -> Self {
        WHATWGOffscreenKernelEnvironment
    }

    fn source_client(&self) -> SourceClient<Self::HttpClient> {
        SourceClient::new(HttpSourceClient::new(WHATWGFetchHttpClient::default()))
    }
}

#[cfg(not(target_feature = "atomics"))]
type CurrentEnvironment<ET> = WinitEnvironment<
    maplibre::io::scheduler::NopScheduler,
    WHATWGFetchHttpClient,
    UsedOffscreenKernelEnvironment,
    platform::singlethreaded::apc::PassingAsyncProcedureCall,
    ET,
>;

#[cfg(target_feature = "atomics")]
type CurrentEnvironment<ET> = WinitEnvironment<
    platform::multithreaded::pool_scheduler::WebWorkerPoolScheduler,
    WHATWGFetchHttpClient,
    UsedOffscreenKernelEnvironment,
    maplibre::io::apc::SchedulerAsyncProcedureCall<
        UsedOffscreenKernelEnvironment,
        platform::multithreaded::pool_scheduler::WebWorkerPoolScheduler,
    >,
    ET,
>;

pub type MapType = Map<CurrentEnvironment<()>>;

#[wasm_bindgen]
pub async fn run_maplibre(new_worker: js_sys::Function) -> Result<(), JSError> {
    run_map::<()>("maplibre", Style::default(), new_worker)
        .await
        .map(|_proxy| ())
}

/// Starts a map with `style` on the canvas with id `canvas_id` and returns a proxy for sending
/// [`MapEvent`]s to it. `new_worker` creates the web workers that fetch and process tiles; in single-threaded
/// builds [`worker_factory`] creates it from a worker script URL.
pub async fn run_map<ET: MapEvent + Clone + PartialEq + Debug>(
    canvas_id: &str,
    style: Style,
    new_worker: js_sys::Function,
) -> Result<WinitEventLoopProxy<ET>, JSError> {
    let mut kernel_builder = KernelBuilder::new()
        .with_map_window_config(WinitMapWindowConfig::new(canvas_id.to_string()))
        .with_http_client(WHATWGFetchHttpClient::default());

    let offscreen_kernel_config = OffscreenKernelConfig {
        cache_directory: None,
    };

    #[cfg(target_feature = "atomics")]
    {
        kernel_builder = kernel_builder
            .with_apc(maplibre::io::apc::SchedulerAsyncProcedureCall::new(
                platform::multithreaded::pool_scheduler::WebWorkerPoolScheduler::new(
                    new_worker.clone(),
                )?,
                offscreen_kernel_config,
            ))
            .with_scheduler(
                platform::multithreaded::pool_scheduler::WebWorkerPoolScheduler::new(new_worker)?,
            );
    }

    #[cfg(not(target_feature = "atomics"))]
    {
        kernel_builder = kernel_builder
            .with_apc(
                platform::singlethreaded::apc::PassingAsyncProcedureCall::new(
                    new_worker,
                    4,
                    offscreen_kernel_config,
                )?,
            )
            .with_scheduler(maplibre::io::scheduler::NopScheduler);
    }

    let kernel: Kernel<WinitEnvironment<_, _, UsedOffscreenKernelEnvironment, _, ET>> =
        kernel_builder.build();

    let mut map: Map<CurrentEnvironment<ET>> = Map::new(
        style,
        kernel,
        RendererBuilder::new(),
        vec![
            Box::<maplibre::render::RenderPlugin>::default(),
            Box::<maplibre::background::BackgroundPlugin>::default(),
            Box::<maplibre::vector::VectorPlugin<platform::UsedVectorTransferables>>::default(),
            Box::new(maplibre::sdf::SdfPlugin::<platform::UsedVectorTransferables>::default()),
            // Box::new(RasterPlugin::<platform::UsedRasterTransferables>::default()),
            #[cfg(all(debug_assertions, feature = "debug-overlay"))]
            Box::<maplibre::debug::DebugPlugin>::default(),
        ],
    )
    .unwrap();
    map.initialize_renderer().await.unwrap();

    let event_loop = map
        .window_mut()
        .take_event_loop()
        .expect("Event loop is not available");
    let proxy = event_loop.create_proxy();
    event_loop.run(map, None);

    Ok(proxy)
}

#[cfg(test)]
/// See https://rustwasm.github.io/wasm-bindgen/wasm-bindgen-test/browsers.html
mod tests {
    use wasm_bindgen_test::*;
    wasm_bindgen_test_configure!(run_in_browser);

    #[wasm_bindgen_test]
    fn pass() {
        assert_eq!(1, 1);
    }
}
