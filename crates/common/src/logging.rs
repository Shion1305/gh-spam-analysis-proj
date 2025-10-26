#[cfg(feature = "otel")]
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};

// OpenTelemetry-related imports are gated behind the `otel` feature
#[cfg(feature = "otel")]
use {
    opentelemetry::KeyValue,
    opentelemetry_otlp::WithExportConfig,
    opentelemetry_sdk::{
        propagation::TraceContextPropagator, runtime, trace as sdktrace, Resource,
    },
};

pub fn init_logging(default_level: &str) {
    if tracing::dispatcher::has_been_set() {
        return;
    }

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    fmt()
        .with_env_filter(env_filter)
        .with_writer(std::io::stderr)
        .init();
}

/// Initialize tracing with optional OpenTelemetry OTLP exporter.
///
/// If `OTEL_EXPORTER_OTLP_ENDPOINT` is set, this sets up an OTLP exporter (gRPC by default,
/// or HTTP if `OTEL_EXPORTER_OTLP_PROTOCOL=http/protobuf`) and adds a tracing layer to
/// forward spans. Otherwise, it falls back to plain stderr logging.
#[cfg(feature = "otel")]
pub fn init_tracing(service_name: &str, default_level: &str) {
    if tracing::dispatcher::has_been_set() {
        return;
    }

    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));

    let fmt_layer = fmt::layer().with_writer(std::io::stderr);

    let maybe_endpoint = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").ok();
    if let Some(endpoint) = maybe_endpoint {
        // Enable W3C trace-context propagation for HTTP services
        opentelemetry::global::set_text_map_propagator(TraceContextPropagator::new());

        let resource = Resource::new(vec![KeyValue::new(
            "service.name",
            service_name.to_string(),
        )]);

        let tracer = opentelemetry_otlp::new_pipeline()
            .tracing()
            .with_trace_config(sdktrace::Config::default().with_resource(resource))
            .with_exporter(
                opentelemetry_otlp::new_exporter()
                    .tonic()
                    .with_endpoint(endpoint),
            )
            .install_batch(runtime::Tokio);

        match tracer {
            Ok(tracer) => {
                let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
                let _ = tracing_subscriber::registry()
                    .with(env_filter)
                    .with(fmt_layer)
                    .with(otel_layer)
                    .try_init();
                return;
            }
            Err(err) => {
                eprintln!("failed to initialize OTLP exporter: {err}");
            }
        }
    }

    // Fallback to plain logging
    let _ = tracing_subscriber::registry()
        .with(env_filter)
        .with(fmt_layer)
        .try_init();
}

// Non-otel build: fall back to plain stderr logging
#[cfg(not(feature = "otel"))]
pub fn init_tracing(_service_name: &str, default_level: &str) {
    init_logging(default_level);
}

// Gracefully shutdown tracer provider if present; no-op without `otel` feature
#[cfg(feature = "otel")]
pub fn shutdown_tracer_provider() {
    opentelemetry::global::shutdown_tracer_provider();
}

#[cfg(not(feature = "otel"))]
pub fn shutdown_tracer_provider() {}
