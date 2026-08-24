//! Testes do Módulo 7/8 (PTY: Gerenciador de Arquivos via Yazi & Terminal Deck).

use hal9001::app::{App, PtyState, Tab};
use hal9001::backend::pty::find_in_path_var;
use hal9001::config::Config;
use hal9001::events::input::key_to_pty_bytes;
use hal9001::events::{Action, AppEvent, PtyCell, PtyColor, PtyScreenSnapshot, PtyTarget};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

// ---------------------------------------------------------------------------
// find_in_path_var — varredura pura de $PATH (sem shell-out)
// ---------------------------------------------------------------------------

#[test]
fn find_in_path_var_locates_executable_binary() {
    let dir = tempfile::tempdir().unwrap();
    let bin_path = dir.path().join("yazi");
    std::fs::write(&bin_path, b"#!/bin/sh\necho yazi\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&bin_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin_path, perms).unwrap();
    }

    let path_var = std::ffi::OsString::from(dir.path());
    let found = find_in_path_var("yazi", &path_var);
    assert_eq!(found, Some(bin_path));
}

#[test]
fn find_in_path_var_ignores_non_executable_file() {
    let dir = tempfile::tempdir().unwrap();
    let bin_path = dir.path().join("yazi");
    std::fs::write(&bin_path, b"not executable").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&bin_path).unwrap().permissions();
        perms.set_mode(0o644);
        std::fs::set_permissions(&bin_path, perms).unwrap();
    }

    let path_var = std::ffi::OsString::from(dir.path());
    assert_eq!(find_in_path_var("yazi", &path_var), None);
}

#[test]
fn find_in_path_var_returns_none_when_missing() {
    let dir = tempfile::tempdir().unwrap();
    let path_var = std::ffi::OsString::from(dir.path());
    assert_eq!(find_in_path_var("yazi", &path_var), None);
}

#[test]
fn find_in_path_var_scans_multiple_dirs_in_order() {
    let dir_a = tempfile::tempdir().unwrap();
    let dir_b = tempfile::tempdir().unwrap();
    let bin_b = dir_b.path().join("yazi");
    std::fs::write(&bin_b, b"#!/bin/sh\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&bin_b).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&bin_b, perms).unwrap();
    }

    let path_var = std::env::join_paths([dir_a.path(), dir_b.path()]).unwrap();
    assert_eq!(find_in_path_var("yazi", &path_var), Some(bin_b));
}

// ---------------------------------------------------------------------------
// key_to_pty_bytes — codificação VT100/xterm de teclas
// ---------------------------------------------------------------------------

fn key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
}

fn ctrl_key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::CONTROL)
}

#[test]
fn key_to_pty_bytes_printable_char() {
    use crossterm::event::KeyCode;
    assert_eq!(key_to_pty_bytes(key(KeyCode::Char('a'))), b"a".to_vec());
}

#[test]
fn key_to_pty_bytes_enter_backspace_tab() {
    use crossterm::event::KeyCode;
    assert_eq!(key_to_pty_bytes(key(KeyCode::Enter)), vec![b'\r']);
    assert_eq!(key_to_pty_bytes(key(KeyCode::Backspace)), vec![0x7f]);
    assert_eq!(key_to_pty_bytes(key(KeyCode::Tab)), vec![b'\t']);
}

#[test]
fn key_to_pty_bytes_arrows_are_csi_sequences() {
    use crossterm::event::KeyCode;
    assert_eq!(key_to_pty_bytes(key(KeyCode::Up)), b"\x1b[A".to_vec());
    assert_eq!(key_to_pty_bytes(key(KeyCode::Down)), b"\x1b[B".to_vec());
    assert_eq!(key_to_pty_bytes(key(KeyCode::Right)), b"\x1b[C".to_vec());
    assert_eq!(key_to_pty_bytes(key(KeyCode::Left)), b"\x1b[D".to_vec());
}

#[test]
fn key_to_pty_bytes_ctrl_letter_is_control_byte() {
    use crossterm::event::KeyCode;
    // Ctrl-A -> 0x01, Ctrl-C -> 0x03 (intercept do Quit acontece antes, em
    // map_key; aqui testamos só a codificação de baixo nível).
    assert_eq!(key_to_pty_bytes(ctrl_key(KeyCode::Char('a'))), vec![0x01]);
    assert_eq!(key_to_pty_bytes(ctrl_key(KeyCode::Char('c'))), vec![0x03]);
}

// ---------------------------------------------------------------------------
// App::sync_pty_size — diffing de resize
// ---------------------------------------------------------------------------

#[test]
fn sync_pty_size_no_action_when_unchanged() {
    let mut app = App::new(Config::default());
    let first = app.sync_pty_size(100, 40);
    assert_eq!(first.len(), 2);
    let second = app.sync_pty_size(100, 40);
    assert!(second.is_empty(), "não deveria reemitir resize sem mudança de tamanho");
}

