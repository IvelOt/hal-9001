//! HAL-9001 — entrypoint do binário `hal9001`.
//!
//! Responsável apenas por: inicializar logging, carregar config, preparar o
//! terminal (raw mode + alt-screen via `ratatui::init`, que também instala um
//! panic hook restaurador) e delegar ao loop principal em `hal9001::run`.

use anyhow::Result;

use hal9001::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    hal9001::logging::init();

    let config = Config::load();

    // `ratatui::init()` entra em raw mode + alternate screen e instala um
    // panic hook que restaura o terminal antes de imprimir o backtrace.
    let terminal = ratatui::init();
    let result = hal9001::run(terminal, config).await;
    ratatui::restore();

    result
}
