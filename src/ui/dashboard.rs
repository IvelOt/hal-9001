//! Dashboard TUI multi-abas do HAL-9001.
//!
//! Abas: **Overview**, **Discos/USB**, **Rede/Wi-Fi**, **Bluetooth** e
//! **AI Terminal Deck**. Inclui o **Gatekeeper Consent Modal** centralizado
//! (aprovação `[y]`/`[n]`) e a estética Retro Terminal Minimalista definida em
//! [`crate::ui`].
//!
//! Conforme seção 2 e 3 de `docs/backend_architecture.md`.

use std::sync::Arc;
use std::time::Duration;

use ratatui::backend::CrosstermBackend;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Tabs, Widget, Wrap};
use ratatui::{Frame, Terminal};

use crate::ai_agent::ipc_server::Gatekeeper;
use crate::ai_agent::pty_session::PtyTarget;
use crate::ai_agent::widget::{AiDeckState, AiDeckWidget};
use crate::events::SystemSnapshot;
use crate::ui::{ACCENT, BG, CYAN, DANGER, DIM, GRAY, TEXT, WARN};

/// Abas do dashboard, na ordem exibida pela barra de abas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Overview = 0,
    Storage = 1,
    Network = 2,
    Bluetooth = 3,
    AiDeck = 4,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Overview,
        Tab::Storage,
        Tab::Network,
        Tab::Bluetooth,
        Tab::AiDeck,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            Tab::Overview => "Overview",
            Tab::Storage => "Discos / USB",
            Tab::Network => "Rede / Wi-Fi",
            Tab::Bluetooth => "Bluetooth",
            Tab::AiDeck => "AI Terminal Deck",
        }
    }

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Self::Overview)
    }

    pub fn next(self) -> Self {
        Self::from_index(self.index() + 1)
    }

    pub fn prev(self) -> Self {
        if self.index() == 0 {
            Self::AiDeck
        } else {
            Self::from_index(self.index() - 1)
        }
    }
}

/// Estado completo do dashboard renderizado pelo loop principal.
pub struct Dashboard {
    pub tab: Tab,
    pub snapshot: Option<Arc<SystemSnapshot>>,
    pub deck: AiDeckState,
    pub gatekeeper: Option<Gatekeeper>,
    pub message: Option<String>,
    pub storage_index: usize,
    pub network_index: usize,
    pub bluetooth_index: usize,
}

impl Default for Dashboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Dashboard {
    pub fn new() -> Self {
        Self {
            tab: Tab::Overview,
            snapshot: None,
            deck: AiDeckState::default(),
            gatekeeper: None,
            message: None,
            storage_index: 0,
            network_index: 0,
            bluetooth_index: 0,
        }
    }

    // ------------------------------------------------------------------
    // Navegação e seleção
    // ------------------------------------------------------------------

    pub fn select_tab(&mut self, tab: Tab) {
        self.tab = tab;
    }

    pub fn next_tab(&mut self) {
        self.tab = self.tab.next();
    }

    pub fn prev_tab(&mut self) {
        self.tab = self.tab.prev();
    }

