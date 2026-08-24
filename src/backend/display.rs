//! Backend do Módulo de Monitores & Displays (X11 / xrandr).
//!
//! 100% Pure Rust — Zero dependências de novas crates externas.
//! Fornece detecção de telas, auto-expansão automática ao plugar monitor e controle de modos.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::events::{Action, AppEvent, EventTx, Toast};

/// Modos de arranjo/layout entre múltiplos monitores.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DisplayLayoutMode {
    /// Expandir monitor secundário à direita do primário (padrão auto-expand)
    ExtendRight,
    /// Expandir monitor secundário à esquerda do primário
    ExtendLeft,
    /// Espelhar tela (mesma imagem em ambos os monitores)
    Mirror,
    /// Apenas o monitor externo ligado
    ExternalOnly,
    /// Apenas a tela do notebook ligada
    InternalOnly,
    /// Customizado / Outro arranjo
    Custom,
}

impl DisplayLayoutMode {
    pub const ALL: [DisplayLayoutMode; 5] = [
        DisplayLayoutMode::ExtendRight,
        DisplayLayoutMode::ExtendLeft,
        DisplayLayoutMode::Mirror,
        DisplayLayoutMode::ExternalOnly,
        DisplayLayoutMode::InternalOnly,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Self::ExtendRight => "Expandir à Direita",
            Self::ExtendLeft => "Expandir à Esquerda",
            Self::Mirror => "Espelhar Telas",
            Self::ExternalOnly => "Apenas Monitor Externo",
            Self::InternalOnly => "Apenas Tela do Notebook",
            Self::Custom => "Personalizado",
        }
    }

    pub fn ascii_badge(&self) -> &'static str {
        match self {
            Self::ExtendRight => "[EXPAND-DIR]",
            Self::ExtendLeft => "[EXPAND-ESQ]",
            Self::Mirror => "[ESPELHO   ]",
            Self::ExternalOnly => "[SO-EXTERNO]",
            Self::InternalOnly => "[SO-INTERNO]",
            Self::Custom => "[CUSTOM    ]",
        }
    }
}

/// Modo de resolução e taxa de atualização suportado por um monitor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayMode {
    pub width: u32,
    pub height: u32,
    pub rate: f32,
    pub is_current: bool,
    pub is_preferred: bool,
}

impl DisplayMode {
    pub fn label(&self) -> String {
        format!("{}x{} @ {:.1}Hz", self.width, self.height, self.rate)
    }
}

/// Representação de uma saída de vídeo / monitor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayNode {
    /// Nome da saída no xrandr (ex: "eDP-1", "HDMI-1", "DP-1")
    pub name: String,
    /// Se o conector possui um cabo de monitor plugado
    pub is_connected: bool,
    /// Se este monitor é a saída primária
    pub is_primary: bool,
    /// Se a saída está ativa e renderizando imagem
    pub is_active: bool,
    /// Resolução e taxa de atualização atual
    pub current_mode: Option<DisplayMode>,
    /// Lista de resoluções suportadas
    pub supported_modes: Vec<DisplayMode>,
    /// Posição X no canvas virtual
    pub pos_x: i32,
    /// Posição Y no canvas virtual
    pub pos_y: i32,
    /// Rotação atual ("normal", "left", "right", "inverted")
    pub rotation: String,
    /// Se é tela interna do notebook (eDP, LVDS, DSI)
    pub is_internal: bool,
}

impl DisplayNode {
    pub fn resolution_str(&self) -> String {
        if let Some(mode) = &self.current_mode {
            mode.label()
        } else if self.is_connected {
            "Desativado".to_string()
        } else {
            "Desconectado".to_string()
        }
    }
}

/// Snapshot consolidado das telas do sistema.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DisplaySnapshot {
    /// Lista de todas as saídas de vídeo detectadas
    pub displays: Vec<DisplayNode>,
    /// Nome do monitor primário
    pub primary_name: Option<String>,
    /// Quantidade de monitores conectados fisicamente
    pub connected_count: usize,
    /// Layout inferido atual
    pub current_layout: Option<DisplayLayoutMode>,
}

impl DisplaySnapshot {
    pub fn internal_display(&self) -> Option<&DisplayNode> {
        self.displays.iter().find(|d| d.is_internal && d.is_connected)
    }

    pub fn external_display(&self) -> Option<&DisplayNode> {
        self.displays.iter().find(|d| !d.is_internal && d.is_connected)
    }

    pub fn connected_displays(&self) -> Vec<&DisplayNode> {
        self.displays.iter().filter(|d| d.is_connected).collect()
    }
}

