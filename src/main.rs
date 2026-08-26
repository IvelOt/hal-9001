use anyhow::Result;

use hal9001::config::Config;

#[tokio::main]
async fn main() -> Result<()> {
    hal9001::logging::init();

    let config = Config::load();

    let terminal = ratatui::init();
    let result = hal9001::run(terminal, config).await;
    ratatui::restore();

    result
}
