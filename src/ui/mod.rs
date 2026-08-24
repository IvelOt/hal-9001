//! Camada de render. `draw` é uma função pura de `&App`.

pub mod theme;
pub mod widgets;

pub mod audio;
pub mod bluetooth;
pub mod config_modal;
pub mod display;
pub mod file_picker;
pub mod files;
pub mod network;
pub mod overview;
pub mod power;
pub mod splash;
pub mod storage;
pub mod terminal;
pub mod updates;

use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Tabs};
use ratatui::Frame;

use crate::app::{App, Phase, Tab};
use crate::events::ToastLevel;
use theme::Palette;

/// Ponto de entrada do render.
pub fn draw(app: &App, f: &mut Frame) {
    let pal = Palette::from_config(&app.config);

    if app.phase == Phase::Splash {
        splash::draw(app, &pal, f, f.area());
        return;
    }

    let chunks = Layout::vertical([
        Constraint::Length(3), // tabbar
        Constraint::Min(0),    // conteúdo
        Constraint::Length(1), // statusline
    ])
    .split(f.area());

    draw_tabbar(app, &pal, f, chunks[0]);
    draw_content(app, &pal, f, chunks[1]);
    draw_statusline(app, &pal, f, chunks[2]);

    if app.active == Tab::Storage && app.storage_modal_open() {
        storage::draw_modal(app, &pal, f);
    }
    if app.show_help {
        draw_help(app, &pal, f);
    }
    if app.show_config {
        config_modal::draw(app, &pal, f);
    }
    // Prioridade máxima de render: o modal nativo de senha de sudo aparece
    // por cima de qualquer outro modal (ex.: instalação do Ventoy em curso),
    // em qualquer aba.
    if let Some(sudo_prompt) = &app.sudo_prompt {
        storage::draw_sudo_prompt(app, &pal, f, sudo_prompt);
    }
}

fn draw_tabbar(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .enumerate()
        .map(|(i, t)| {
            Line::from(vec![
                Span::styled(format!("{} ", i + 1), Style::default().fg(pal.dim)),
                Span::raw(t.title_in(app.lang)),
            ])
        })
        .collect();

    // Título da janela: nome do assistente + SO/arch quando já houver snapshot.
    // Ex.: `HAL-9001 · Assistente de Sistema (Arch Linux x86_64)`.
    let title = match &app.system {
        Some(s) => {
            let plat = match &s.detail.cpu_arch {
                Some(arch) => format!("{} {arch}", s.os),
                None => s.os.clone(),
            };
            format!(" HAL-9001 · {} ({plat}) ", m.app_title_suffix)
        }
        None => format!(" HAL-9001 · {} ", m.app_title_suffix),
    };

    let tabs = Tabs::new(titles)
        .select(app.active.index())
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(pal.dim))
                .title(Span::styled(
                    title,
                    Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                )),
        )
        .style(Style::default().fg(pal.fg))
        .highlight_style(
            Style::default()
                .fg(pal.accent)
                .add_modifier(Modifier::BOLD | Modifier::REVERSED),
        )
        .divider(Span::styled("│", Style::default().fg(pal.dim)));

    f.render_widget(tabs, area);
}

fn draw_content(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    match app.active {
        Tab::Overview => overview::draw(app, pal, f, area),
        Tab::Network => network::draw(app, pal, f, area),
        Tab::Bluetooth => bluetooth::draw(app, pal, f, area),
        Tab::Storage => storage::draw(app, pal, f, area),
        Tab::Audio => audio::draw(app, pal, f, area),
        Tab::Displays => display::draw(app, pal, f, area),
        Tab::Files => files::draw(app, pal, f, area),
        Tab::Terminal => terminal::draw(app, pal, f, area),
    }
}

fn draw_statusline(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    // Toast tem prioridade sobre os hints de atalho.
    if let Some((toast, _)) = &app.toast {
        let color = match toast.level {
            ToastLevel::Info => pal.accent,
            ToastLevel::Success => pal.ok,
            ToastLevel::Warning => pal.warn,
            ToastLevel::Error => pal.err,
        };
        let line = Line::from(vec![
            Span::styled(" ▎", Style::default().fg(color)),
            Span::styled(toast.text.clone(), Style::default().fg(color)),
        ]);
        f.render_widget(Paragraph::new(line), area);
        return;
    }

    let hints =
        " [1-8/Tab] abas   [j/k] navegar   [Enter] ação   [r] refresh   [?] ajuda   [q] sair ";
    let line = Line::from(Span::styled(hints, Style::default().fg(pal.dim)));
    f.render_widget(Paragraph::new(line), area);
}