#[test]
fn sync_pty_size_emits_resize_pair_on_change() {
    let mut app = App::new(Config::default());
    let _ = app.sync_pty_size(100, 40);
    let resized = app.sync_pty_size(120, 50);
    assert_eq!(resized.len(), 2);
    for action in &resized {
        match action {
            Action::PtyResize { cols, rows, .. } => {
                assert!(*cols > 0);
                assert!(*rows > 0);
            }
            other => panic!("esperava Action::PtyResize, obteve {other:?}"),
        }
    }
    let targets: Vec<_> = resized
        .iter()
        .map(|a| match a {
            Action::PtyResize { target, .. } => *target,
            _ => unreachable!(),
        })
        .collect();
    assert!(targets.contains(&PtyTarget::Files));
    assert!(targets.contains(&PtyTarget::Terminal));
}

#[test]
fn sync_pty_size_never_underflows_on_tiny_terminal() {
    let mut app = App::new(Config::default());
    let resized = app.sync_pty_size(1, 1);
    for action in resized {
        if let Action::PtyResize { cols, rows, .. } = action {
            assert!(cols >= 1);
            assert!(rows >= 1);
        }
    }
}

// ---------------------------------------------------------------------------
// Render sem pânico em cada PtyState, para Arquivos e Terminal
// ---------------------------------------------------------------------------

fn fixture_screen() -> PtyScreenSnapshot {
    let normal = PtyCell {
        ch: 'x',
        fg: PtyColor::Indexed(2),
        bg: PtyColor::Rgb(10, 10, 10),
        bold: true,
        underline: true,
        inverse: false,
        italic: true,
    };
    PtyScreenSnapshot {
        cols: 10,
        rows: 3,
        cells: vec![vec![normal; 10]; 3],
        cursor: (1, 1),
        cursor_visible: true,
    }
}

#[test]
fn render_files_and_terminal_in_every_pty_state_without_panic() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    let mut terminal = Terminal::new(TestBackend::new(100, 40)).unwrap();
    let (tx, _rx) = tokio::sync::broadcast::channel(8);

    for tab in [Tab::Files, Tab::Terminal] {
        app.dispatch(Action::SelectTab(tab.index()), &tx);

        // Starting (estado inicial de App::new).
        terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();

        // Unavailable.
        let target = if tab == Tab::Files {
            PtyTarget::Files
        } else {
            PtyTarget::Terminal
        };
        app.handle_event(AppEvent::PtyUnavailable {
            target,
            reason: "não encontrado no teste".to_string(),
        });
        terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();

        // Running, com foco desligado e ligado (cursor visível em ambos).
        app.handle_event(AppEvent::PtyScreenUpdate {
            target,
            screen: Box::new(fixture_screen()),
        });
        terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
        app.dispatch(Action::PtyFocus, &tx);
        assert!(app.pty_focused());
        terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();

        // Exited.
        app.handle_event(AppEvent::PtyExited { target });
        assert!(!app.pty_focused());
        terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
        assert!(matches!(
            if tab == Tab::Files {
                &app.files_pty
            } else {
                &app.terminal_pty
            },
            PtyState::Exited
        ));
    }
}

#[test]
fn pty_focus_unfocus_cycle_via_dispatch() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.dispatch(Action::SelectTab(Tab::Terminal.index()), &tokio::sync::broadcast::channel(8).0);
    let (tx, _rx) = tokio::sync::broadcast::channel(8);

    // Sem sessão rodando ainda: foco não é concedido.
    app.dispatch(Action::PtyFocus, &tx);
    assert!(!app.pty_focused());

    app.handle_event(AppEvent::PtyScreenUpdate {
        target: PtyTarget::Terminal,
        screen: Box::new(fixture_screen()),
    });
    app.dispatch(Action::PtyFocus, &tx);
    assert!(app.pty_focused());

    // Bytes digitados enquanto focado são apenas repassados ao backend (não
    // mutam o estado local do App) e não entram em pânico.
    app.dispatch(
        Action::PtyInput {
            target: PtyTarget::Terminal,
            bytes: b"ls\r".to_vec(),
        },
        &tx,
    );
    assert!(app.pty_focused());

    app.dispatch(Action::PtyUnfocus, &tx);
    assert!(!app.pty_focused());

    // Trocar de aba também derruba o foco.
    app.handle_event(AppEvent::PtyScreenUpdate {
        target: PtyTarget::Terminal,
        screen: Box::new(fixture_screen()),
    });
    app.dispatch(Action::PtyFocus, &tx);
    assert!(app.pty_focused());
    app.dispatch(Action::SelectTab(Tab::Overview.index()), &tx);
    assert!(!app.pty_focused());
}
