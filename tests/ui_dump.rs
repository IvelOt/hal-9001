use hal9001::app::{App, Phase};
use hal9001::config::Config;
use hal9001::i18n::Language;
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use std::fs;

#[test]
fn dump_config_modal() {
    let _ = fs::create_dir_all("/tmp/no-mistakes-evidence/01M11Y7QRDNSPANV57H68T9GMD");

    for lang in [Language::PtBr, Language::EnUs, Language::EsEs] {
        let mut cfg = Config::default();
        cfg.ui.language = lang.code().to_string();
        let mut app = App::new(cfg);
        app.phase = Phase::Running;
        app.show_config = true;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();

        let buf = terminal.backend().buffer().clone();

        let mut out = String::new();
        for y in 0..24 {
            for x in 0..80 {
                out.push_str(buf.cell((x, y)).unwrap().symbol());
            }
            out.push('\n');
        }

        let filename = format!(
            "/tmp/no-mistakes-evidence/01M11Y7QRDNSPANV57H68T9GMD/config_modal_{}.txt",
            lang.code()
        );
        fs::write(filename, out).unwrap();
    }
}

#[test]
fn dump_overview_tab() {
    let _ = fs::create_dir_all("/tmp/no-mistakes-evidence/01M11Y7QRDNSPANV57H68T9GMD");

    for lang in [Language::PtBr, Language::EnUs, Language::EsEs] {
        let mut cfg = Config::default();
        cfg.ui.language = lang.code().to_string();
        let mut app = App::new(cfg);
        app.phase = Phase::Running;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();

        let buf = terminal.backend().buffer().clone();

        let mut out = String::new();
        for y in 0..24 {
            for x in 0..80 {
                out.push_str(buf.cell((x, y)).unwrap().symbol());
            }
            out.push('\n');
        }

        let filename = format!(
            "/tmp/no-mistakes-evidence/01M11Y7QRDNSPANV57H68T9GMD/overview_tab_{}.txt",
            lang.code()
        );
        fs::write(filename, out).unwrap();
    }
}