fn draw_help(app: &App, pal: &Palette, f: &mut Frame) {
    let area = centered(60, 40, f.area());
    f.render_widget(Clear, area);

    let text = vec![
        Line::from(Span::styled(
            "HAL-9001 — Ajuda",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from("1..8 / Tab / Shift-Tab   trocar de aba"),
        Line::from("j / k / ↑ / ↓            navegar listas"),
        Line::from("Enter                   ação primária do item"),
        Line::from("r                       refresh / rescan"),
        Line::from(".                       Overview: detalhes normal/expandido"),
        Line::from("c / F2                  abrir configurações / settings"),
        Line::from("?                       abrir/fechar esta ajuda"),
        Line::from("q / Ctrl-c              sair"),
        Line::from(""),
        Line::from(Span::styled(
            format!("Aba ativa: {}", app.active.title()),
            Style::default().fg(pal.dim),
        )),
    ];

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.accent))
        .title(" ? ");
    f.render_widget(
        Paragraph::new(text).block(block).alignment(Alignment::Left),
        area,
    );
}

/// Retângulo centralizado com `pw`/`ph` por cento da área.
fn centered(pw: u16, ph: u16, area: Rect) -> Rect {
    let v = Layout::vertical([
        Constraint::Percentage((100 - ph) / 2),
        Constraint::Percentage(ph),
        Constraint::Percentage((100 - ph) / 2),
    ])
    .split(area);
    Layout::horizontal([
        Constraint::Percentage((100 - pw) / 2),
        Constraint::Percentage(pw),
        Constraint::Percentage((100 - pw) / 2),
    ])
    .split(v[1])[1]
}

/// Painel padrão para abas cujo backend ainda é um stub (Módulos 2..8).
/// Mostra o estado do serviço e as ações previstas.
pub(crate) fn draw_pending(
    app: &App,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
    title: &str,
    service_key: &'static str,
    actions: &[&str],
) {
    let block = content_block(title, pal);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut lines: Vec<Line> = Vec::new();
    match app
        .services
        .get(service_key)
        .and_then(|s| s.degraded.clone())
    {
        Some(reason) => lines.push(Line::from(vec![
            Span::styled("● ", Style::default().fg(pal.warn)),
            Span::styled(reason, Style::default().fg(pal.warn)),
        ])),
        None => lines.push(Line::from(Span::styled(
            "● inicializando serviço…",
            Style::default().fg(pal.dim),
        ))),
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Ações previstas:",
        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
    )));
    for a in actions {
        lines.push(Line::from(vec![
            Span::styled("  · ", Style::default().fg(pal.dim)),
            Span::styled((*a).to_string(), Style::default().fg(pal.fg)),
        ]));
    }

    f.render_widget(Paragraph::new(lines), inner);
}

/// Cols x rows disponíveis para a grade VT100 dentro da `area` de conteúdo
/// de uma aba PTY (Arquivos/Terminal), depois de descontar o chrome fixo que
/// `ui::terminal`/`ui::files` usam para o próprio layout: uma linha de
/// cabeçalho, uma linha de rodapé, e as bordas do bloco que envolve a grade.
/// Única fonte de verdade dessa conta — usada tanto pelo render quanto por
/// `App::sync_pty_size` (via [`pty_grid_size_for_terminal`]), para que o
/// tamanho da grade renderizada nunca fique fora de sincronia com o
/// `PtySize` real da sessão.
pub(crate) fn pty_grid_size(area: Rect) -> (u16, u16) {
    let cols = area.width.saturating_sub(2); // bordas esquerda/direita do bloco
    let rows = area.height.saturating_sub(4); // header(1) + footer(1) + bordas topo/baixo(2)
    (cols.max(1), rows.max(1))
}

/// Mesma conta de [`pty_grid_size`], mas a partir do tamanho bruto do
/// terminal (`term_w`x`term_h`), replicando o layout vertical de `ui::draw`
/// (tabbar `Length(3)` + statusline `Length(1)`) para chegar à mesma `area`
/// de conteúdo que `draw_content` passa às abas Arquivos/Terminal.
pub(crate) fn pty_grid_size_for_terminal(term_w: u16, term_h: u16) -> (u16, u16) {
    let content_area = Rect {
        x: 0,
        y: 0,
        width: term_w,
        height: term_h.saturating_sub(4), // tabbar(3) + statusline(1)
    };
    pty_grid_size(content_area)
}

/// Bloco padrão de conteúdo com título de aba.
pub(crate) fn content_block(title: &str, pal: &Palette) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(pal.dim))
        .title(Span::styled(
            format!(" {title} "),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ))
}
