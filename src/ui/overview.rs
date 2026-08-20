//! Aba 1 — Overview (estilo Hermes-Agent): coluna de **identidade** à esquerda
//! (logo das engrenagens com o olho do HAL-9000 + metadados compactos) e coluna
//! de **seções categorizadas** à direita (Hardware, Sistema, Periféricos, Paleta).
//!
//! O bloco de conteúdo é centralizado harmoniosamente na área disponível via
//! layouts flexíveis (`Flex::Center`). A largura da coluna da logo é **fixada**
//! pelo tamanho escolhido — a tecla `.` (modo Detalhado) apenas revela linhas
//! extras nas seções da direita, **sem encolher a logo**. Em telas muito
//! estreitas a logo é reduzida (Main→Medium→Compact) ou recolhida, mantendo
//! apenas o painel de informações centralizado.

use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::ascii::{self, LogoSize};
use crate::backend::system::{DetailInfo, SystemSnapshot};

use super::theme::Palette;
use super::widgets::{
    bar_line, bar_line_suffix, human_bytes, human_uptime, kv_line, palette_line, section_title,
};

/// Folga horizontal entre a coluna de identidade e a de informações.
const GAP: u16 = 4;

/// Largura mínima reservada à coluna de informações ao decidir o tamanho da
/// logo. Mantém as seções legíveis e — por ser fixa — protege a largura da logo
/// ao alternar o modo detalhado.
const MIN_INFO: u16 = 34;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let block = super::content_block("Overview", pal);
    let inner = block.inner(area);
    f.render_widget(block, area);

    // Reserva a última linha para a statusline do Overview (indicador do modo).
    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner);
    draw_center(app, pal, f, rows[0]);
    draw_footer(app, pal, f, rows[1]);
}

/// Centraliza identidade + informações, protegendo a largura da logo.
fn draw_center(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    // Sem snapshot ainda: mensagem simples e centralizada.
    let Some(s) = &app.system else {
        let msg = Paragraph::new(Line::from(Span::styled(
            "coletando dados do sistema…",
            Style::default().fg(pal.dim),
        )));
        f.render_widget(msg, area);
        return;
    };

    match pick_size(&app.config.overview.ascii, area) {
        Some(size) => draw_two_columns(app, s, pal, f, area, size),
        None => draw_single_column(app, s, pal, f, area),
    }
}

/// Escolhe o tamanho da logo pela largura reservada à sua coluna **e** pela
/// altura disponível, degradando (Main→Medium→Compact→sem logo) até caber. O
/// orçamento de largura usa `MIN_INFO` fixo — independente dos campos
/// detalhados —, então a logo não encolhe ao alternar `.`.
fn pick_size(pref: &str, area: Rect) -> Option<LogoSize> {
    let budget = area.width.saturating_sub(GAP + MIN_INFO);
    let first = ascii::select(pref, budget)?;
    let order = [LogoSize::Main, LogoSize::Medium, LogoSize::Compact];
    let start = order.iter().position(|&s| s == first).unwrap_or(0);
    // Precisa caber a logo + linha em branco + ao menos a 1ª linha de metadados.
    order[start..]
        .iter()
        .copied()
        .find(|s| s.width() <= budget && area.height >= s.height() + 2)
}

