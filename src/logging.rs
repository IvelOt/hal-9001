pub fn init() {
    let Ok(path) = std::env::var("HAL9001_LOG") else {
        return;
    };
    let Ok(file) = std::fs::File::create(&path) else {
        return;
    };

    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("HAL9001_LOG_LEVEL")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(move || file.try_clone().expect("clonar handle do log"))
        .try_init();
}
