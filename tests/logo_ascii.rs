//! Testes da logo das engrenagens com o olho do HAL-9000 (`src/ascii.rs`):
//! invariantes de dimensão, seleção por orçamento e colorização multi-span
//! (engrenagens em bronze/cinza/âmbar/ouro; olho em vermelho).

use hal9001::ascii::{self, LogoSize};
use ratatui::style::Color;
use unicode_width::UnicodeWidthStr;

const ALL: [LogoSize; 3] = [LogoSize::Main, LogoSize::Medium, LogoSize::Compact];

/// Reconstrói o texto cru de uma logo a partir dos spans coloridos.
fn logo_text(size: LogoSize) -> Vec<String> {
    ascii::logo_lines(size)
        .into_iter()
        .map(|line| {
            line.spans
                .iter()
                .map(|s| s.content.to_string())
                .collect::<String>()
        })
        .collect()
}

#[test]
fn arts_have_uniform_line_width_and_ascii_only() {
    for size in ALL {
        let lines = logo_text(size);
        let w = size.width() as usize;
        assert!(w > 0 && !lines.is_empty(), "logo vazia");
        for (i, l) in lines.iter().enumerate() {
            // Largura uniforme mantém o olho central alinhado (colorização
            // radial simétrica) e o layout de duas colunas estável.
            assert_eq!(
                UnicodeWidthStr::width(l.as_str()),
                w,
                "linha {i} com largura divergente em {size:?}"
            );
            assert!(l.is_ascii(), "logo deve ser somente ASCII ({size:?})");
        }
        assert_eq!(lines.len() as u16, size.height());
    }
}

#[test]
fn sizes_are_ordered_main_medium_compact() {
    assert!(LogoSize::Main.width() > LogoSize::Medium.width());
    assert!(LogoSize::Medium.width() > LogoSize::Compact.width());
    assert!(LogoSize::Main.height() >= LogoSize::Medium.height());
    assert!(LogoSize::Medium.height() >= LogoSize::Compact.height());
}

#[test]
fn eye_and_gears_are_present() {
    for size in ALL {
        let joined = logo_text(size).join("\n");
        // Olho do HAL: núcleo incandescente 'O'.
        assert!(joined.contains('O'), "olho do HAL ausente em {size:?}");
        // Engrenagem: dentes '#' (anel externo).
        assert!(joined.contains('#'), "engrenagem ausente em {size:?}");
    }
}

#[test]
fn eye_core_is_red_and_gears_are_not() {
    // O núcleo 'O' recebe vermelho vivo; os dentes '#' recebem bronze (nunca
    // vermelho). Garante a colorização multi-span exigida pelo briefing.
    let red = Color::Rgb(255, 50, 50);
    let bronze = Color::Rgb(180, 140, 60);

    for size in ALL {
        let mut saw_red_eye = false;
        let mut saw_bronze_gear = false;
        for line in ascii::logo_lines(size) {
            for span in &line.spans {
                if span.content.contains('O') {
                    assert_eq!(span.style.fg, Some(red), "olho não-vermelho em {size:?}");
                    saw_red_eye = true;
                }
                if span.content.contains('#') {
                    assert_eq!(
                        span.style.fg,
                        Some(bronze),
                        "engrenagem com cor errada em {size:?}"
                    );
                    saw_bronze_gear = true;
                }
            }
        }
        assert!(saw_red_eye && saw_bronze_gear, "spans esperados ausentes em {size:?}");
    }
}

#[test]
fn select_respects_forced_preference() {
    assert_eq!(ascii::select("main", 0), Some(LogoSize::Main));
    assert_eq!(ascii::select("A", 0), Some(LogoSize::Main));
    assert_eq!(ascii::select("medium", 0), Some(LogoSize::Medium));
    assert_eq!(ascii::select("compact", 0), Some(LogoSize::Compact));
    assert_eq!(ascii::select("none", 999), None);
    assert_eq!(ascii::select("off", 999), None);
}

#[test]
fn select_auto_picks_largest_that_fits() {
    // Orçamento generoso → maior logo.
    assert_eq!(ascii::select("auto", 999), Some(LogoSize::Main));
    // Logo intermediária cabe, principal não.
    let budget = LogoSize::Medium.width();
    assert_eq!(ascii::select("auto", budget), Some(LogoSize::Medium));
    // Só a compacta cabe.
    let budget = LogoSize::Compact.width();
    assert_eq!(ascii::select("auto", budget), Some(LogoSize::Compact));
    // Nem a compacta cabe → sem logo.
    assert_eq!(ascii::select("auto", LogoSize::Compact.width() - 1), None);
}
