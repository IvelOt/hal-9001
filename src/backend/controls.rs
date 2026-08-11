//! Controles de hardware de resposta imediata via utilitários CLI seguros:
//! **`wpctl`** (WirePlumber / PipeWire) para volume e **`brightnessctl`** para brilho.
//!
//! Conforme seção 1.5 de `docs/backend_architecture.md`. Todas as operações são
//! assíncronas (`tokio::process::Command`).

use anyhow::{anyhow, Context, Result};
use tokio::process::Command;

/// Backend de controles de hardware (áudio e brilho).
pub struct Controls;

impl Controls {
    /// Cria o backend de controles. Não exige inicialização, mas documenta o
    /// ponto de entrada para futuros estados/limites.
    pub async fn new() -> Result<Self> {
        Ok(Self)
    }

    // ------------------------------------------------------------------
    // Áudio (wpctl)
    // ------------------------------------------------------------------

    /// Lê o volume atual do sink padrão como fração (0.0 a 1.0).
    pub async fn get_volume(&self) -> Result<f64> {
        let output = Command::new("wpctl")
            .args(["get-volume", "@DEFAULT_AUDIO_SINK@"])
            .output()
            .await
            .context("falha ao executar `wpctl get-volume`")?;

        if !output.status.success() {
            return Err(anyhow!(
                "`wpctl get-volume` falhou: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_wpctl_volume(&stdout).context("não foi possível interpretar a saída de `wpctl get-volume`")
    }

    /// Define o volume do sink padrão a partir de uma fração (0.0 a 1.0).
    ///
    /// Ação mutável — será acionada pela TUI / Gatekeeper de consentimento.
    #[allow(dead_code)]
    pub async fn set_volume(&self, volume: f64) -> Result<()> {
        let volume = volume.clamp(0.0, 1.0);
        let status = Command::new("wpctl")
            .args(["set-volume", "@DEFAULT_AUDIO_SINK@", &format!("{volume:.2}")])
            .status()
            .await
            .context("falha ao executar `wpctl set-volume`")?;

        if !status.success() {
            return Err(anyhow!("`wpctl set-volume` retornou status {status}"));
        }
        Ok(())
    }

    /// Alterna o mudo do sink padrão.
    ///
    /// Ação mutável — será acionada pela TUI / Gatekeeper de consentimento.
    #[allow(dead_code)]
    pub async fn toggle_mute(&self) -> Result<()> {
        let status = Command::new("wpctl")
            .args(["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"])
            .status()
            .await
            .context("falha ao executar `wpctl set-mute toggle`")?;

        if !status.success() {
            return Err(anyhow!("`wpctl set-mute toggle` retornou status {status}"));
        }
        Ok(())
    }

    // ------------------------------------------------------------------
    // Brilho (brightnessctl)
    // ------------------------------------------------------------------

    /// Retorna o brilho atual (0..100), calculado sobre o valor máximo reportado.
    pub async fn get_brightness_percent(&self) -> Result<u8> {
        let current = self.brightness_value("get").await?;
        let max = self.brightness_value("max").await?;
        if max == 0 {
            return Err(anyhow!("`brightnessctl max` retornou 0"));
        }
        Ok(((current as f64 / max as f64) * 100.0).round() as u8)
    }

    /// Define o brilho em percentual (0..100).
    ///
    /// Ação mutável — será acionada pela TUI / Gatekeeper de consentimento.
    #[allow(dead_code)]
    pub async fn set_brightness_percent(&self, percent: u8) -> Result<()> {
        let percent = percent.min(100);
        let status = Command::new("brightnessctl")
            .args(["set", &format!("{percent}%")])
            .status()
            .await
            .context("falha ao executar `brightnessctl set`")?;

        if !status.success() {
            return Err(anyhow!("`brightnessctl set {percent}%` retornou status {status}"));
        }
        Ok(())
    }

    /// Lê um valor numérico bruto do `brightnessctl` (ex.: `get`, `max`).
    #[allow(dead_code)]
    async fn brightness_value(&self, subcommand: &str) -> Result<u64> {
        let output = Command::new("brightnessctl")
            .arg(subcommand)
            .output()
            .await
            .with_context(|| format!("falha ao executar `brightnessctl {subcommand}`"))?;

        if !output.status.success() {
            return Err(anyhow!(
                "`brightnessctl {subcommand}` falhou: {}",
                String::from_utf8_lossy(&output.stderr)
            ));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout
            .trim()
            .parse::<u64>()
            .with_context(|| format!("saída não numérica de `brightnessctl {subcommand}`: {stdout:?}"))
    }
}

/// Interpreta a saída de `wpctl get-volume`, ex.: `Volume: 0.55` ou `Volume: 0.55 [MUTED]`.
fn parse_wpctl_volume(stdout: &str) -> Option<f64> {
    let line = stdout.lines().next()?;
    let value = line.strip_prefix("Volume: ")?;
    let value = value.split_whitespace().next()?;
    value.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_wpctl_volume() {
        assert_eq!(parse_wpctl_volume("Volume: 0.55\n"), Some(0.55));
        assert_eq!(parse_wpctl_volume("Volume: 0.55 [MUTED]\n"), Some(0.55));
        assert_eq!(parse_wpctl_volume("Volume: 1.00 [MUTED]\n"), Some(1.0));
        assert_eq!(parse_wpctl_volume("lixo"), None);
    }
}
