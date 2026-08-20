//! Paleta de cores derivada da [`Config`].

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
        match config.theme.name.as_str() {
            "mono" => Palette {
                bg: Color::Reset,
                fg: Color::Gray,
                dim: Color::DarkGray,
                accent: Color::White,
                ok: Color::Gray,
                warn: Color::Gray,
                err: Color::White,
            },
            // "hal" (padrão): âmbar de cockpit sobre fundo escuro.
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
