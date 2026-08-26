
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::events::{Action, AppEvent, EventTx};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioCategory {

    Sink,

    AppStream,

    Source,
}

impl AudioCategory {
    pub const ALL: [AudioCategory; 3] = [
        AudioCategory::Sink,
        AudioCategory::AppStream,
        AudioCategory::Source,
    ];

    pub fn title(&self) -> &'static str {
        match self {
            Self::Sink => "Saídas de Som",
            Self::AppStream => "Aplicativos",
            Self::Source => "Microfones",
        }
    }

    pub fn ascii_badge(&self) -> &'static str {
        match self {
            Self::Sink => "[SAID]",
            Self::AppStream => "[APP ]",
            Self::Source => "[MIC ]",
        }
    }

    pub fn nerd_glyph(&self) -> &'static str {
        match self {
            Self::Sink => "󰓃",
            Self::AppStream => "󰘔",
            Self::Source => "󰍬",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioNode {

    pub id: u32,

    pub name: String,

    pub description: String,

    pub category: AudioCategory,

    pub volume: f32,

    pub is_muted: bool,

    pub is_default: bool,

    pub icon_name: Option<String>,
}

impl AudioNode {

    pub fn volume_percent(&self) -> u32 {
        (self.volume * 100.0).round() as u32
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct AudioSnapshot {

    pub server_name: String,

    pub sinks: Vec<AudioNode>,

    pub apps: Vec<AudioNode>,

    pub sources: Vec<AudioNode>,

    pub default_sink_id: Option<u32>,

    pub default_source_id: Option<u32>,
}

impl AudioSnapshot {

    pub fn nodes_for_category(&self, cat: AudioCategory) -> &[AudioNode] {
        match cat {
            AudioCategory::Sink => &self.sinks,
            AudioCategory::AppStream => &self.apps,
            AudioCategory::Source => &self.sources,
        }
    }
}

pub async fn run(
    poll_interval_ms: u64,
    tx: EventTx,
    mut action_rx: broadcast::Receiver<Action>,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(poll_interval_ms.max(500)));

    loop {
        tokio::select! {
            _ = interval.tick() => {
                if let Ok(snap) = fetch_audio_snapshot().await {
                    let _ = tx.send(AppEvent::Audio(Box::new(snap)));
                }
            }

            Ok(action) = action_rx.recv() => {
                match action {
                    Action::AudioSetVolume { node_id, volume } => {
                        let _ = set_volume(node_id, volume).await;
                        if let Ok(snap) = fetch_audio_snapshot().await {
                            let _ = tx.send(AppEvent::Audio(Box::new(snap)));
                        }
                    }

                    Action::AudioVolumeUp(node_id, delta) => {
                        let _ = adjust_volume(node_id, delta).await;
                        if let Ok(snap) = fetch_audio_snapshot().await {
                            let _ = tx.send(AppEvent::Audio(Box::new(snap)));
                        }
                    }

                    Action::AudioVolumeDown(node_id, delta) => {
                        let _ = adjust_volume(node_id, -delta).await;
                        if let Ok(snap) = fetch_audio_snapshot().await {
                            let _ = tx.send(AppEvent::Audio(Box::new(snap)));
                        }
                    }

                    Action::AudioToggleMute(node_id) => {
                        let _ = toggle_mute(node_id).await;
                        if let Ok(snap) = fetch_audio_snapshot().await {
                            let _ = tx.send(AppEvent::Audio(Box::new(snap)));
                        }
                    }

                    Action::AudioSetDefault(node_id) => {
                        let _ = set_default_node(node_id).await;
                        if let Ok(snap) = fetch_audio_snapshot().await {
                            let _ = tx.send(AppEvent::Audio(Box::new(snap)));
                        }
                    }

                    _ => {}
                }
            }
        }
    }
}

pub async fn fetch_audio_snapshot() -> anyhow::Result<AudioSnapshot> {

    if let Ok(output) = tokio::process::Command::new("wpctl")
        .arg("status")
        .output()
        .await
    {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Ok(snap) = parse_wpctl_status(&stdout) {
                return Ok(snap);
            }
        }
    }

    if let Ok(output) = tokio::process::Command::new("pactl")
        .args(["list", "sinks"])
        .output()
        .await
    {
        if output.status.success() {
            let sinks_str = String::from_utf8_lossy(&output.stdout);
            let apps_str = tokio::process::Command::new("pactl")
                .args(["list", "sink-inputs"])
                .output()
                .await
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();
            let sources_str = tokio::process::Command::new("pactl")
                .args(["list", "sources"])
                .output()
                .await
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_default();

            return Ok(parse_pactl_output(&sinks_str, &apps_str, &sources_str));
        }
    }

    Ok(AudioSnapshot {
        server_name: "Áudio Indisponível".to_string(),
        sinks: Vec::new(),
        apps: Vec::new(),
        sources: Vec::new(),
        default_sink_id: None,
        default_source_id: None,
    })
}

pub fn parse_wpctl_status(output: &str) -> anyhow::Result<AudioSnapshot> {
    let mut sinks = Vec::new();
    let mut apps = Vec::new();
    let mut sources = Vec::new();
    let mut default_sink_id = None;
    let mut default_source_id = None;

    let mut current_section = Section::None;

    for line in output.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with("Audio") {
            continue;
        } else if trimmed.contains("Sinks:") {
            current_section = Section::Sinks;
            continue;
        } else if trimmed.contains("Sources:") {
            current_section = Section::Sources;
            continue;
        } else if trimmed.contains("Streams:") {
            current_section = Section::Streams;
            continue;
        } else if trimmed.starts_with("Video") || trimmed.starts_with("Settings") {
            current_section = Section::None;
            continue;
        } else if (trimmed.starts_with("├─") || trimmed.starts_with("└─")) && !trimmed.contains(':') {
            current_section = Section::None;
        }

        match current_section {
            Section::Sinks | Section::Sources | Section::Streams => {
                if let Some(node) = parse_wpctl_line(line, &current_section) {
                    if node.is_default {
                        match node.category {
                            AudioCategory::Sink => default_sink_id = Some(node.id),
                            AudioCategory::Source => default_source_id = Some(node.id),
                            _ => {}
                        }
                    }
                    match node.category {
                        AudioCategory::Sink => sinks.push(node),
                        AudioCategory::AppStream => apps.push(node),
                        AudioCategory::Source => sources.push(node),
                    }
                }
            }
            Section::None => {}
        }
    }

    Ok(AudioSnapshot {
        server_name: "PipeWire (WirePlumber)".to_string(),
        sinks,
        apps,
        sources,
        default_sink_id,
        default_source_id,
    })
}

