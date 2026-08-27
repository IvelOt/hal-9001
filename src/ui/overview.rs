use ratatui::layout::{Constraint, Flex, Layout, Rect};
use ratatui::style::{Modifier, Style};
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

const GAP: u16 = 4;

const MIN_INFO: u16 = 34;

pub fn draw(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    let m = app.lang.messages();
    let block = super::content_block(m.tab_overview, pal);
    let inner = block.inner(area);
    f.render_widget(block, area);

    let rows = Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).split(inner);
    draw_center(app, pal, f, rows[0]);
    draw_footer(app, pal, f, rows[1]);
}

fn draw_center(app: &App, pal: &Palette, f: &mut Frame, area: Rect) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let m = app.lang.messages();
    let Some(s) = &app.system else {
        let msg = Paragraph::new(Line::from(Span::styled(
            m.overview_collecting,
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

fn pick_size(pref: &str, area: Rect) -> Option<LogoSize> {
    if area.height > area.width || area.width < 72 {
        return None;
    }
    let budget = area.width.saturating_sub(GAP + MIN_INFO);
    let first = ascii::select(pref, budget)?;
    let order = [LogoSize::Main, LogoSize::Medium, LogoSize::Compact];
    let start = order.iter().position(|&s| s == first).unwrap_or(0);

    order[start..]
        .iter()
        .copied()
        .find(|s| s.width() <= budget && area.height >= s.height() + 2)
}

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

    let left_w = size.width().max(meta_w).min(area.width);
    let remaining = area.width.saturating_sub(left_w + GAP).max(1);

    let sections = build_sections(app, s, pal, remaining);
    let info_w = sections
        .iter()
        .map(line_width)
        .max()
        .unwrap_or(0)
        .min(remaining as usize)
        .max(1) as u16;

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

fn eye_phase(app: &App) -> u8 {
    ((app.elapsed_ms() / 250) % 4) as u8
}

fn draw_single_column(app: &App, s: &SystemSnapshot, pal: &Palette, f: &mut Frame, area: Rect) {
    let info_w = area.width;
    let mut lines = Vec::new();

    let logo_size = if app.config.overview.ascii != "none" && app.config.overview.ascii != "off" {
        if area.width >= 30 && area.height >= 34 {
            Some(LogoSize::Compact)
        } else {
            None
        }
    } else {
        None
    };

    if let Some(size) = logo_size {
        let logo = ascii::logo_lines_phase(size, eye_phase(app));
        let logo_w = size.width() as usize;
        let pad = (info_w as usize).saturating_sub(logo_w) / 2;
        let pad_str = " ".repeat(pad);

        for line in logo {
            let mut spans = vec![Span::raw(pad_str.clone())];
            spans.extend(line.spans);
            lines.push(Line::from(spans));
        }
        lines.push(Line::from(""));
    }

    let title_pad = (info_w as usize).saturating_sub(8) / 2;
    lines.push(Line::from(vec![
        Span::raw(" ".repeat(title_pad)),
        Span::styled(
            "HAL-9001",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(""));

    lines.extend(build_meta(s, pal));
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

fn draw_identity(f: &mut Frame, area: Rect, size: LogoSize, meta: Vec<Line>, phase: u8) {
    let mut lines = ascii::logo_lines_phase(size, phase);
    lines.push(Line::from(""));
    lines.extend(meta);
    f.render_widget(Paragraph::new(Text::from(lines)), area);
}

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

    if area.width < 50 {
        spans.extend(hint(".", "Det"));
        spans.extend(hint("p", "Perfil"));
        spans.extend(hint("c", "Config"));
    } else if area.width < 68 {
        spans.extend(hint(".", "Detalhe"));
        spans.extend(hint("p", m.label_power_profile));
        spans.extend(hint("b/v", "Brilho/Vol"));
        spans.extend(hint("c", "Config"));
    } else {
        let details_label = if app.lang == crate::i18n::Language::EnUs {
            "Details:"
        } else if app.lang == crate::i18n::Language::EsEs {
            "Detalles:"
        } else {
            "Detalhes:"
        };
        spans.extend(hint(".", details_label));
        spans.push(Span::styled(
            format!("{mode} "),
            Style::default().fg(pal.fg),
        ));
        let mute_label = match app.lang {
            crate::i18n::Language::EnUs => "Mute",
            crate::i18n::Language::EsEs => "Mudo",
            crate::i18n::Language::PtBr => "Mudo",
        };
        let config_label = match app.lang {
            crate::i18n::Language::EnUs => "Config",
            crate::i18n::Language::EsEs => "Config",
            crate::i18n::Language::PtBr => "Config",
        };
        spans.extend(hint("p", m.label_power_profile));
        spans.extend(hint("b/B", m.label_brightness));
        spans.extend(hint("v/V", m.label_volume));
        spans.extend(hint("m", mute_label));
        spans.extend(hint("c", config_label));
    }

    let mut line = Line::from(spans);
    if line_width(&line) > area.width as usize {
        let text = truncate_str(&spans_text(&line), area.width as usize);
        line = Line::from(Span::styled(text, Style::default().fg(pal.dim)));
    }
    f.render_widget(Paragraph::new(line), area);
}

fn spans_text(line: &Line) -> String {
    line.spans.iter().map(|s| s.content.as_ref()).collect()
}

fn build_meta<'a>(s: &SystemSnapshot, pal: &Palette) -> Vec<Line<'a>> {
    let mut out: Vec<Line> = Vec::new();

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

    let kernel = match &s.detail.cpu_arch {
        Some(arch) => format!("{} · {arch}", s.kernel),
        None => s.kernel.clone(),
    };
    out.push(meta_line("kernel", kernel, pal));

    let session = match (&s.detail.desktop, &s.detail.session_type) {
        (Some(de), Some(st)) => format!("{de} ({st})"),
        (Some(de), None) => de.clone(),
        (None, Some(st)) => st.clone(),
        (None, None) => s.shell.clone(),
    };
    out.push(meta_line("session", session, pal));

    out
}

fn meta_line<'a>(label: &'a str, value: String, pal: &Palette) -> Line<'a> {
    Line::from(vec![
        Span::styled(format!("{label:<8}"), Style::default().fg(pal.dim)),
        Span::styled(value, Style::default().fg(pal.fg)),
    ])
}

#[derive(Clone, Copy)]
struct Cols {
    width: usize,

    bar_w: usize,

    val_w: usize,
}

const MAX_INFO_W: usize = 48;

impl Cols {
    fn new(width: u16) -> Self {
        let width = (width as usize).min(MAX_INFO_W);
        let bar_w = (width / 4).clamp(6, 14);

        let reserved = 10 + bar_w + 7 + 1;
        let val_w = width.saturating_sub(reserved).clamp(8, 40);
        Self {
            width,
            bar_w,
            val_w,
        }
    }
}

fn build_sections<'a>(app: &App, s: &SystemSnapshot, pal: &Palette, width: u16) -> Vec<Line<'a>> {
    let cols = Cols::new(width);
    let detailed = app.detailed_overview;
    let m = app.lang.messages();
    let mut out: Vec<Line> = Vec::new();

    section_hardware(s, pal, cols, detailed, m, &mut out);
    if detailed {
        section_top_processes(s, pal, cols, &mut out);
    }
    section_platform(s, pal, cols, detailed, m, &mut out);
    section_power(s, pal, cols, detailed, m, &mut out);
    out.push(section_title(m.sec_palette, pal));
    out.push(palette_line());

    out
}

fn section_top_processes<'a>(
    s: &SystemSnapshot,
    pal: &Palette,
    cols: Cols,
    out: &mut Vec<Line<'a>>,
) {
    out.push(section_title("TOP PROCESSES", pal));

    let pid_w = 6;
    let cpu_w = 6;
    let ram_w = 8;
    let reserved = pid_w + cpu_w + ram_w + 3;
    let proc_w = if cols.width > reserved {
        cols.width - reserved
    } else {
        5
    };

    let header = format!(
        "{:<pid_w$} {:<proc_w$} {:>cpu_w$} {:>ram_w$}",
        "PID",
        "PROCESSO",
        "CPU%",
        "RAM",
        pid_w = pid_w,
        proc_w = proc_w,
        cpu_w = cpu_w,
        ram_w = ram_w
    );
    out.push(Line::from(vec![Span::styled(
        header,
        ratatui::style::Style::default()
            .fg(pal.dim)
            .add_modifier(ratatui::style::Modifier::BOLD),
    )]));

    for p in &s.detail.top_processes {
        let pid_str = p.pid.to_string();
        let cpu_str = format!("{:.1}", p.cpu_usage);
        let ram_str = human_bytes(p.mem_bytes);

        let name_trunc = truncate_str(&p.name, proc_w);
        let row = format!(
            "{:<pid_w$} {:<proc_w$} {:>cpu_w$} {:>ram_w$}",
            pid_str,
            name_trunc,
            cpu_str,
            ram_str,
            pid_w = pid_w,
            proc_w = proc_w,
            cpu_w = cpu_w,
            ram_w = ram_w
        );
        out.push(Line::from(vec![Span::styled(
            row,
            ratatui::style::Style::default().fg(pal.fg),
        )]));
    }
}

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

        if let Some(t) = d.cpu_temp_c {
            let val = match d.cpu_freq_ghz {
                Some(fq) => format!("{t:.0} °C @ {fq:.2} GHz"),
                None => format!("{t:.0} °C"),
            };
            out.push(kv_line(m.label_temperature, val, cols.width, pal));
        }

        if let Some(board) = join_opt(d.board_vendor.as_deref(), d.board_name.as_deref()) {
            out.push(kv_line(m.label_board, board, cols.width, pal));
        }

        if let Some(gpu) = &d.gpu {
            out.push(kv_line(m.label_gpu, clean_gpu(gpu), cols.width, pal));
        }
    }
}

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
            (Some(v), Some(dt)) => out.push(kv_line(
                m.label_bios,
                format!("{v} ({dt})"),
                cols.width,
                pal,
            )),
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

    let shell = match (detailed, &d.desktop) {
        (true, Some(de)) => format!("{} · {de}", s.shell),
        _ => s.shell.clone(),
    };
    out.push(kv_line(m.label_shell, shell, cols.width, pal));

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

fn section_power(
    s: &SystemSnapshot,
    pal: &Palette,
    cols: Cols,
    detailed: bool,
    m: &'static crate::i18n::Messages,
    out: &mut Vec<Line>,
) {
    out.push(section_title(m.sec_peripherals, pal));

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
                    parts.push(format!("{} {:.0}%", m.overview_health_label, h * 100.0));
                }
                if let Some(c) = b.cycle_count {
                    parts.push(format!("{c} {}", m.overview_cycles_suffix));
                }
                if let Some(tech) = &b.technology {
                    parts.push(tech.clone());
                }
                if !parts.is_empty() {
                    out.push(kv_line(
                        m.overview_battery_extra_label,
                        parts.join(" · "),
                        cols.width,
                        pal,
                    ));
                }
            }
        }
        None => out.push(kv_line(
            m.label_battery,
            m.overview_desktop_na.into(),
            cols.width,
            pal,
        )),
    }

    match s.brightness {
        Some(r) => out.push(metric_line(
            m.label_brightness,
            "",
            cols.val_w,
            r,
            cols.bar_w,
            pal,
            None,
        )),
        None => out.push(kv_line(m.label_brightness, "N/A".into(), cols.width, pal)),
    }

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

    match s.power_profile {
        Some(p) => out.push(kv_line(
            m.label_power_profile,
            format!("{} (p: alternar)", p.tag()),
            cols.width,
            pal,
        )),
        None => out.push(kv_line(
            m.label_power_profile,
            "N/A".into(),
            cols.width,
            pal,
        )),
    }
}

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

fn battery_suffix(b: &crate::backend::system::Battery) -> String {
    match (b.power_watts, b.status.power_sign()) {
        (Some(w), sign) if !sign.is_empty() => {
            format!("[{} {}{:.0}W]", b.status.tag(), sign, w)
        }
        _ => format!("[{}]", b.status.tag()),
    }
}

fn join_opt(a: Option<&str>, b: Option<&str>) -> Option<String> {
    match (a, b) {
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (Some(a), None) => Some(a.to_string()),
        (None, Some(b)) => Some(b.to_string()),
        (None, None) => None,
    }
}

fn line_width(line: &Line) -> usize {
    line.spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}
