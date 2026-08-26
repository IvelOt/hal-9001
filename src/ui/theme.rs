
use ratatui::style::Color;

use crate::config::Config;

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

    pub fn from_config(config: &Config) -> Palette {
        match config.theme.name.to_lowercase().as_str() {
            "catppuccin" | "mocha" => Palette {
                bg: Color::Reset,
                fg: Color::Rgb(0xCD, 0xD6, 0xF4),
                dim: Color::Rgb(0x6C, 0x70, 0x86),
                accent: Color::Rgb(0x89, 0xB4, 0xFA),
                ok: Color::Rgb(0xA6, 0xE3, 0xA1),
                warn: Color::Rgb(0xF9, 0xE2, 0xAF),
                err: Color::Rgb(0xF3, 0x8B, 0xA8),
            },
            "tokyo-night" | "tokyonight" => Palette {
                bg: Color::Reset,
                fg: Color::Rgb(0xC0, 0xCA, 0xF5),
                dim: Color::Rgb(0x56, 0x5F, 0x89),
                accent: Color::Rgb(0x7A, 0xA2, 0xF7),
                ok: Color::Rgb(0x9E, 0xCE, 0x6A),
                warn: Color::Rgb(0xE0, 0xAF, 0x68),
                err: Color::Rgb(0xF7, 0x76, 0x8E),
            },
            "nord" => Palette {
                bg: Color::Reset,
                fg: Color::Rgb(0xEC, 0xEF, 0xF4),
                dim: Color::Rgb(0x4C, 0x56, 0x6A),
                accent: Color::Rgb(0x88, 0xC0, 0xD0),
                ok: Color::Rgb(0xA3, 0xBE, 0x8C),
                warn: Color::Rgb(0xEB, 0xCB, 0x8B),
                err: Color::Rgb(0xBF, 0x61, 0x6A),
            },
            "gruvbox" => Palette {
                bg: Color::Reset,
                fg: Color::Rgb(0xEB, 0xDB, 0xB2),
                dim: Color::Rgb(0x92, 0x83, 0x74),
                accent: Color::Rgb(0xFE, 0x80, 0x19),
                ok: Color::Rgb(0xB8, 0xBB, 0x26),
                warn: Color::Rgb(0xFA, 0xBD, 0x2F),
                err: Color::Rgb(0xFB, 0x49, 0x34),
            },
            "cyberpunk" => Palette {
                bg: Color::Reset,
                fg: Color::Rgb(0x00, 0xFF, 0xCC),
                dim: Color::Rgb(0x62, 0x72, 0xA4),
                accent: Color::Rgb(0xFC, 0xEE, 0x0A),
                ok: Color::Rgb(0x00, 0xFF, 0x9F),
                warn: Color::Rgb(0xFF, 0x00, 0x7F),
                err: Color::Rgb(0xFF, 0x00, 0x3C),
            },
            "dracula" => Palette {
                bg: Color::Reset,
                fg: Color::Rgb(0xF8, 0xF8, 0xF2),
                dim: Color::Rgb(0x62, 0x72, 0xA4),
                accent: Color::Rgb(0xBD, 0x93, 0xF9),
                ok: Color::Rgb(0x50, 0xFA, 0x7B),
                warn: Color::Rgb(0xFF, 0xB8, 0x6C),
                err: Color::Rgb(0xFF, 0x55, 0x55),
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