fn parse_wpctl_line(line: &str, section: &Section) -> Option<AudioNode> {
    let clean = line.replace(['│', '└', '─', '├'], " ");
    let trimmed = clean.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.contains('>')
        || trimmed.contains("playback_")
        || trimmed.starts_with("output_")
        || trimmed.starts_with("input_")
    {
        return None;
    }

    let is_default = trimmed.starts_with('*');
    let without_star = if is_default {
        trimmed.trim_start_matches('*').trim()
    } else {
        trimmed
    };

    let (id_str, rest) = without_star.split_once('.')?;
    let id: u32 = id_str.trim().parse().ok()?;

    let (name_part, vol_part) = match rest.rsplit_once('[') {
        Some((n, v)) => (n.trim(), v.trim_end_matches(']').trim()),
        None => (rest.trim(), ""),
    };

    let mut volume: f32 = 1.0;
    let mut is_muted = false;

    if vol_part.to_uppercase().contains("MUTED") {
        is_muted = true;
    }

    if let Some((_, after_vol)) = vol_part.split_once("vol:") {
        if let Some(vol_tok) = after_vol.split_whitespace().next() {
            if let Ok(v) = vol_tok.parse::<f32>() {
                volume = v;
            }
        }
    }

    let category = match section {
        Section::Sinks => AudioCategory::Sink,
        Section::Streams => AudioCategory::AppStream,
        Section::Sources => AudioCategory::Source,
        Section::None => return None,
    };

    let name = clean_node_name(name_part);

    Some(AudioNode {
        id,
        name: name.clone(),
        description: name_part.to_string(),
        category,
        volume,
        is_muted,
        is_default,
        icon_name: None,
    })
}

