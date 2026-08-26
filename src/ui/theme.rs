//! Paleta de cores derivada da [`Config`].
//!
//! Suporta temas embutidos de alta fidelidade:
//! - `hal` (Âmbar Sci-Fi / Assistente de Sistema, padrão)
//! - `catppuccin` (Catppuccin Mocha)
//! - `tokyo-night` (Tokyo Night)
//! - `nord` (Nord Arctic)
//! - `gruvbox` (Gruvbox Dark)
//! - `cyberpunk` (Cyberpunk 2077 Neon)
//! - `dracula` (Dracula)
//! - `mono` (Monocromático puro)

use ratatui::style::Color;

use crate::config::Config;

/// Paleta usada por toda a UI.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Color,
    pub fg: Color,
    pub dim: Color,
    pub accent: Color,
    pub ok: Color,
    pub warn: Color,
    pub err: Color,
}

impl Palette {
    /// Constrói a paleta a partir do nome de tema na config.
    pub fn from_config(config: &Config) -> Palette {
        match config.theme.name.to_lowercase().as_str() {
            "catppuccin" | "mocha" => Palette {
                bg: Color::Reset,
                fg: Color::Rgb(0xCD, 0xD6, 0xF4), // Text (#cdd6f4)
                dim: Color::Rgb(0x6C, 0x70, 0x86), // Overlay0 (#6c7086)
                accent: Color::Rgb(0x89, 0xB4, 0xFA), // Blue (#89b4fa)
                ok: Color::Rgb(0xA6, 0xE3, 0xA1), // Green (#a6e3a1)
                warn: Color::Rgb(0xF9, 0xE2, 0xAF), // Yellow (#f9e2af)
                err: Color::Rgb(0xF3, 0x8B, 0xA8), // Red (#f38ba8)
            },
            "tokyo-night" | "tokyonight" => Palette {
                bg: Color::Reset,
                fg: Color::Rgb(0xC0, 0xCA, 0xF5), // Foreground (#c0caf5)
                dim: Color::Rgb(0x56, 0x5F, 0x89), // Comment (#565f89)
                accent: Color::Rgb(0x7A, 0xA2, 0xF7), // Blue (#7aa2f7)
                ok: Color::Rgb(0x9E, 0xCE, 0x6A), // Green (#9ece6a)
                warn: Color::Rgb(0xE0, 0xAF, 0x68), // Yellow/Orange (#e0af68)
                err: Color::Rgb(0xF7, 0x76, 0x8E), // Red (#f7768e)
            },
            "nord" => Palette {
                bg: Color::Reset,
                fg: Color::Rgb(0xEC, 0xEF, 0xF4), // Snow Storm (#eceff4)
                dim: Color::Rgb(0x4C, 0x56, 0x6A), // Polar Night (#4c566a)
                accent: Color::Rgb(0x88, 0xC0, 0xD0), // Frost Cyan (#88c0d0)
                ok: Color::Rgb(0xA3, 0xBE, 0x8C), // Aurora Green (#a3be8c)
                warn: Color::Rgb(0xEB, 0xCB, 0x8B), // Aurora Yellow (#ebcb8b)
                err: Color::Rgb(0xBF, 0x61, 0x6A), // Aurora Red (#bf616a)
            },
            "gruvbox" => Palette {
                bg: Color::Reset,
                fg: Color::Rgb(0xEB, 0xDB, 0xB2), // Light 1 (#ebdbb2)
                dim: Color::Rgb(0x92, 0x83, 0x74), // Gray (#928374)
                accent: Color::Rgb(0xFE, 0x80, 0x19), // Orange (#fe8019)
                ok: Color::Rgb(0xB8, 0xBB, 0x26), // Green (#b8bb26)
                warn: Color::Rgb(0xFA, 0xBD, 0x2F), // Yellow (#fabd2f)
                err: Color::Rgb(0xFB, 0x49, 0x34), // Red (#fb4934)
            },
            "cyberpunk" => Palette {
                bg: Color::Reset,
                fg: Color::Rgb(0x00, 0xFF, 0xCC), // Neon Cyan (#00ffcc)
                dim: Color::Rgb(0x62, 0x72, 0xA4), // Slate (#6272a4)
                accent: Color::Rgb(0xFC, 0xEE, 0x0A), // Neon Yellow (#fcee0a)
                ok: Color::Rgb(0x00, 0xFF, 0x9F), // Neon Green (#00ff9f)
                warn: Color::Rgb(0xFF, 0x00, 0x7F), // Neon Pink (#ff007f)
                err: Color::Rgb(0xFF, 0x00, 0x3C), // Cyber Red (#ff003c)
            },
            "dracula" => Palette {
                bg: Color::Reset,
                fg: Color::Rgb(0xF8, 0xF8, 0xF2), // Foreground (#f8f8f2)
                dim: Color::Rgb(0x62, 0x72, 0xA4), // Comment (#6272a4)
                accent: Color::Rgb(0xBD, 0x93, 0xF9), // Purple (#bd93f9)
                ok: Color::Rgb(0x50, 0xFA, 0x7B), // Green (#50fa7b)
                warn: Color::Rgb(0xFF, 0xB8, 0x6C), // Orange (#ffb86c)
                err: Color::Rgb(0xFF, 0x55, 0x55), // Red (#ff5555)
            },
            "mono" => Palette {
                bg: Color::Reset,
                fg: Color::Gray,
                dim: Color::DarkGray,
                accent: Color::White,
                ok: Color::Gray,
                warn: Color::Gray,
                err: Color::White,
            },
            // "hal" (padrão): âmbar do Assistente de Sistema sobre fundo escuro.
            _ => Palette {
                bg: Color::Reset,
                fg: Color::Rgb(0xD8, 0xD8, 0xD2),
                dim: Color::Rgb(0x6C, 0x70, 0x6B),
                accent: Color::Rgb(0xFF, 0x8C, 0x1A),
                ok: Color::Rgb(0x8E, 0xC0, 0x7C),
                warn: Color::Rgb(0xE5, 0xC0, 0x7B),
                err: Color::Rgb(0xE0, 0x6C, 0x75),
            },
        }
    }

    /// Cor do besouro conforme a posição vertical `t` em 0.0..=1.0, produzindo
    /// um gradiente do topo (mais escuro) à base (mais claro) quando o acento é
    /// RGB. Temas de cor nomeada (ex.: `mono`) mantêm o acento fixo.
    pub fn gradient(&self, t: f64) -> Color {
        match self.accent {
            Color::Rgb(r, g, b) => {
                let f = 0.65 + 0.5 * t.clamp(0.0, 1.0);
                let scale = |c: u8| ((c as f64 * f).round() as u32).min(255) as u8;
                Color::Rgb(scale(r), scale(g), scale(b))
            }
            other => other,
        }
    }

    /// Cor de uma barra conforme a fração preenchida (verde→amarelo→vermelho).
    pub fn gauge_color(&self, ratio: f64) -> Color {
        if ratio >= 0.85 {
            self.err
        } else if ratio >= 0.6 {
            self.warn
        } else {
            self.ok
        }
    }
}