/// Layout de duas colunas: identidade (logo + metadados) | seções.
fn draw_two_columns(
    app: &App,
    s: &SystemSnapshot,
    pal: &Palette,
    f: &mut Frame,
    area: Rect,
    size: LogoSize,
) {
    let meta = build_meta(s, pal);
    let meta_w = meta.iter().map(line_width).max().unwrap_or(0) as u16;

    // A largura da coluna da esquerda acomoda a logo e os metadados; ambos são
    // independentes do modo detalhado, então a coluna é estável.
    let left_w = size.width().max(meta_w).min(area.width);
    let remaining = area.width.saturating_sub(left_w + GAP).max(1);

    // A coluna de informações é dimensionada pelo seu conteúdo (não pelo espaço
    // restante), permitindo que o bloco inteiro seja centralizado em telas
    // largas em vez de colar na borda.
    let sections = build_sections(app, s, pal, remaining);
    let info_w = sections
        .iter()
        .map(line_width)
        .max()
        .unwrap_or(0)
        .min(remaining as usize)
        .max(1) as u16;

    // Altura do bloco = maior das duas colunas, limitada à área.
    let logo_h = size.height();
    let left_h = logo_h + 1 + meta.len() as u16;
    let content_h = left_h.max(sections.len() as u16).min(area.height).max(1);

    let vband = Layout::vertical([Constraint::Length(content_h)])
        .flex(Flex::Center)
        .split(area)[0];

    let cols = Layout::horizontal([
        Constraint::Length(left_w),
        Constraint::Length(GAP),
        Constraint::Length(info_w),
    ])
    .flex(Flex::Center)
    .split(vband);

    draw_identity(f, cols[0], size, meta);
    f.render_widget(Paragraph::new(Text::from(sections)), cols[2]);
}

/// Layout de coluna única (telas estreitas): metadados + seções, sem logo.
fn draw_single_column(app: &App, s: &SystemSnapshot, pal: &Palette, f: &mut Frame, area: Rect) {
    let info_w = area.width;
    let mut lines = build_meta(s, pal);
    lines.push(Line::from(""));
    lines.extend(build_sections(app, s, pal, info_w));

    let content_h = (lines.len() as u16).min(area.height).max(1);
    let vband = Layout::vertical([Constraint::Length(content_h)])
        .flex(Flex::Center)
        .split(area)[0];

    let info_w = info_w.min(area.width);
    let col = Layout::horizontal([Constraint::Length(info_w)])
        .flex(Flex::Center)
        .split(vband)[0];
    f.render_widget(Paragraph::new(Text::from(lines)), col);
}

/// Desenha a coluna de identidade: logo colorida + metadados compactos.
fn draw_identity(f: &mut Frame, area: Rect, size: LogoSize, meta: Vec<Line>) {
    let mut lines = ascii::logo_lines(size);
    lines.push(Line::from(""));
    lines.extend(meta);
    f.render_widget(Paragraph::new(Text::from(lines)), area);
}

/// Statusline interna do Overview com o indicador do modo de detalhe.
fn draw_footer(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let mode = if app.detailed_overview {
        "Expandido"
    } else {
        "Normal"
    };
    let line = Line::from(vec![
        Span::styled(" [.] ", Style::default().fg(pal.accent)),
        Span::styled("Detalhes: ", Style::default().fg(pal.dim)),
        Span::styled(mode, Style::default().fg(pal.fg)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

// --- Coluna de identidade (metadados) ------------------------------------

/// Metadados compactos exibidos sob a logo — idênticos nos modos Normal e
/// Detalhado (garantindo a estabilidade da coluna da esquerda).
fn build_meta<'a>(s: &SystemSnapshot, pal: &Palette) -> Vec<Line<'a>> {
    let mut out: Vec<Line> = Vec::new();

    // user@host + régua.
    out.push(Line::from(vec![
        Span::styled(s.user.clone(), Style::default().fg(pal.accent)),
        Span::styled("@", Style::default().fg(pal.dim)),
        Span::styled(s.host.clone(), Style::default().fg(pal.accent)),
    ]));
    let ruler = (s.user.len() + s.host.len() + 1).min(28);
    out.push(Line::from(Span::styled(
        "─".repeat(ruler),
        Style::default().fg(pal.dim),
    )));

    out.push(meta_line("uptime", human_uptime(s.uptime_secs), pal));

    // kernel · arch.
    let kernel = match &s.detail.cpu_arch {
        Some(arch) => format!("{} · {arch}", s.kernel),
        None => s.kernel.clone(),
    };
    out.push(meta_line("kernel", kernel, pal));

    // session: desktop/WM (+ tipo de sessão).
    let session = match (&s.detail.desktop, &s.detail.session_type) {
        (Some(de), Some(st)) => format!("{de} ({st})"),
        (Some(de), None) => de.clone(),
        (None, Some(st)) => st.clone(),
        (None, None) => s.shell.clone(),
    };
    out.push(meta_line("session", session, pal));

    out
}

/// Linha de metadado compacta `rótulo  valor` (rótulo esmaecido).
fn meta_line<'a>(label: &'a str, value: String, pal: &Palette) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{label:<8}"), Style::default().fg(pal.dim)),
        Span::styled(value, Style::default().fg(pal.fg)),
    ])
}