    /// Move a seleção da lista da aba atual por `delta` (-1 / +1).
    pub fn move_selection(&mut self, delta: isize) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        let idx = match self.tab {
            Tab::Storage => &mut self.storage_index,
            Tab::Network => &mut self.network_index,
            Tab::Bluetooth => &mut self.bluetooth_index,
            _ => return,
        };
        *idx = (*idx as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    /// Redimensiona o PTY do AI Terminal Deck para o tamanho informado.
    pub fn on_resize(&self, cols: u16, rows: u16) {
        if let Some(session) = &self.deck.session {
            let _ = session.resize(rows.saturating_sub(4), cols.saturating_sub(2));
        }
    }

    /// Desenha o dashboard completo num frame do Ratatui.
    pub fn render_frame(&self, frame: &mut Frame) {
        let area = frame.area();
        let chunks = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(area);

        self.render_tab_bar(chunks[0], frame.buffer_mut());
        self.render_body(chunks[1], frame);
        self.render_status_bar(chunks[2], frame.buffer_mut());
        self.render_consent_modal(frame);
    }

    /// Conveniência para desenhar num `Terminal` Crossterm.
    pub fn draw(&mut self, terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>) -> anyhow::Result<()> {
        terminal.draw(|frame| self.render_frame(frame))?;
        Ok(())
    }

    // ------------------------------------------------------------------
    // Barra de abas
    // ------------------------------------------------------------------

    fn render_tab_bar(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 2 || area.height < 2 {
            return;
        }

        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(DIM))
            .style(Style::new().bg(BG))
            .title(Line::from(Span::styled(
                " HAL-9001 ",
                Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
            )))
            .title_style(Style::new().fg(ACCENT));
        let inner = block.inner(area);
        block.render(area, buf);

        let titles: Vec<Line<'_>> = Tab::ALL
            .iter()
            .map(|tab| {
                Line::from(format!("{} {}", tab.index() + 1, tab.label()))
            })
            .collect();

        let tabs = Tabs::new(titles)
            .select(Some(self.tab.index()))
            .highlight_style(
                Style::new()
                    .fg(ACCENT)
                    .bg(DIM)
                    .add_modifier(Modifier::BOLD),
            )
            .style(Style::new().fg(GRAY))
            .divider("  ");

        tabs.render(inner, buf);
    }

    // ------------------------------------------------------------------
    // Corpo
    // ------------------------------------------------------------------

    fn render_body(&self, area: Rect, frame: &mut Frame) {
        match self.tab {
            Tab::Overview => self.render_overview(area, frame.buffer_mut()),
            Tab::Storage => self.render_storage(area, frame.buffer_mut()),
            Tab::Network => self.render_network(area, frame.buffer_mut()),
            Tab::Bluetooth => self.render_bluetooth(area, frame.buffer_mut()),
            Tab::AiDeck => {
                frame.render_widget(AiDeckWidget::new(&self.deck), area);
            }
        }
    }

    /// [1] Overview — resumo do sistema, energia, rede ativa e mídias.
    fn render_overview(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 4 {
            return;
        }
        let columns = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(area);
        let left = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(columns[0]);
        let right = Layout::vertical([Constraint::Percentage(50), Constraint::Percentage(50)]).split(columns[1]);

        self.render_system_panel(left[0], buf);
        self.render_energy_panel(left[1], buf);
        self.render_network_panel(right[0], buf);
        self.render_storage_panel(right[1], buf);
    }

    fn render_system_panel(&self, area: Rect, buf: &mut Buffer) {
        let snapshot = self.snapshot.clone().unwrap_or_default();
        let mut lines = Vec::new();
        lines.push(Line::from(Span::styled(
            format!("  carga (1min): {:.2}", snapshot.system.load1),
            Style::new().fg(TEXT),
        )));

        let mem_total = snapshot.system.mem_total_kb;
        let mem_used = snapshot.system.mem_used_kb();
        let mem_pct = if mem_total > 0 {
            (mem_used as f64 / mem_total as f64) * 100.0
        } else {
            0.0
        };
        lines.push(Line::from(Span::styled(
            format!(
                "  memória:     {} / {} ({:.0}%)",
                human_kib(mem_used),
                human_kib(mem_total),
                mem_pct
            ),
            Style::new().fg(TEXT),
        )));
        lines.push(Line::from(Span::styled(
            format!("               {}", bar(mem_pct, 16)),
            Style::new().fg(ACCENT),
        )));
        lines.push(Line::from(Span::styled(
            format!("  em operação: {}", human_duration(Duration::from_secs(snapshot.system.uptime_secs))),
            Style::new().fg(GRAY),
        )));

        let volume = snapshot
            .volume
            .map(|v| format!("{:.0}%", v * 100.0))
            .unwrap_or_else(|| "—".to_string());
        let brightness = snapshot
            .brightness
            .map(|b| format!("{b}%"))
            .unwrap_or_else(|| "—".to_string());
        lines.push(Line::from(vec![
            Span::styled("  volume:     ", Style::new().fg(GRAY)),
            Span::styled(volume, Style::new().fg(TEXT)),
            Span::styled("   brilho: ", Style::new().fg(GRAY)),
            Span::styled(brightness, Style::new().fg(TEXT)),
        ]));

        render_block_lines(area, buf, " SISTEMA ", lines);
    }

