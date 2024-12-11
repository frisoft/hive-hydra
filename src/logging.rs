use tracing::{info, Level};
use tracing_subscriber::{
    fmt::{self, time::LocalTime},
    layer::SubscriberExt,
    Registry,
};
use tracing_appender::rolling::{RollingFileAppender, Rotation};

pub fn setup_logging() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Create logs directory if it doesn't exist
    std::fs::create_dir_all("logs")?;

    // File appender
    let file_appender = RollingFileAppender::new(
        Rotation::DAILY,
        "logs",
        "hive-hydra.log",
    );

    // Console layer with colored output and local time
    let console_layer = fmt::layer()
        .with_target(false)
        .with_thread_names(false)
        .with_thread_ids(true)
        .with_line_number(false)
        .with_timer(LocalTime::rfc_3339())
        .with_file(false)
        .with_level(true);

    // File layer with more detailed output
    let file_layer = fmt::layer()
        .with_target(false)
        .with_thread_names(false)
        .with_thread_ids(true)
        .with_ansi(false)
        .with_file(false)
        .with_line_number(false)
        .with_timer(LocalTime::rfc_3339())
        .with_writer(file_appender);

    // Combine layers
    let subscriber = Registry::default()
        .with(console_layer)
        .with(file_layer);

    // Set as global default
    tracing::subscriber::set_global_default(subscriber)?;

    // Log initial message to verify setup
    info!("Logging system initialized");

    Ok(())
}