fn clean_node_name(raw: &str) -> String {
    let s = raw.trim();
    if s.is_empty() {
        return "Dispositivo de Áudio".to_string();
    }
    s.to_string()
}

pub fn parse_pactl_output(sinks_str: &str, apps_str: &str, sources_str: &str) -> AudioSnapshot {
    let sinks = parse_pactl_nodes(sinks_str, AudioCategory::Sink);
    let apps = parse_pactl_nodes(apps_str, AudioCategory::AppStream);
    let sources = parse_pactl_nodes(sources_str, AudioCategory::Source);

    let default_sink_id = sinks.first().map(|s| s.id);
    let default_source_id = sources.first().map(|s| s.id);

    AudioSnapshot {
        server_name: "PulseAudio".to_string(),
        sinks,
        apps,
        sources,
        default_sink_id,
        default_source_id,
    }
}

fn parse_pactl_nodes(output: &str, category: AudioCategory) -> Vec<AudioNode> {
    let mut nodes = Vec::new();
    let mut current_id: Option<u32> = None;
    let mut current_name = String::new();
    let mut current_vol: f32 = 1.0;
    let mut current_muted = false;

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("Sink #") || trimmed.starts_with("Sink Input #") || trimmed.starts_with("Source #") {
            if let Some(id) = current_id {
                nodes.push(AudioNode {
                    id,
                    name: if current_name.is_empty() { format!("Audio Node {id}") } else { current_name.clone() },
                    description: current_name.clone(),
                    category,
                    volume: current_vol,
                    is_muted: current_muted,
                    is_default: false,
                    icon_name: None,
                });
            }
            if let Some(id_str) = trimmed.split('#').nth(1) {
                current_id = id_str.trim().parse().ok();
            }
            current_name.clear();
            current_vol = 1.0;
            current_muted = false;
        } else if trimmed.starts_with("Description:") || trimmed.starts_with("application.name =") {
            let val = trimmed.split_once(':').map(|x| x.1).or_else(|| trimmed.split_once('=').map(|x| x.1)).unwrap_or("");
            current_name = val.trim().trim_matches('"').to_string();
        } else if trimmed.starts_with("Mute:") {
            current_muted = trimmed.contains("yes");
        } else if trimmed.starts_with("Volume:") {
            if let Some(pct_str) = trimmed.split('/').nth(1) {
                if let Some(num_str) = pct_str.trim().strip_suffix('%') {
                    if let Ok(pct) = num_str.trim().parse::<f32>() {
                        current_vol = pct / 100.0;
                    }
                }
            }
        }
    }

    if let Some(id) = current_id {
        nodes.push(AudioNode {
            id,
            name: if current_name.is_empty() { format!("Audio Node {id}") } else { current_name },
            description: String::new(),
            category,
            volume: current_vol,
            is_muted: current_muted,
            is_default: false,
            icon_name: None,
        });
    }

    nodes
}

pub async fn set_volume(node_id: u32, volume: f32) -> anyhow::Result<()> {
    let vol_clamped = volume.clamp(0.0, 1.5);
    let _ = tokio::process::Command::new("wpctl")
        .args(["set-volume", &node_id.to_string(), &format!("{vol_clamped:.2}")])
        .output()
        .await;
    Ok(())
}

pub async fn adjust_volume(node_id: u32, delta: f32) -> anyhow::Result<()> {
    let sign = if delta >= 0.0 { "+" } else { "-" };
    let abs_pct = (delta.abs() * 100.0).round() as u32;
    let _ = tokio::process::Command::new("wpctl")
        .args(["set-volume", "--limit", "1.5", &node_id.to_string(), &format!("{abs_pct}%{sign}")])
        .output()
        .await;
    Ok(())
}

pub async fn toggle_mute(node_id: u32) -> anyhow::Result<()> {
    let _ = tokio::process::Command::new("wpctl")
        .args(["set-mute", &node_id.to_string(), "toggle"])
        .output()
        .await;
    Ok(())
}

pub async fn set_default_node(node_id: u32) -> anyhow::Result<()> {
    let _ = tokio::process::Command::new("wpctl")
        .args(["set-default", &node_id.to_string()])
        .output()
        .await;
    Ok(())
}

enum Section {
    None,
    Sinks,
    Sources,
    Streams,
}