    fn render_energy_panel(&self, area: Rect, buf: &mut Buffer) {
        let snapshot = self.snapshot.clone().unwrap_or_default();
        let mut lines = Vec::new();

        let source = match snapshot.on_battery {
            Some(true) => Span::styled("em bateria", Style::new().fg(WARN)),
            Some(false) => Span::styled("na tomada", Style::new().fg(ACCENT)),
            None => Span::styled("indisponível", Style::new().fg(GRAY)),
        };
        lines.push(Line::from(vec![
            Span::styled("  fonte:      ", Style::new().fg(GRAY)),
            source,
        ]));

        if let Some(battery) = &snapshot.primary_battery {
            lines.push(Line::from(Span::styled(
                format!("  estado:     {}", power_state_label(battery.state)),
                Style::new().fg(TEXT),
            )));
            lines.push(Line::from(Span::styled(
                format!(
                    "  nível:      {:.0}%  [saúde {:.0}%]",
                    battery.percentage, battery.capacity
                ),
                Style::new().fg(TEXT),
            )));
            lines.push(Line::from(Span::styled(
                format!("               {}", bar(battery.percentage, 16)),
                Style::new().fg(CYAN),
            )));
            if let Some(remaining) = battery.estimated_time_remaining() {
                lines.push(Line::from(Span::styled(
                    format!("  restante:   {}", human_duration(remaining)),
                    Style::new().fg(GRAY),
                )));
            }
        } else if snapshot.on_battery.is_none() {
            lines.push(Line::from(Span::styled(
                "  (UPower indisponível)",
                Style::new().fg(GRAY),
            )));
        } else {
            lines.push(Line::from(Span::styled(
                "  nenhuma bateria presente",
                Style::new().fg(GRAY),
            )));
        }

        render_block_lines(area, buf, " ENERGIA / UPower ", lines);
    }

    fn render_network_panel(&self, area: Rect, buf: &mut Buffer) {
        let snapshot = self.snapshot.clone().unwrap_or_default();
        let mut lines = Vec::new();

        let enabled = match snapshot.network.wireless_enabled {
            Some(true) => Span::styled("ligado", Style::new().fg(ACCENT)),
            Some(false) => Span::styled("desligado", Style::new().fg(WARN)),
            None => Span::styled("indisponível", Style::new().fg(GRAY)),
        };
        lines.push(Line::from(vec![
            Span::styled("  wi-fi:      ", Style::new().fg(GRAY)),
            enabled,
        ]));

        match &snapshot.network.active {
            Some(wifi) => {
                lines.push(Line::from(Span::styled(
                    format!("  ssid:       {}", wifi.ssid),
                    Style::new().fg(TEXT),
                )));
                lines.push(Line::from(Span::styled(
                    format!("  sinal:      {}%", wifi.strength),
                    Style::new().fg(ACCENT),
                )));
                lines.push(Line::from(Span::styled(
                    format!("               {}", bar(wifi.strength as f64, 16)),
                    Style::new().fg(CYAN),
                )));
            }
            None => lines.push(Line::from(Span::styled(
                "  nenhuma rede sem fio ativa",
                Style::new().fg(GRAY),
            ))),
        }

        render_block_lines(area, buf, " REDE / Wi-Fi ", lines);
    }

