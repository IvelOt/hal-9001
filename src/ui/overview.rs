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
    human_bytes, human_uptime, kv_line, metric_line, palette_line, section_title, truncate_str,
};

/// Folga horizontal entre a coluna de identidade e a de informações.
const GAP: u16 = 4;

/// Largura mínima reservada à coluna de informações ao decidir o tamanho da
/// logo. Mantém as seções legíveis e — por ser fixa — protege a largura da logo
/// ao alternar o modo detalhado.
const MIN_INFO: u16 = 34;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let block = super::content_block(m.tab_overview, pal);
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

    draw_identity(f, cols[0], size, meta, eye_phase(app));
    f.render_widget(Paragraph::new(Text::from(sections)), cols[2]);
}

/// Fase do pulso de respiração do Olho do HAL, derivada do tempo decorrido.
/// Cicla 0→1→2→3 a cada 250 ms, produzindo uma pulsação sutil e contínua.
fn eye_phase(app: &App) -> u8 {
    ((app.elapsed_ms() / 250) % 4) as u8
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

/// Desenha a coluna de identidade: logo colorida (com o pulso do olho na `phase`
/// atual) + metadados compactos.
fn draw_identity(f: &mut Frame, area: Rect, size: LogoSize, meta: Vec<Line>, phase: u8) {
    let mut lines = ascii::logo_lines_phase(size, phase);
    lines.push(Line::from(""));
    lines.extend(meta);
    f.render_widget(Paragraph::new(Text::from(lines)), area);
}

/// Statusline interna do Overview: indicador do modo de detalhe + atalhos de
/// controle interativo (brilho/volume/mudo).
fn draw_footer(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let mode = if app.detailed_overview {
        if app.lang == crate::i18n::Language::EnUs {
            "Expanded"
        } else {
            "Expandido"
        }
    } else {
        "Normal"
    };
    let hint = |key: &'static str, label: &'static str| {
        [
            Span::styled(format!(" [{key}] "), Style::default().fg(pal.accent)),
            Span::styled(label, Style::default().fg(pal.dim)),
        ]
    };
    let mut spans = Vec::new();
    let details_label = if app.lang == crate::i18n::Language::EnUs {
        "Details:"
    } else if app.lang == crate::i18n::Language::EsEs {
        "Detalles:"
    } else {
        "Detalhes:"
    };
    spans.extend(hint(".", details_label));
    spans.push(Span::styled(format!("{mode} "), Style::default().fg(pal.fg)));
    spans.extend(hint("p", m.label_power_profile));
    spans.extend(hint("b/B", m.label_brightness));
    spans.extend(hint("v/V", m.label_volume));
    spans.extend(hint("m", m.hint_mute));
    spans.extend(hint("c", m.hint_config));
    // Trunca a linha inteira à largura disponível para nunca vazar.
    let mut line = Line::from(spans);
    if line_width(&line) > area.width as usize {
        let text = truncate_str(&spans_text(&line), area.width as usize);
        line = Line::from(Span::styled(text, Style::default().fg(pal.dim)));
    }
    f.render_widget(Paragraph::new(line), area);
}

/// Concatena o texto de todos os spans de uma linha.
fn spans_text(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
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

/// Larguras das colunas de informação, derivadas da largura disponível. As
/// linhas densas (`métrica + barra`) usam `bar_w` para a barra e `val_w` para o
/// valor alinhado; as linhas `rótulo valor` usam `width` para o truncamento.
#[derive(Clone, Copy)]
struct Cols {
    /// Largura total disponível para a coluna de informações.
    width: usize,
    /// Largura da barra de progresso (blocos internos).
    bar_w: usize,
    /// Largura alinhada da coluna de valor nas linhas densas.
    val_w: usize,
}

/// Largura-alvo máxima da coluna de informações. Impede que valores longos
/// (GPU/BIOS/Host) estiquem a coluna até a borda em telas largas — preservando
/// a folga que permite centralizar o bloco (`Flex::Center`).
const MAX_INFO_W: usize = 48;

impl Cols {
    fn new(width: u16) -> Self {
        // Limita a largura-alvo para manter a coluna compacta e centralizável.
        let width = (width as usize).min(MAX_INFO_W);
        let bar_w = (width / 4).clamp(6, 14);
        // Reserva: rótulo(10) + barra(`[`+bar_w+`] `+`NNN%`) + 1 folga.
        let reserved = 10 + bar_w + 7 + 1;
        let val_w = width.saturating_sub(reserved).clamp(8, 40);
        Self {
            width,
            bar_w,
            val_w,
        }
    }
}

/// Monta as seções categorizadas da coluna direita.
///
/// O layout é **denso**: cada métrica (CPU/RAM/Swap/Disco/Bateria/Brilho/
/// Volume) ocupa uma única linha combinando valor + barra, e as seções são
/// separadas apenas pelos títulos (sem linhas em branco), mantendo o total em
fn build_sections<'a>(app: &App, s: &SystemSnapshot, pal: &Palette, width: u16) -> Vec<Line<'a>> {
    let cols = Cols::new(width);
    let detailed = app.detailed_overview;
    let m = app.lang.messages();
    let mut out: Vec<Line> = Vec::new();

    section_hardware(s, pal, cols, detailed, m, &mut out);
    section_platform(s, pal, cols, detailed, m, &mut out);
    section_power(s, pal, cols, detailed, m, &mut out);
    out.push(section_title(m.sec_palette, pal));
    out.push(palette_line());

    out
}

/// Seção **Available Compute / Hardware** — linhas densas (métrica + barra).
fn section_hardware(
    s: &SystemSnapshot,
    pal: &Palette,
    cols: Cols,
    detailed: bool,
    m: &'static crate::i18n::Messages,
    out: &mut Vec<Line>,
) {
    let d: &DetailInfo = &s.detail;
    out.push(section_title(m.sec_compute, pal));

    // CPU: nome limpo + núcleos combinados numa única linha com a barra de uso.
    let cores = match d.cpu_cores_physical {
        Some(p) => format!("{p}c/{}t", d.cpu_cores_logical),
        None => format!("{}t", d.cpu_cores_logical),
    };
    let cpu_val = format!("{} ({cores})", clean_cpu(&s.cpu_name));
    out.push(metric_line(
        "CPU",
        &cpu_val,
        cols.val_w,
        s.cpu_ratio(),
        cols.bar_w,
        pal,
        None,
    ));

    // RAM: uso / total + barra.
    out.push(metric_line(
        m.label_ram,
        &format!("{} / {}", human_bytes(s.mem_used), human_bytes(s.mem_total)),
        cols.val_w,
        s.mem_ratio(),
        cols.bar_w,
        pal,
        None,
    ));

    if detailed {
        // Swap.
        if d.swap_total > 0 {
            out.push(metric_line(
                m.label_swap,
                &format!(
                    "{} / {}",
                    human_bytes(d.swap_used),
                    human_bytes(d.swap_total)
                ),
                cols.val_w,
                d.swap_ratio(),
                cols.bar_w,
                pal,
                None,
            ));
        } else {
            out.push(kv_line(m.label_swap, "N/A".into(), cols.width, pal));
        }

        // Temperatura da CPU (+ frequência, quando houver).
        if let Some(t) = d.cpu_temp_c {
            let val = match d.cpu_freq_ghz {
                Some(fq) => format!("{t:.0} °C @ {fq:.2} GHz"),
                None => format!("{t:.0} °C"),
            };
            out.push(kv_line(m.label_temperature, val, cols.width, pal));
        }

        // Placa-mãe.
        if let Some(board) = join_opt(d.board_vendor.as_deref(), d.board_name.as_deref()) {
            out.push(kv_line(m.label_board, board, cols.width, pal));
        }

        // GPU (nome limpo).
        if let Some(gpu) = &d.gpu {
            out.push(kv_line(m.label_gpu, clean_gpu(gpu), cols.width, pal));
        }
    }
}

/// Seção **System & Platform**.
fn section_platform(
    s: &SystemSnapshot,
    pal: &Palette,
    cols: Cols,
    detailed: bool,
    m: &'static crate::i18n::Messages,
    out: &mut Vec<Line>,
) {
    let d: &DetailInfo = &s.detail;
    out.push(section_title(m.sec_system, pal));

    out.push(kv_line(m.label_os, s.os.clone(), cols.width, pal));
    if let Some(model) = &s.host_model {
        out.push(kv_line(m.label_host, model.clone(), cols.width, pal));
    }
    out.push(kv_line(m.label_kernel, s.kernel.clone(), cols.width, pal));

    if detailed {
        match (&d.bios_version, &d.bios_date) {
            (Some(v), Some(dt)) => out.push(kv_line(m.label_bios, format!("{v} ({dt})"), cols.width, pal)),
            (Some(v), None) => out.push(kv_line(m.label_bios, v.clone(), cols.width, pal)),
            _ => {}
        }
    }

    out.push(kv_line(
        m.label_packages,
        s.packages
            .as_ref()
            .map(|p| p.summary())
            .unwrap_or_else(|| "N/A".into()),
        cols.width,
        pal,
    ));

    // Shell (+ WM/desktop no modo detalhado).
    let shell = match (detailed, &d.desktop) {
        (true, Some(de)) => format!("{} · {de}", s.shell),
        _ => s.shell.clone(),
    };
    out.push(kv_line(m.label_shell, shell, cols.width, pal));

    // Disco raiz — linha densa quando há dados.
    match (s.disk_ratio(), s.disk_used, s.disk_total) {
        (Some(r), Some(u), Some(t)) => out.push(metric_line(
            m.label_disk_root,
            &format!("{} / {}", human_bytes(u), human_bytes(t)),
            cols.val_w,
            r,
            cols.bar_w,
            pal,
            None,
        )),
        _ => out.push(kv_line(m.label_disk_root, "N/A".into(), cols.width, pal)),
    }
}

/// Seção **Peripherals & Power** — bateria/brilho/volume em linhas densas.
fn section_power(
    s: &SystemSnapshot,
    pal: &Palette,
    cols: Cols,
    detailed: bool,
    m: &'static crate::i18n::Messages,
    out: &mut Vec<Line>,
) {
    out.push(section_title(m.sec_peripherals, pal));

    // Bateria (ou N/A em desktop) — sufixo neofetch: [DISCHARGING -14W].
    match &s.battery {
        Some(b) => {
            let suffix = battery_suffix(b);
            out.push(metric_line(
                m.label_battery,
                "",
                cols.val_w,
                b.ratio(),
                cols.bar_w,
                pal,
                Some(&suffix),
            ));

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
                    out.push(kv_line("Bateria+", parts.join(" · "), cols.width, pal));
                }
            }
        }
        None => out.push(kv_line(m.label_battery, "N/A (Desktop)".into(), cols.width, pal)),
    }

    // Brilho.
    match s.brightness {
        Some(r) => out.push(metric_line(
            m.label_brightness, "", cols.val_w, r, cols.bar_w, pal, None,
        )),
        None => out.push(kv_line(m.label_brightness, "N/A".into(), cols.width, pal)),
    }

    // Volume — sem emojis; [MUTED] apenas quando mudo.
    match &s.volume {
        Some(v) => {
            let suffix = if v.muted { Some("[MUTED]") } else { None };
            out.push(metric_line(
                m.label_volume,
                "",
                cols.val_w,
                v.ratio(),
                cols.bar_w,
                pal,
                suffix,
            ));
        }
        None => out.push(kv_line(m.label_volume, "N/A".into(), cols.width, pal)),
    }

    // Perfil de energia — tag neofetch + dica de atalho; N/A gracioso quando
    // não há daemon/governor (ex.: desktop).
    match s.power_profile {
        Some(p) => out.push(kv_line(
            m.label_power_profile,
            format!("{} (p: alternar)", p.tag()),
            cols.width,
            pal,
        )),
        None => out.push(kv_line(m.label_power_profile, "N/A".into(), cols.width, pal)),
    }
}

/// Encurta o nome da CPU removendo ruído de marketing (`(R)`, `(TM)`, `CPU`,
/// `Processor`), colapsando espaços redundantes.
fn clean_cpu(name: &str) -> String {
    let cleaned = name
        .replace("(R)", "")
        .replace("(r)", "")
        .replace("(TM)", "")
        .replace("(tm)", "")
        .replace(" CPU", "")
        .replace(" Processor", "");
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.is_empty() {
        name.to_string()
    } else {
        joined
    }
}

/// Encurta o nome da GPU: prefere o nome comercial entre colchetes
/// (`[Iris Xe Graphics]`) quando presente; senão remove `Corporation`.
fn clean_gpu(name: &str) -> String {
    if let Some(open) = name.find('[') {
        if let Some(close_rel) = name[open + 1..].find(']') {
            let inner = name[open + 1..open + 1 + close_rel].trim();
            if !inner.is_empty() {
                return inner.to_string();
            }
        }
    }
    let cleaned = name.replace(" Corporation", "");
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.is_empty() {
        name.to_string()
    } else {
        joined
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
