//! Interface TUI (Ratatui) do HAL-9001.
//!
//! Dashboard multi-abas (Overview, Discos, Rede, Bluetooth, AI Deck) com estética
//! **Retro Terminal Minimalista**: verde acentuado `#00FF66`, ciano `#00E5FF` e
//! cinza escuro `#1A1D23`, com contornos limpos e cantos arredondados.
//!
//! Conforme seção 3 (`src/ui/`) de `docs/backend_architecture.md`.

pub mod dashboard;
pub mod file_manager;
pub mod toast;
pub mod yazi_dock;

use ratatui::style::Color;

pub use dashboard::Dashboard;
pub use yazi_dock::YaziDock;

/// Fundo escuro da interface (cinza escuro `#1A1D23`).
pub const BG: Color = Color::Rgb(0x1A, 0x1D, 0x23);
/// Verde acentuado (`#00FF66`) — destaques, valores e ações.
pub const ACCENT: Color = Color::Rgb(0x00, 0xFF, 0x66);
/// Ciano (`#00E5FF`) — destaques secundários, seleção e títulos.
pub const CYAN: Color = Color::Rgb(0x00, 0xE5, 0xFF);
/// Cinza intermediário para bordas e separadores.
pub const DIM: Color = Color::Rgb(0x2A, 0x2F, 0x3A);
/// Cinza apagado para texto secundário.
pub const GRAY: Color = Color::Rgb(0x8B, 0x92, 0x9E);
/// Vermelho para estados de erro / perigo.
pub const DANGER: Color = Color::Rgb(0xFF, 0x55, 0x55);
/// Âmbar para avisos (ex.: bateria crítica, Wi-Fi desligado).
pub const WARN: Color = Color::Rgb(0xFF, 0xB8, 0x4D);
/// Cor de texto principal.
pub const TEXT: Color = Color::Rgb(0xE6, 0xEA, 0xF0);

/// Mapeia um tema de acento (config) para a cor primária de destaque.
pub fn accent_color(theme: crate::config::AccentTheme) -> Color {
    use crate::config::AccentTheme;
    match theme {
        AccentTheme::Green => Color::Rgb(0x00, 0xFF, 0x66),
        AccentTheme::Cyan => Color::Rgb(0x00, 0xE5, 0xFF),
        AccentTheme::Magenta => Color::Rgb(0xFF, 0x3A, 0xD0),
        AccentTheme::Retro => Color::Rgb(0x00, 0xFF, 0x66),
    }
}