    fn render_storage_panel(&self, area: Rect, buf: &mut Buffer) {
        let snapshot = self.snapshot.clone().unwrap_or_default();
        let mut lines = Vec::new();

        let mounted = snapshot.storage.iter().filter(|d| d.mounted).count();
        lines.push(Line::from(Span::styled(
            format!(
                "  {} mídia(s) · {} montada(s)",
                snapshot.storage.len(),
                mounted
            ),
            Style::new().fg(TEXT),
        )));

        for device in snapshot.storage.iter().take(4) {
            let marker = if device.mounted { "[M]" } else { "[·]" };
            let label = if device.label.is_empty() {
                "(sem rótulo)".to_string()
            } else {
                device.label.clone()
            };
            let style = if device.mounted {
                Style::new().fg(ACCENT)
            } else {
                Style::new().fg(TEXT)
            };
            lines.push(Line::from(Span::styled(
                format!("  {marker} {}  {}  {}", device.device, label, human_bytes(device.size)),
                style,
            )));
        }
        if snapshot.storage.is_empty() {
            lines.push(Line::from(Span::styled(
                "  nenhuma mídia removível",
                Style::new().fg(GRAY),
            )));
        }

        render_block_lines(area, buf, " ARMAZENAMENTO ", lines);
    }

    /// [2] Discos / USB — lista de partições UDisks2 com montar/desmontar `[m]`.
    fn render_storage(&self, area: Rect, buf: &mut Buffer) {
        let snapshot = self.snapshot.clone().unwrap_or_default();
        let mut lines = Vec::new();
        for device in &snapshot.storage {
            let label = if device.label.is_empty() {
                "(sem rótulo)".to_string()
            } else {
                device.label.clone()
            };
            let marker = if device.mounted { "MONTADO" } else { "-----" };
            let style = if device.mounted {
                Style::new().fg(ACCENT)
            } else {
                Style::new().fg(TEXT)
            };
            lines.push(Line::from(Span::styled(
                format!(
                    "  {marker}  {}  {:<22}  {}",
                    device.device,
                    truncate(&label, 22),
                    human_bytes(device.size)
                ),
                style,
            )));
        }
        if snapshot.storage.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (UDisks2 indisponível ou nenhuma mídia removível)",
                Style::new().fg(GRAY),
            )));
        }

        render_block_list(area, buf, " DISCOS / USB ", lines, self.selected_storage());
    }

    /// [3] Rede / Wi-Fi — lista de pontos de acesso com ativar/desativar `[w]`.
    fn render_network(&self, area: Rect, buf: &mut Buffer) {
        let snapshot = self.snapshot.clone().unwrap_or_default();
        let mut lines = Vec::new();
        for ap in &snapshot.network.access_points {
            let marker = if ap.is_active { "●" } else { "○" };
            let style = if ap.is_active {
                Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)
            } else {
                Style::new().fg(TEXT)
            };
            let ssid = if ap.ssid.is_empty() {
                "(rede oculta)".to_string()
            } else {
                ap.ssid.clone()
            };
            lines.push(Line::from(Span::styled(
                format!("  {marker}  {:<26}  {:>3}%", truncate(&ssid, 26), ap.strength),
                style,
            )));
        }
        if snapshot.network.access_points.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (NetworkManager indisponível ou nenhuma rede visível)",
                Style::new().fg(GRAY),
            )));
        }

        render_block_list(area, buf, " REDE / WI-FI ", lines, self.selected_network());
    }

    /// [4] Bluetooth — adaptadores e dispositivos BlueZ, conectar/desconectar `[b]`.
    fn render_bluetooth(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 2 || area.height < 2 {
            return;
        }
        let snapshot = self.snapshot.clone().unwrap_or_default();

        let chunks = Layout::vertical([Constraint::Length(3), Constraint::Min(1)]).split(area);
        let adapters_area = chunks[0];
        let devices_area = chunks[1];

        let adapters = snapshot
            .bluetooth
            .adapters
            .iter()
            .map(|a| truncate(a, 44))
            .collect::<Vec<_>>()
            .join(", ");
        let adapter_line = Line::from(vec![
            Span::styled("  ", Style::new().fg(GRAY)),
            Span::styled(
                if adapters.is_empty() {
                    "nenhum adaptador".to_string()
                } else {
                    adapters
                },
                Style::new().fg(TEXT),
            ),
        ]);
        render_block_lines(adapters_area, buf, " ADAPTADORES ", vec![adapter_line]);

        let mut device_lines = Vec::new();
        for device in &snapshot.bluetooth.devices {
            let name = if device.name.is_empty() {
                "(desconhecido)".to_string()
            } else {
                device.name.clone()
            };
            let status = if device.connected {
                "CONECTADO"
            } else if device.paired {
                "emparelhado"
            } else {
                "não emparelhado"
            };
            let style = if device.connected {
                Style::new().fg(ACCENT)
            } else {
                Style::new().fg(TEXT)
            };
            device_lines.push(Line::from(Span::styled(
                format!("  {:<26}  {status}", truncate(&name, 26)),
                style,
            )));
        }
        if snapshot.bluetooth.devices.is_empty() {
            device_lines.push(Line::from(Span::styled(
                "  (BlueZ indisponível ou nenhum dispositivo conhecido)",
                Style::new().fg(GRAY),
            )));
        }

        render_block_list(devices_area, buf, " DISPOSITIVOS ", device_lines, self.selected_bluetooth());
    }

    // ------------------------------------------------------------------
    // Barra de status
    // ------------------------------------------------------------------

    fn render_status_bar(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 4 || area.height < 1 {
            return;
        }

        let hints = self.hints();
        let left = format!("  HAL-9001 · {} · {}", self.tab.label(), hints);
        let message = self
            .message
            .as_deref()
            .map(|m| format!(" · {m}"))
            .unwrap_or_default();

        let pending = self
            .gatekeeper
            .as_ref()
            .map(|gk| gk.pending().len())
            .unwrap_or(0);
        let listening = if self.deck.ipc_listening { "ativo" } else { "inativo" };
        let uptime = self
            .snapshot
            .as_ref()
            .map(|s| human_duration(Duration::from_secs(s.system.uptime_secs)))
            .unwrap_or_else(|| "-".to_string());
        let right = format!(" IPC {listening} · consentimentos: {pending} · uptime {uptime} ");

        let style = Style::new().bg(BG).fg(GRAY);
        let full = format!("{left}{message}");
        buf.set_stringn(area.x, area.y, &full, area.width as usize, style);
        // Alinha a porção direita à direita, preservando a região esquerda.
        if right.len() < area.width as usize {
            let x = area.x + (area.width.saturating_sub(right.len() as u16));
            buf.set_stringn(x, area.y, &right, area.width as usize, style);
        }
    }

    fn hints(&self) -> &'static str {
        match self.tab {
            Tab::Overview => "[1-5] aba · [q] sair",
            Tab::Storage => "[↑/↓] navegar · [m] montar/desmontar · [1-5] aba · [q] sair",
            Tab::Network => "[↑/↓] navegar · [w] ligar/desligar Wi-Fi · [1-5] aba · [q] sair",
            Tab::Bluetooth => "[↑/↓] navegar · [b] conectar/desconectar · [1-5] aba · [q] sair",
            Tab::AiDeck => "[Tab/←/→] trocar aba · [Ctrl+Q] sair — teclas vão para o agente",
        }
    }

    // ------------------------------------------------------------------
    // Gatekeeper Consent Modal
    // ------------------------------------------------------------------

    fn render_consent_modal(&self, frame: &mut Frame) {
        let Some(gatekeeper) = &self.gatekeeper else {
            return;
        };
        let pending = gatekeeper.pending();
        let Some(request) = pending.first() else {
            return;
        };

        let area = frame.area();
        let modal_area = centered_rect(72, 40, area);

        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::new().fg(WARN))
            .style(Style::new().bg(BG))
            .title(Line::from(Span::styled(
                " GATEKEEPER — CONSENTIMENTO ",
                Style::new().fg(WARN).add_modifier(Modifier::BOLD),
            )))
            .title_style(Style::new().fg(WARN));

        frame.render_widget(Clear, modal_area);
        let inner = block.inner(modal_area);
        block.render(modal_area, frame.buffer_mut());
        let chunks = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(inner);

        // Método + agente solicitante.
        let method = Line::from(vec![
            Span::styled(" método: ", Style::new().fg(GRAY)),
            Span::styled(request.method.clone(), Style::new().fg(ACCENT)),
        ]);
        frame.render_widget(Paragraph::new(method), chunks[0]);

        // Descrição da ação solicitada.
        let description = Paragraph::new(Line::from(Span::styled(
            request.description.clone(),
            Style::new().fg(TEXT),
        )))
        .wrap(Wrap { trim: true })
        .style(Style::new().bg(BG));
        frame.render_widget(description, chunks[1]);

        // Decisão.
        let decision = Line::from(vec![
            Span::styled("  [y] Sim ", Style::new().fg(ACCENT).add_modifier(Modifier::BOLD)),
            Span::styled("   ", Style::new().fg(GRAY)),
            Span::styled("[n] Não ", Style::new().fg(DANGER).add_modifier(Modifier::BOLD)),
        ]);
        frame.render_widget(Paragraph::new(decision), chunks[2]);
    }

    // ------------------------------------------------------------------
    // Helpers de seleção
    // ------------------------------------------------------------------

    fn list_len(&self) -> usize {
        match self.tab {
            Tab::Storage => {
                self.snapshot.as_ref().map(|s| s.storage.len()).unwrap_or(0)
            }
            Tab::Network => {
                self.snapshot.as_ref().map(|s| s.network.access_points.len()).unwrap_or(0)
            }
            Tab::Bluetooth => {
                self.snapshot.as_ref().map(|s| s.bluetooth.devices.len()).unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn selected_storage(&self) -> Option<usize> {
        let len = self.snapshot.as_ref().map(|s| s.storage.len()).unwrap_or(0);
        if len == 0 { None } else { Some(self.storage_index.min(len - 1)) }
    }

    fn selected_network(&self) -> Option<usize> {
        let len = self.snapshot.as_ref().map(|s| s.network.access_points.len()).unwrap_or(0);
        if len == 0 { None } else { Some(self.network_index.min(len - 1)) }
    }

    fn selected_bluetooth(&self) -> Option<usize> {
        let len = self.snapshot.as_ref().map(|s| s.bluetooth.devices.len()).unwrap_or(0);
        if len == 0 { None } else { Some(self.bluetooth_index.min(len - 1)) }
    }
}

// ---------------------------------------------------------------------------
// Helpers de renderização
// ---------------------------------------------------------------------------

/// Bloco com bordas arredondadas e estilo Retro Terminal Minimalista.
fn block<'a>(title: &'a str) -> Block<'a> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(DIM))
        .style(Style::new().bg(BG))
        .title(Line::from(Span::styled(
            title.to_string(),
            Style::new().fg(ACCENT).add_modifier(Modifier::BOLD),
        )))
        .title_style(Style::new().fg(ACCENT))
}