// --- Coluna de seções ----------------------------------------------------

/// Monta as seções categorizadas da coluna direita.
fn build_sections<'a>(app: &App, s: &SystemSnapshot, pal: &Palette, width: u16) -> Vec<Line<'a>> {
    let bar_w = (width as usize / 4).clamp(6, 20);
    let detailed = app.detailed_overview;
    let mut out: Vec<Line> = Vec::new();

    section_hardware(s, pal, bar_w, detailed, &mut out);
    out.push(Line::from(""));
    section_platform(s, pal, bar_w, detailed, &mut out);
    out.push(Line::from(""));
    section_power(s, pal, bar_w, detailed, &mut out);
    out.push(Line::from(""));
    out.push(section_title("Color Palette", pal));
    out.push(palette_line());

    out
}

/// Seção **Available Compute / Hardware**.
fn section_hardware(
    s: &SystemSnapshot,
    pal: &Palette,
    bar_w: usize,
    detailed: bool,
    out: &mut Vec<Line>,
) {
    let d: &DetailInfo = &s.detail;
    out.push(section_title("Available Compute / Hardware", pal));

    out.push(kv_line("CPU", s.cpu_name.clone(), pal));

    // Núcleos / frequência / arquitetura.
    let cores = match d.cpu_cores_physical {
        Some(p) => format!("{p}c / {}t", d.cpu_cores_logical),
        None => format!("{}t", d.cpu_cores_logical),
    };
    let mut cpu_det = cores;
    if let Some(fq) = d.cpu_freq_ghz {
        cpu_det.push_str(&format!(" @ {fq:.2} GHz"));
    }
    out.push(kv_line("Núcleos", cpu_det, pal));
    out.push(bar_line("Uso CPU", s.cpu_ratio(), bar_w, pal));

    // RAM.
    out.push(kv_line(
        "RAM",
        format!("{} / {}", human_bytes(s.mem_used), human_bytes(s.mem_total)),
        pal,
    ));
    out.push(bar_line("Mem", s.mem_ratio(), bar_w, pal));

    if detailed {
        // Swap.
        if d.swap_total > 0 {
            out.push(kv_line(
                "Swap",
                format!(
                    "{} / {}",
                    human_bytes(d.swap_used),
                    human_bytes(d.swap_total)
                ),
                pal,
            ));
            out.push(bar_line("Swap", d.swap_ratio(), bar_w, pal));
        } else {
            out.push(kv_line("Swap", "N/A".into(), pal));
        }

        // Temperatura da CPU.
        if let Some(t) = d.cpu_temp_c {
            out.push(kv_line("Temp CPU", format!("{t:.0} °C"), pal));
        }

        // Placa-mãe.
        if let Some(board) = join_opt(d.board_vendor.as_deref(), d.board_name.as_deref()) {
            out.push(kv_line("Placa", board, pal));
        }

        // GPU.
        if let Some(gpu) = &d.gpu {
            out.push(kv_line("GPU", gpu.clone(), pal));
        }
    }
}

