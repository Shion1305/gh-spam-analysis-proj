fn main() {
    // Default to SQLx offline mode, but allow callers to override when preparing metadata.
    let offline = std::env::var("SQLX_OFFLINE").unwrap_or_else(|_| "true".to_owned());
    println!("cargo:rustc-env=SQLX_OFFLINE={offline}");
}
