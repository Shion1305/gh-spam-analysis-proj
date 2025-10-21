use tracing_subscriber::{fmt, EnvFilter};

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