/// Renderiza um bloco com linhas de texto (sem seleção), preservando estilos
/// de cada `Span` (ex.: valores destacados em acento).
fn render_block_lines(area: Rect, buf: &mut Buffer, title: &str, lines: Vec<Line<'static>>) {
    let block = block(title);
    let inner = block.inner(area);
    block.render(area, buf);
    for (i, line) in lines.iter().enumerate() {
        if i as u16 >= inner.height {
            break;
        }
        let y = inner.y + i as u16;
        let mut x = inner.x;
        for span in &line.spans {
            if x >= inner.x + inner.width {
                break;
            }
            let style = Style::new().fg(TEXT).patch(span.style);
            buf.set_stringn(x, y, &span.content, (inner.x + inner.width - x) as usize, style);
            x = x.saturating_add(span.width() as u16);
        }
    }
}

/// Renderiza um bloco com uma lista selecionável e rolagem embutida.
fn render_block_list(
    area: Rect,
    buf: &mut Buffer,
    title: &str,
    lines: Vec<Line<'static>>,
    selected: Option<usize>,
) {
    if area.width < 2 || area.height < 2 {
        return;
    }

    let block = block(title);
    let inner = block.inner(area);
    block.render(area, buf);

    if lines.is_empty() {
        buf.set_stringn(
            inner.x,
            inner.y,
            "  (vazio)",
            inner.width as usize,
            Style::new().fg(GRAY),
        );
        return;
    }

    let len = lines.len();
    let height = inner.height as usize;
    let start = match selected {
        Some(s) if s >= height => s - height + 1,
        _ => 0,
    };
    let start = start.min(len.saturating_sub(height));

    for (offset, index) in (start..len).enumerate().take(height) {
        let y = inner.y + offset as u16;
        if selected == Some(index) {
            let text: String = lines[index].spans.iter().map(|s| s.content.as_ref()).collect();
            buf.set_stringn(
                inner.x,
                y,
                &text,
                inner.width as usize,
                Style::new().fg(BG).bg(ACCENT).add_modifier(Modifier::BOLD),
            );
        } else {
            let mut x = inner.x;
            for span in &lines[index].spans {
                if x >= inner.x + inner.width {
                    break;
                }
                let style = Style::new().fg(TEXT).patch(span.style);
                buf.set_stringn(x, y, &span.content, (inner.x + inner.width - x) as usize, style);
                x = x.saturating_add(span.width() as u16);
            }
        }
    }
}