/// Task assíncrona Tokio para monitoramento e controle de monitores.
pub async fn run(
    poll_interval_ms: u64,
    tx: EventTx,
    mut action_rx: broadcast::Receiver<Action>,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(poll_interval_ms.max(1000)));
    let mut last_connected_names: Vec<String> = Vec::new();
    let mut is_first_run = true;

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Ok(snap) = fetch_display_snapshot().await {
                    // Verificação de Hotplug de Monitores
                    let current_connected: Vec<String> = snap.displays.iter()
                        .filter(|d| d.is_connected)
                        .map(|d| d.name.clone())
                        .collect();

                    if !is_first_run {
                        // Detecta monitores recém-conectados
                        for name in &current_connected {
                            if !last_connected_names.contains(name) {
                                // Novo monitor conectado!
                                let _ = tx.send(AppEvent::Toast(Toast::success(format!(
                                    "Monitor {name} conectado. Ativando modo Expandir..."
                                ))));

                                // Regra de Ouro: Auto-Expandir automaticamente!
                                if let (Some(internal), Some(external)) = (snap.internal_display(), snap.external_display()) {
                                    let _ = apply_layout_xrandr(DisplayLayoutMode::ExtendRight, &internal.name, &external.name).await;
                                }
                            }
                        }

                        // Detecta monitores desconectados
                        for name in &last_connected_names {
                            if !current_connected.contains(name) {
                                let _ = tx.send(AppEvent::Toast(Toast::info(format!(
                                    "Monitor {name} desconectado."
                                ))));
                            }
                        }
                    }

                    last_connected_names = current_connected;
                    is_first_run = false;

                    let _ = tx.send(AppEvent::Display(Box::new(snap)));
                }
            }

            Ok(action) = action_rx.recv() => {
                match action {
                    Action::DisplaySetLayout(layout) => {
                        if let Ok(snap) = fetch_display_snapshot().await {
                            if let (Some(internal), Some(external)) = (snap.internal_display(), snap.external_display()) {
                                let _ = apply_layout_xrandr(layout, &internal.name, &external.name).await;
                                let _ = tx.send(AppEvent::Toast(Toast::success(format!(
                                    "Layout de telas: modo {} aplicado.", layout.title()
                                ))));
                            } else if let Some(internal) = snap.internal_display() {
                                if layout == DisplayLayoutMode::InternalOnly {
                                    let _ = tokio::process::Command::new("xrandr")
                                        .args(["--output", &internal.name, "--auto", "--primary"])
                                        .output().await;
                                }
                            }
                        }
                        if let Ok(snap) = fetch_display_snapshot().await {
                            let _ = tx.send(AppEvent::Display(Box::new(snap)));
                        }
                    }

                    Action::DisplaySetResolution { display, mode, rate } => {
                        let mut args = vec!["--output".to_string(), display.clone(), "--mode".to_string(), mode.clone()];
                        if let Some(r) = rate {
                            args.push("--rate".to_string());
                            args.push(format!("{r:.2}"));
                        }
                        let _ = tokio::process::Command::new("xrandr").args(&args).output().await;
                        let _ = tx.send(AppEvent::Toast(Toast::success(format!(
                            "{display} alterado para {mode}"
                        ))));
                        if let Ok(snap) = fetch_display_snapshot().await {
                            let _ = tx.send(AppEvent::Display(Box::new(snap)));
                        }
                    }

                    Action::DisplaySetPrimary(display) => {
                        let _ = tokio::process::Command::new("xrandr")
                            .args(["--output", &display, "--primary"])
                            .output().await;
                        let _ = tx.send(AppEvent::Toast(Toast::info(format!(
                            "{display} definido como tela principal."
                        ))));
                        if let Ok(snap) = fetch_display_snapshot().await {
                            let _ = tx.send(AppEvent::Display(Box::new(snap)));
                        }
                    }

                    _ => {}
                }
            }
        }
    }
}

/// Extrai o snapshot de telas através do `xrandr --query`.
pub async fn fetch_display_snapshot() -> anyhow::Result<DisplaySnapshot> {
    let output = tokio::process::Command::new("xrandr")
        .arg("--query")
        .output()
        .await?;

    if !output.status.success() {
        return Ok(DisplaySnapshot::default());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_xrandr_query(&stdout)
}

/// Parser puro da saída de `xrandr --query`.
pub fn parse_xrandr_query(output: &str) -> anyhow::Result<DisplaySnapshot> {
    let mut displays = Vec::new();
    let mut current_display: Option<DisplayNode> = None;
    let mut primary_name = None;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Screen ") || trimmed.is_empty() {
            continue;
        }

        // Nova linha de saída: ex: "eDP-1 connected primary 1366x768+0+0 ..." ou "HDMI-1 connected (normal..."
        if !line.starts_with("   ") && !line.starts_with('\t') && (line.contains("connected") || line.contains("disconnected")) {
            if let Some(d) = current_display.take() {
                if d.is_primary {
                    primary_name = Some(d.name.clone());
                }
                displays.push(d);
            }

            if let Some(node) = parse_display_header_line(line) {
                current_display = Some(node);
            }
        } else if let Some(d) = &mut current_display {
            // Linha de modo de resolução (ex: "   1920x1080     60.00*+  59.94")
            if let Some(mode) = parse_display_mode_line(line) {
                if mode.is_current {
                    d.current_mode = Some(mode.clone());
                }
                d.supported_modes.push(mode);
            }
        }
    }

    if let Some(d) = current_display {
        if d.is_primary {
            primary_name = Some(d.name.clone());
        }
        displays.push(d);
    }

    let connected_count = displays.iter().filter(|d| d.is_connected).count();
    let current_layout = infer_layout(&displays);

    Ok(DisplaySnapshot {
        displays,
        primary_name,
        connected_count,
        current_layout,
    })
}

