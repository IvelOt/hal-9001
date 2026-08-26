
use hal9001::ascii::{self, LogoSize};
use ratatui::style::Color;
use unicode_width::UnicodeWidthStr;

const ALL: [LogoSize; 3] = [LogoSize::Main, LogoSize::Medium, LogoSize::Compact];

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

        assert!(joined.contains('O'), "olho do HAL ausente em {size:?}");

        assert!(joined.contains('#'), "engrenagem ausente em {size:?}");
    }
}

#[test]
fn eye_core_is_red_and_gears_are_not() {

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
fn eye_pulse_phase0_matches_static_logo() {

    for size in ALL {
        let stat = ascii::logo_lines(size);
        let ph0 = ascii::logo_lines_phase(size, 0);
        assert_eq!(stat.len(), ph0.len());
        for (a, b) in stat.iter().zip(ph0.iter()) {
            let sa: Vec<_> = a.spans.iter().map(|s| (s.content.clone(), s.style.fg)).collect();
            let sb: Vec<_> = b.spans.iter().map(|s| (s.content.clone(), s.style.fg)).collect();
            assert_eq!(sa, sb, "fase 0 diverge da logo estática em {size:?}");
        }
    }
}

#[test]
fn eye_pulses_across_phases_but_gears_stay_fixed() {

    let bronze = Color::Rgb(180, 140, 60);
    let core_color = |size, phase| -> Option<Color> {
        ascii::logo_lines_phase(size, phase)
            .into_iter()
            .flat_map(|l| l.spans)
            .find(|s| s.content.contains('O'))
            .and_then(|s| s.style.fg)
    };
    let c0 = core_color(LogoSize::Main, 0).unwrap();
    let c2 = core_color(LogoSize::Main, 2).unwrap();
    assert_ne!(c0, c2, "núcleo do olho não pulsou entre fases");

    for phase in 0..4u8 {
        for line in ascii::logo_lines_phase(LogoSize::Main, phase) {
            for span in &line.spans {
                if span.content.contains('#') {
                    assert_eq!(span.style.fg, Some(bronze), "engrenagem mudou na fase {phase}");
                }
            }
        }
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

    assert_eq!(ascii::select("auto", 999), Some(LogoSize::Main));

    let budget = LogoSize::Medium.width();
    assert_eq!(ascii::select("auto", budget), Some(LogoSize::Medium));

    let budget = LogoSize::Compact.width();
    assert_eq!(ascii::select("auto", budget), Some(LogoSize::Compact));

    assert_eq!(ascii::select("auto", LogoSize::Compact.width() - 1), None);
}
