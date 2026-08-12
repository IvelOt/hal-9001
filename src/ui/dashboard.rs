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
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Tabs, Widget, Wrap};
use ratatui::{Frame, Terminal};

use crate::ai_agent::ipc_server::Gatekeeper;
use crate::ai_agent::pty_session::PtyTarget;
use crate::ai_agent::widget::AiDeckState;
use crate::config::{load as load_config, Config, HostInfo};
use crate::events::SystemSnapshot;
use crate::ui::file_manager::FileManagerState;
use crate::ui::toast::{Toast, ToastBar};
use crate::ui::{accent_color, ACCENT, BG, DANGER, DIM, GRAY, TEXT, WARN};

/// Abas do dashboard, na ordem exibida pela barra de abas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Home = 0,
    Storage = 1,
    Network = 2,
    Bluetooth = 3,
    Files = 4,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Home,
        Tab::Storage,
        Tab::Network,
        Tab::Bluetooth,
        Tab::Files,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            Tab::Home => "Home",
            Tab::Storage => "Discos / USB",
            Tab::Network => "Rede / Wi-Fi",
            Tab::Bluetooth => "Bluetooth",
            Tab::Files => "Arquivos",
        }
    }

    pub fn from_index(index: usize) -> Self {
        Self::ALL.get(index).copied().unwrap_or(Self::Home)
    }

    pub fn next(self) -> Self {
        Self::from_index(self.index() + 1)
    }

    pub fn prev(self) -> Self {
        if self.index() == 0 {
            Self::Files
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
    /// Pilha de notificações toast a renderizar no canto inferior direito.
    pub toasts: Vec<Toast>,
    /// Configuração da Home estilo Fastfetch (logo, acentos, métricas).
    pub config: Config,
    /// Dados de sistema lidos para a Home (OS/host/kernel/cpu/shell).
    pub host: HostInfo,
    /// Estado do navegador de arquivos.
    pub files: FileManagerState,
}

impl Default for Dashboard {
    fn default() -> Self {
        Self::new()
    }
}

impl Dashboard {
    pub fn new() -> Self {
        Self {
            tab: Tab::Home,
            snapshot: None,
            deck: AiDeckState::default(),
            gatekeeper: None,
            message: None,
            storage_index: 0,
            network_index: 0,
            bluetooth_index: 0,
            toasts: Vec::new(),
            config: load_config(),
            host: crate::config::collect_host_info(),
            files: FileManagerState::load(),
        }
    }

    /// Registra uma nova notificação toast (auto-dispensa em 4s).
    pub fn push_toast(&mut self, toast: Toast) {
        self.toasts.push(toast);
        ToastBar::prune(&mut self.toasts);
    }

    /// Remove as notificações já vencidas.
    pub fn prune_toasts(&mut self) {
        ToastBar::prune(&mut self.toasts);
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
        // Toasts por cima de tudo, no canto inferior direito.
        let bar = ToastBar::new(self.toasts.clone());
        frame.render_widget(bar, area);
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
            Tab::Home => self.render_home(area, frame.buffer_mut()),
            Tab::Storage => self.render_storage(area, frame.buffer_mut()),
            Tab::Network => self.render_network(area, frame.buffer_mut()),
            Tab::Bluetooth => self.render_bluetooth(area, frame.buffer_mut()),
            Tab::Files => {
                frame.render_widget(YaziPrompt::new(), area);
            }
        }
    }

    /// [1] Home — estilo Fastfetch: logo ASCII à esquerda, métricas à direita.
    fn render_home(&self, area: Rect, buf: &mut Buffer) {
        if area.width < 12 || area.height < 6 {
            return;
        }
        let accent = accent_color(self.config.accent);

        // Título centralizado.
        let title_width = self.config.title.chars().count() as u16;
        let title_x = area.x + (area.width.saturating_sub(title_width) / 2).min(area.width.saturating_sub(1));
        buf.set_stringn(
            title_x,
            area.y,
            &self.config.title,
            area.width as usize,
            Style::new().fg(accent).add_modifier(Modifier::BOLD),
        );

        let body = Layout::horizontal([
            Constraint::Percentage(45),
            Constraint::Percentage(55),
        ])
        .split(Rect::new(area.x, area.y + 2, area.width, area.height.saturating_sub(2)));

        self.render_home_logo(body[0], buf, accent);
        self.render_home_metrics(body[1], buf, accent);
    }

    /// Coluna esquerda: logo ASCII colorido.
    fn render_home_logo(&self, area: Rect, buf: &mut Buffer, accent: Color) {
        let logo = &self.config.logo;
        let logo_lines: Vec<&str> = logo.lines().collect();
        let start_y = if logo_lines.len() as u16 >= area.height {
            area.y
        } else {
            area.y + (area.height.saturating_sub(logo_lines.len() as u16) / 2)
        };
        let style = Style::new().fg(accent);
        for (i, line) in logo_lines.iter().enumerate() {
            let y = start_y + i as u16;
            if y >= area.y + area.height {
                break;
            }
            let content = line.trim_end();
            if content.is_empty() {
                continue;
            }
            buf.set_stringn(area.x, y, content, area.width as usize, style);
        }
    }

    /// Coluna direita: métricas de sistema no estilo Fastfetch.
    fn render_home_metrics(&self, area: Rect, buf: &mut Buffer, accent: Color) {
        let snapshot = self.snapshot.clone().unwrap_or_default();
        let m = &self.config.metrics;
        let host = &self.host;
        let user_env = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
        let hostname = host.host.clone();

        // Cabeçalho `user@host` com destaque.
        let header = format!("{user_env}@{hostname}");
        buf.set_stringn(
            area.x,
            area.y,
            &header,
            area.width as usize,
            Style::new().fg(accent).add_modifier(Modifier::BOLD),
        );
        let mut y = area.y + 2;

        let mut metrics: Vec<(String, String)> = Vec::new();

        if m.os {
            metrics.push(("OS".to_string(), host.os.clone()));
        }
        if m.host {
            metrics.push(("Host".to_string(), hostname.clone()));
        }
        if m.kernel {
            metrics.push(("Kernel".to_string(), host.kernel.clone()));
        }
        if m.uptime {
            metrics.push((
                "Uptime".to_string(),
                human_duration(Duration::from_secs(snapshot.system.uptime_secs)),
            ));
        }
        if m.cpu {
            metrics.push(("CPU".to_string(), host.cpu.clone()));
        }
        if m.ram {
            let total = snapshot.system.mem_total_kb;
            let used = snapshot.system.mem_used_kb();
            metrics.push((
                "RAM".to_string(),
                format!("{} / {}", human_kib(used), human_kib(total)),
            ));
        }
        if m.disks {
            let mounted = snapshot.storage.iter().filter(|d| d.mounted).count();
            metrics.push((
                "Discos".to_string(),
                format!("{} mídias, {} montadas", snapshot.storage.len(), mounted),
            ));
        }
        if m.battery {
            let battery_text = match &snapshot.primary_battery {
                Some(bat) => format!("{:.0}%", bat.percentage),
                None => "—".to_string(),
            };
            metrics.push(("Bateria".to_string(), battery_text));
        }
        if m.shell {
            metrics.push(("Shell".to_string(), shell_name(&host.shell)));
        }

        let max_key = metrics.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0);
        for (key, value) in metrics {
            if y >= area.y + area.height {
                break;
            }
            let key_span = Span::styled(
                format!("{}", pad(key, max_key)),
                Style::new().fg(accent),
            );
            let value_span = Span::styled(value, Style::new().fg(TEXT));
            let line = Line::from(vec![key_span, Span::styled("  ", Style::new()), value_span]);
            self.render_home_line(line, area.x, y, buf);
            y += 1;
        }
    }

    /// Escreve uma linha respeitando os limites da área.
    fn render_home_line(&self, line: Line<'static>, x: u16, y: u16, buf: &mut Buffer) {
        let mut cx = x;
        for span in &line.spans {
            if span.content.is_empty() {
                continue;
            }
            let width = span.content.chars().count() as u16;
            buf.set_stringn(cx, y, &span.content, width as usize, span.style);
            cx = cx.saturating_add(width);
        }
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
                    "  {:<8}  {:<18}  {:<26}  {:>10}",
                    marker,
                    truncate(&device.device, 18),
                    truncate(&label, 26),
                    human_bytes(device.size)
                ),
                style,
            )));
        }
        if snapshot.storage.is_empty() {
            lines.push(Line::from(Span::styled(
                "  (UDisks2 indisponível ou nenhum dispositivo de bloco encontrado)",
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
            Tab::Home => "[1-5] aba · [q] sair",
            Tab::Storage => "[↑/↓] navegar · [m] montar/desmontar · [1-5] aba · [q] sair",
            Tab::Network => "[↑/↓] navegar · [Enter/c] conectar · [d] desconectar · [1-5] aba · [q] sair",
            Tab::Bluetooth => "[↑/↓] navegar · [b] conectar/desconectar · [1-5] aba · [q] sair",
            Tab::Files => "[Enter/f] abrir Yazi · [1-5] aba · [q] sair",
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

/// Tela da aba **Arquivos**: convida o usuário a abrir o Yazi File Manager.
///
/// A execução nativa do Yazi é feita no loop principal (`src/main.rs`), que
/// suspende o raw mode do Ratatui, roda o subprocesso e restaura a TUI ao sair.
pub struct YaziPrompt;

impl YaziPrompt {
    pub fn new() -> Self {
        Self
    }
}

impl Widget for YaziPrompt {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 24 || area.height < 8 {
            return;
        }
        let block = block(" ARQUIVOS · YAZI FILE MANAGER ");
        let inner = block.inner(area);
        block.render(area, buf);

        let lines: Vec<Line<'static>> = vec![
            Line::from(""),
            Line::from(Span::styled(
                "        ██   ██   ██████   ███████   ██    ██",
                Style::new().fg(ACCENT),
            )),
            Line::from(Span::styled(
                "        ██  ██   ██   ██  ██        ███   ███",
                Style::new().fg(ACCENT),
            )),
            Line::from(Span::styled(
                "        █████    ██   ██  ███████   █████████",
                Style::new().fg(ACCENT),
            )),
            Line::from(Span::styled(
                "        ██  ██   ██   ██       ██  ██  ██  ██",
                Style::new().fg(ACCENT),
            )),
            Line::from(Span::styled(
                "        ██   ██  ██████   ███████  ██      ██",
                Style::new().fg(ACCENT),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "  Pressione [f] ou [Enter] para abrir o Yazi File Manager.",
                Style::new().fg(TEXT),
            )),
            Line::from(Span::styled(
                "  [1-5] troca de aba · [q] sair",
                Style::new().fg(GRAY),
            )),
        ];

        let start_y = inner.y + (inner.height.saturating_sub(lines.len() as u16) / 2);
        for (i, line) in lines.iter().enumerate() {
            let y = start_y + i as u16;
            if y >= inner.y + inner.height {
                break;
            }
            let mut x = inner.x;
            for span in &line.spans {
                if x >= inner.x + inner.width {
                    break;
                }
                let style = Style::new().fg(TEXT).patch(span.style);
                buf.set_stringn(
                    x,
                    y,
                    &span.content,
                    (inner.x + inner.width - x) as usize,
                    style,
                );
                x = x.saturating_add(span.width() as u16);
            }
        }
    }
}

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

/// Preenche uma string com espaços à direita até `width` caracteres.
fn pad(value: String, width: usize) -> String {
    if value.chars().count() >= width {
        return value;
    }
    let mut result = value;
    let padding = width.saturating_sub(result.chars().count());
    result.push_str(&" ".repeat(padding));
    result
}

/// Extrai o nome do shell (basename) a partir do caminho `$SHELL`.
fn shell_name(shell: &str) -> String {
    shell.trim_end_matches('/').rsplit('/').next().unwrap_or(shell).to_string()
}