fn parse_display_header_line(line: &str) -> Option<DisplayNode> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.len() < 2 {
        return None;
    }

    let name = tokens[0].to_string();
    let is_connected = tokens[1] == "connected";
    let is_primary = line.contains("primary");
    let is_internal = name.starts_with("eDP") || name.starts_with("LVDS") || name.starts_with("DSI");

    let mut pos_x = 0;
    let mut pos_y = 0;
    let mut is_active = false;
    let mut rotation = "normal".to_string();

    // Extrai geometria (ex: "1366x768+0+0")
    for token in &tokens[2..] {
        if token.contains('x') && token.contains('+') {
            if let Some((_, pos_part)) = token.split_once('+') {
                if let Some((x_str, y_str)) = pos_part.split_once('+') {
                    pos_x = x_str.parse().unwrap_or(0);
                    pos_y = y_str.parse().unwrap_or(0);
                    is_active = true;
                }
            }
        }
        if *token == "left" || *token == "right" || *token == "inverted" {
            rotation = token.to_string();
        }
    }

    Some(DisplayNode {
        name,
        is_connected,
        is_primary,
        is_active,
        current_mode: None,
        supported_modes: Vec::new(),
        pos_x,
        pos_y,
        rotation,
        is_internal,
    })
}

fn parse_display_mode_line(line: &str) -> Option<DisplayMode> {
    let tokens: Vec<&str> = line.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }

    let res_tok = tokens[0];
    let (w_str, h_str) = res_tok.split_once('x')?;
    let width: u32 = w_str.parse().ok()?;
    let height: u32 = h_str.parse().ok()?;

    let mut rate = 60.0;
    let mut is_current = false;
    let mut is_preferred = false;

    if tokens.len() > 1 {
        let rate_str = tokens[1];
        if rate_str.contains('*') {
            is_current = true;
        }
        if rate_str.contains('+') {
            is_preferred = true;
        }
        let clean_rate: String = rate_str.chars().filter(|c| c.is_ascii_digit() || *c == '.').collect();
        if let Ok(r) = clean_rate.parse::<f32>() {
            rate = r;
        }
    }

    Some(DisplayMode {
        width,
        height,
        rate,
        is_current,
        is_preferred,
    })
}

fn infer_layout(displays: &[DisplayNode]) -> Option<DisplayLayoutMode> {
    let connected: Vec<&DisplayNode> = displays.iter().filter(|d| d.is_connected && d.is_active).collect();
    if connected.len() < 2 {
        if let Some(d) = connected.first() {
            return if d.is_internal {
                Some(DisplayLayoutMode::InternalOnly)
            } else {
                Some(DisplayLayoutMode::ExternalOnly)
            };
        }
        return None;
    }

    let d1 = connected[0];
    let d2 = connected[1];

    if d1.pos_x == d2.pos_x && d1.pos_y == d2.pos_y {
        return Some(DisplayLayoutMode::Mirror);
    }
    if d1.is_internal {
        if d2.pos_x > d1.pos_x {
            Some(DisplayLayoutMode::ExtendRight)
        } else {
            Some(DisplayLayoutMode::ExtendLeft)
        }
    } else if d1.pos_x > d2.pos_x {
        Some(DisplayLayoutMode::ExtendRight)
    } else {
        Some(DisplayLayoutMode::ExtendLeft)
    }
}

/// Aplica um layout de monitores usando `xrandr`.
pub async fn apply_layout_xrandr(
    layout: DisplayLayoutMode,
    internal: &str,
    external: &str,
) -> anyhow::Result<()> {
    match layout {
        DisplayLayoutMode::ExtendRight => {
            let _ = tokio::process::Command::new("xrandr")
                .args([
                    "--output", internal, "--auto", "--primary",
                    "--output", external, "--auto", "--right-of", internal,
                ])
                .output().await;
        }
        DisplayLayoutMode::ExtendLeft => {
            let _ = tokio::process::Command::new("xrandr")
                .args([
                    "--output", internal, "--auto", "--primary",
                    "--output", external, "--auto", "--left-of", internal,
                ])
                .output().await;
        }
        DisplayLayoutMode::Mirror => {
            let _ = tokio::process::Command::new("xrandr")
                .args([
                    "--output", internal, "--auto", "--primary",
                    "--output", external, "--auto", "--same-as", internal,
                ])
                .output().await;
        }
        DisplayLayoutMode::ExternalOnly => {
            let _ = tokio::process::Command::new("xrandr")
                .args([
                    "--output", external, "--auto", "--primary",
                    "--output", internal, "--off",
                ])
                .output().await;
        }
        DisplayLayoutMode::InternalOnly => {
            let _ = tokio::process::Command::new("xrandr")
                .args([
                    "--output", internal, "--auto", "--primary",
                    "--output", external, "--off",
                ])
                .output().await;
        }
        DisplayLayoutMode::Custom => {}
    }
    Ok(())
}
