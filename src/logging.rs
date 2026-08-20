//! Logging estruturado para arquivo (nunca para stdout/stderr em modo TUI,
//! pois corromperia a tela alternada).
//!
//! Ativado apenas quando `HAL9001_LOG=<caminho>` está definido. Best-effort:
//! qualquer falha é silenciosamente ignorada para não impedir o boot da TUI.

/// Inicializa o subscriber de `tracing` se `HAL9001_LOG` apontar para um
/// arquivo gravável. No-op caso contrário.
pub fn init() {
    let Ok(path) = std::env::var("HAL9001_LOG") else {
        return;
    };
    let Ok(file) = std::fs::File::create(&path) else {
        return;
    };

    // Closure `Fn() -> File` satisfaz `MakeWriter`; clonamos o handle a cada
    // escrita para não exigir sincronização externa.
    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("HAL9001_LOG_LEVEL")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(move || file.try_clone().expect("clonar handle do log"))
        .try_init();
}