/// Área centralizada proporcionalmente dentro de `area`.
fn centered_rect(percent_x: u16, percent_y: u16, area: Rect) -> Rect {
    let vertical = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(vertical[1])[1]
}

/// Barra de progresso ASCII (16 células).
fn bar(percent: f64, width: usize) -> String {
    let width = width.max(2);
    let filled = ((percent.clamp(0.0, 100.0) / 100.0) * width as f64).round() as usize;
    let mut s = String::with_capacity(width + 2);
    s.push('[');
    for i in 0..width {
        s.push(if i < filled { '█' } else { '·' });
    }
    s.push(']');
    s
}

/// Rótulo PT-BR para um estado de energia UPower.
fn power_state_label(state: crate::backend::power::PowerState) -> &'static str {
    use crate::backend::power::PowerState;
    match state {
        PowerState::Charging => "carregando",
        PowerState::Discharging => "descarregando",
        PowerState::Empty => "vazia",
        PowerState::FullyCharged => "totalmente carregada",
        PowerState::PendingCharge => "carga pendente",
        PowerState::PendingDischarge => "descarga pendente",
        PowerState::Unknown => "desconhecido",
    }
}

/// Formata tamanho em bytes de forma legível (KiB, MiB, GiB...).
fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Formata um tamanho em KiB de forma legível.
fn human_kib(kib: u64) -> String {
    human_bytes(kib * 1024)
}

/// Formata uma duração em formato `Xd HH:MM:SS`.
fn human_duration(duration: Duration) -> String {
    let total = duration.as_secs();
    let days = total / 86_400;
    let hours = (total % 86_400) / 3_600;
    let minutes = (total % 3_600) / 60;
    let seconds = total % 60;
    if days > 0 {
        format!("{days}d {hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{hours:02}:{minutes:02}:{seconds:02}")
    }
}

/// Trunca uma string para no máximo `max` caracteres de largura, anexando `…`.
fn truncate(value: &str, max: usize) -> String {
    if value.chars().count() <= max {
        return value.to_string();
    }
    let mut end = max.saturating_sub(1);
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    let mut result: String = value[..end].to_string();
    result.push('…');
    result
}