/// Seção **System & Platform**.
fn section_platform(
    s: &SystemSnapshot,
    pal: &Palette,
    bar_w: usize,
    detailed: bool,
    out: &mut Vec<Line>,
) {
    let d: &DetailInfo = &s.detail;
    out.push(section_title("System & Platform", pal));

    out.push(kv_line("OS", s.os.clone(), pal));
    if let Some(model) = &s.host_model {
        out.push(kv_line("Host", model.clone(), pal));
    }
    out.push(kv_line("Kernel", s.kernel.clone(), pal));

    if detailed {
        match (&d.bios_version, &d.bios_date) {
            (Some(v), Some(dt)) => out.push(kv_line("BIOS", format!("{v} ({dt})"), pal)),
            (Some(v), None) => out.push(kv_line("BIOS", v.clone(), pal)),
            _ => {}
        }
    }

    out.push(kv_line(
        "Pacotes",
        s.packages
            .as_ref()
            .map(|p| p.summary())
            .unwrap_or_else(|| "N/A".into()),
        pal,
    ));

    // Shell (+ WM/desktop no modo detalhado).
    let shell = match (detailed, &d.desktop) {
        (true, Some(de)) => format!("{} · {de}", s.shell),
        _ => s.shell.clone(),
    };
    out.push(kv_line("Shell", shell, pal));

    // Disco raiz.
    match (s.disk_ratio(), s.disk_used, s.disk_total) {
        (Some(r), Some(u), Some(t)) => {
            out.push(kv_line(
                "Disco /",
                format!("{} / {}", human_bytes(u), human_bytes(t)),
                pal,
            ));
            out.push(bar_line("Disco", r, bar_w, pal));
        }
        _ => out.push(kv_line("Disco /", "N/A".into(), pal)),
    }
}

/// Seção **Peripherals & Power**.
fn section_power(
    s: &SystemSnapshot,
    pal: &Palette,
    bar_w: usize,
    detailed: bool,
    out: &mut Vec<Line>,
) {
    out.push(section_title("Peripherals & Power", pal));

    // Bateria (ou N/A em desktop) — sufixo neofetch: [DISCHARGING -14W].
    match &s.battery {
        Some(b) => {
            let suffix = battery_suffix(b);
            out.push(bar_line_suffix("Bateria", b.ratio(), bar_w, pal, &suffix));

            if detailed {
                let mut parts: Vec<String> = Vec::new();
                if let Some(h) = b.health {
                    parts.push(format!("saúde {:.0}%", h * 100.0));
                }
                if let Some(c) = b.cycle_count {
                    parts.push(format!("{c} ciclos"));
                }
                if let Some(tech) = &b.technology {
                    parts.push(tech.clone());
                }
                if !parts.is_empty() {
                    out.push(kv_line("Bateria+", parts.join(" · "), pal));
                }
            }
        }
        None => out.push(kv_line("Bateria", "N/A (Desktop)".into(), pal)),
    }

    // Brilho.
    match s.brightness {
        Some(r) => out.push(bar_line("Brilho", r, bar_w, pal)),
        None => out.push(kv_line("Brilho", "N/A".into(), pal)),
    }

    // Volume — sem emojis; [MUTED] apenas quando mudo.
    match &s.volume {
        Some(v) if v.muted => {
            out.push(bar_line_suffix("Volume", v.ratio(), bar_w, pal, "[MUTED]"));
        }
        Some(v) => out.push(bar_line("Volume", v.ratio(), bar_w, pal)),
        None => out.push(kv_line("Volume", "N/A".into(), pal)),
    }
}

/// Sufixo textual de bateria no estilo neofetch clássico, sem emojis:
/// `[CHARGING +18W]`, `[DISCHARGING -14W]`, `[FULL]`.
fn battery_suffix(b: &crate::backend::system::Battery) -> String {
    match (b.power_watts, b.status.power_sign()) {
        (Some(w), sign) if !sign.is_empty() => {
            format!("[{} {}{:.0}W]", b.status.tag(), sign, w)
        }
        _ => format!("[{}]", b.status.tag()),
    }
}

/// Junta dois campos opcionais com espaço, retornando `None` se ambos vazios.
fn join_opt(a: Option<&str>, b: Option<&str>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (Some(a), None) => Some(a.to_string()),
        (None, Some(b)) => Some(b.to_string()),
        (None, None) => None,
    }
}

/// Largura de exibição (colunas) de uma `Line`, somando seus spans.
fn line_width(line: &Line) -> usize {
    line.spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}
