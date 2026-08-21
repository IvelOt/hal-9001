//! Testes do Módulo 4 (Armazenamento/Discos): parsing puro, trava de
//! segurança `is_system_disk` e render da aba sem pânico. Também cobre os
//! Épicos G (Formatador) e H (ISO Flasher): máquina de estados dos modais,
//! cálculo de velocidade/ETA e a invariante de segurança contra discos de
//! sistema.

use hal9001::app::{App, FlasherStage, FormatField, StorageModal, Tab};
use hal9001::backend::storage::{
    compute_speed_eta, is_system_disk, parse_proc_mounts, parse_proc_swaps, BusType, DriveInfo,
    FsKind, PartitionInfo, StorageRow, StorageSnapshot,
};
use hal9001::config::Config;
use hal9001::events::{Action, AppEvent, DeviceId};

fn drive(removable: bool, bus: BusType) -> DriveInfo {
    DriveInfo {
        id: DeviceId("/org/freedesktop/UDisks2/drives/test".into()),
        dev_node: "/dev/sda".into(),
        model: "Test Drive".into(),
        vendor: "ACME".into(),
        size: 512 * 1024 * 1024 * 1024,
        removable,
        ejectable: removable,
        bus,
        rotational: false,
        is_system: false,
        partitions: Vec::new(),
    }
}

fn partition(mount_points: Vec<&str>, is_swap: bool) -> PartitionInfo {
    PartitionInfo {
        id: DeviceId("/org/freedesktop/UDisks2/block_devices/sda1".into()),
        dev_node: "/dev/sda1".into(),
        label: "root".into(),
        fs: FsKind::Ext4,
        size: 100 * 1024 * 1024 * 1024,
        used: Some(40 * 1024 * 1024 * 1024),
        mount_points: mount_points.into_iter().map(String::from).collect(),
        is_swap,
        is_system: false,
    }
}

// ---------------------------------------------------------------------------
// Trava de segurança `is_system_disk` — invariante inegociável.
// ---------------------------------------------------------------------------

#[test]
fn root_mount_is_always_system() {
    // Mesmo tentando "burlar" via removable=true + bus USB, uma partição
    // montada em `/` NUNCA pode deixar de ser marcada como sistema.
    let d = drive(true, BusType::Usb);
    let p = partition(vec!["/"], false);
    assert!(is_system_disk(&d, &p));
}

#[test]
fn boot_and_boot_efi_and_home_are_system() {
    let d = drive(true, BusType::Usb);
    for mp in ["/boot", "/boot/efi", "/home"] {
        let p = partition(vec![mp], false);
        assert!(
            is_system_disk(&d, &p),
            "mountpoint {mp} deveria ser sistema"
        );
    }
}

#[test]
fn trailing_slash_mount_still_matches_root() {
    let d = drive(true, BusType::Usb);
    let p = partition(vec!["//"], false);
    assert!(is_system_disk(&d, &p));
}

#[test]
fn active_swap_partition_is_system() {
    let d = drive(true, BusType::Usb);
    let p = partition(vec![], true);
    assert!(is_system_disk(&d, &p));
}

#[test]
fn fixed_internal_drive_is_system_even_without_protected_mount() {
    // Heurística conservadora: disco interno fixo (não removível, não-USB)
    // nunca é alvo por padrão, mesmo sem nenhuma partição montada.
    let d = drive(false, BusType::Sata);
    let p = partition(vec![], false);
    assert!(is_system_disk(&d, &p));
}

#[test]
fn removable_usb_partition_unmounted_is_not_system() {
    let d = drive(true, BusType::Usb);
    let p = partition(vec![], false);
    assert!(!is_system_disk(&d, &p));
}

#[test]
fn removable_usb_partition_mounted_elsewhere_is_not_system() {
    let d = drive(true, BusType::Usb);
    let p = partition(vec!["/run/media/user/KINGSTON"], false);
    assert!(!is_system_disk(&d, &p));
}

// ---------------------------------------------------------------------------
// Parsers puros
// ---------------------------------------------------------------------------

#[test]
fn parse_swaps_extracts_partition_devices_only() {
    let text = "Filename\t\t\t\tType\t\tSize\t\tUsed\tPriority\n\
                /dev/sda2                              partition\t8388604\t0\t-2\n\
                /swapfile                              file    \t2097148\t0\t-3\n";
    let swaps = parse_proc_swaps(text);
    assert_eq!(swaps, vec!["/dev/sda2".to_string()]);
}

#[test]
fn parse_mounts_extracts_device_and_mountpoint_pairs() {
    let text = "/dev/sda1 / ext4 rw,relatime 0 0\n\
                /dev/sdb1 /run/media/user/My\\040Drive vfat rw,nosuid 0 0\n\
                tmpfs /run tmpfs rw 0 0\n";
    let mounts = parse_proc_mounts(text);
    assert_eq!(
        mounts,
        vec![
            ("/dev/sda1".to_string(), "/".to_string()),
            (
                "/dev/sdb1".to_string(),
                "/run/media/user/My Drive".to_string()
            ),
        ]
    );
}

// ---------------------------------------------------------------------------
// StorageSnapshot / navegação achatada
// ---------------------------------------------------------------------------

fn mock_snapshot() -> StorageSnapshot {
    let mut system_drive = drive(false, BusType::Sata);
    system_drive.id = DeviceId("/drives/system".into());
    let mut root_part = partition(vec!["/"], false);
    root_part.is_system = true;
    system_drive.partitions = vec![root_part];
    system_drive.is_system = true;

    let mut usb_drive = drive(true, BusType::Usb);
    usb_drive.id = DeviceId("/drives/usb".into());
    usb_drive.dev_node = "/dev/sdb".into();
    let usb_part = partition(vec!["/run/media/user/USB"], false);
    usb_drive.partitions = vec![usb_part];
    usb_drive.is_system = false;

    StorageSnapshot {
        udisks_available: true,
        drives: vec![system_drive, usb_drive],
    }
}

#[test]
fn rows_flatten_drives_and_partitions_in_order() {
    let snap = mock_snapshot();
    let rows = snap.rows();
    assert_eq!(
        rows,
        vec![
            StorageRow::Drive(0),
            StorageRow::Partition(0, 0),
            StorageRow::Drive(1),
            StorageRow::Partition(1, 0),
        ]
    );
}

#[test]
fn system_disk_never_reachable_for_eject_via_app_dispatch() {
    // Invariante de teste inegociável: um disco com `/` montado nunca pode
    // ser selecionado como alvo de uma ação destrutiva/de ejeção — o `App`
    // recusa e emite um toast de erro em vez de despachar `StorageEject`.
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));
    app.active = Tab::Storage;
    app.storage_selected = 0; // linha do drive de sistema.

    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageEjectSelected, &tx);

    // Nenhuma ação chegou ao backend.
    assert!(rx.try_recv().is_err());
    // E o usuário foi avisado via toast.
    assert!(app.toast.is_some());
}

#[test]
fn non_system_drive_ejects_through_dispatch() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));
    app.active = Tab::Storage;
    app.storage_selected = 2; // linha do drive USB (não-sistema).

    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageEjectSelected, &tx);

    match rx.try_recv() {
        Ok(Action::StorageEject(id)) => assert_eq!(id, DeviceId("/drives/usb".into())),
        other => panic!("esperava StorageEject(usb), obteve {other:?}"),
    }
}

#[test]
fn mount_toggle_sends_unmount_when_already_mounted() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));
    app.active = Tab::Storage;
    app.storage_selected = 3; // partição do drive USB, montada.

    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageMountToggleSelected, &tx);

    match rx.try_recv() {
        Ok(Action::StorageUnmount(_)) => {}
        other => panic!("esperava StorageUnmount, obteve {other:?}"),
    }
}

#[test]
fn storage_selection_clamps_to_row_count_after_shrink() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));
    app.storage_selected = 999; // fora dos limites.
    assert_eq!(app.storage_row(), Some(StorageRow::Partition(1, 0)));
}

// ---------------------------------------------------------------------------
// Render sem pânico (degradado e com snapshot populado).
// ---------------------------------------------------------------------------

#[test]
fn render_storage_tab_degraded_without_snapshot() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.active = Tab::Storage;

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
}

#[test]
fn render_storage_tab_with_snapshot_and_selection_without_panic() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.active = Tab::Storage;
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    for row in 0..6 {
        app.storage_selected = row;
        terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
    }
}

#[test]
fn render_storage_tab_empty_drives_without_panic() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.active = Tab::Storage;
    app.handle_event(AppEvent::Storage(Box::new(StorageSnapshot {
        udisks_available: true,
        drives: vec![],
    })));

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
}

// ---------------------------------------------------------------------------
// Épico G/H: cálculo puro de velocidade/ETA e helpers de snapshot
// ---------------------------------------------------------------------------

#[test]
fn compute_speed_eta_reports_zero_when_no_bytes_transferred() {
    let (speed, eta) = compute_speed_eta(0, 1.0, 0, 100);
    assert_eq!(speed, 0.0);
    assert_eq!(eta, 0);
}

#[test]
fn compute_speed_eta_computes_rate_and_remaining_time() {
    // 4 MiB em 1s de janela == 4 MB/s; restam 16 MiB de um total de 20 MiB
    // escritos até agora (4 MiB) => ETA = 16 / 4 = 4s.
    let window_bytes = 4 * 1024 * 1024;
    let total = 20 * 1024 * 1024;
    let written = 4 * 1024 * 1024;
    let (speed, eta) = compute_speed_eta(window_bytes, 1.0, written, total);
    assert!((speed - 4.0).abs() < 0.01, "esperava ~4.0 MB/s, obteve {speed}");
    assert_eq!(eta, 4);
}

#[test]
fn compute_speed_eta_never_divides_by_a_zero_window() {
    // Janela de tempo "instantânea" (0s) não deve gerar pânico/infinito.
    let (speed, _eta) = compute_speed_eta(1024, 0.0, 0, 1024 * 1024);
    assert!(speed.is_finite());
}

fn usb_target_snapshot(dev_node: &str, size: u64) -> StorageSnapshot {
    let mut d = drive(true, BusType::Usb);
    d.id = DeviceId("/drives/usb-target".into());
    d.dev_node = dev_node.to_string();
    d.size = size;
    d.is_system = false;
    d.partitions = Vec::new();
    StorageSnapshot {
        udisks_available: true,
        drives: vec![d],
    }
}

fn app_with_usb_target(dev_node: &str, size: u64) -> App {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.active = Tab::Storage;
    app.handle_event(AppEvent::Storage(Box::new(usb_target_snapshot(
        dev_node, size,
    ))));
    app.storage_selected = 0; // única linha: o drive USB.
    app
}

// ---------------------------------------------------------------------------
// Épico G — máquina de estados do modal de formatação
// ---------------------------------------------------------------------------

#[test]
fn format_open_is_refused_for_system_disk_and_no_modal_opens() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.active = Tab::Storage;
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));
    app.storage_selected = 0; // drive de sistema.

    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFormatOpen, &tx);

    assert!(matches!(app.storage_modal, StorageModal::None));
    assert!(app.toast.is_some());
    assert!(rx.try_recv().is_err());
}

#[test]
fn format_modal_opens_for_non_system_drive_defaulting_to_pendrive_label() {
    let app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let mut app = app;
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFormatOpen, &tx);

    match &app.storage_modal {
        StorageModal::Format(s) => {
            assert_eq!(s.label, "PENDRIVE");
            assert_eq!(s.field, FormatField::Fs);
        }
        other => panic!("esperava modal de formatação, obteve {other:?}"),
    }
}

#[test]
fn format_modal_cycles_fs_edits_label_and_sends_action_on_confirm() {
    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFormatOpen, &tx);

    // Vfat (padrão) -> Exfat -> Ext4 avançando com `Right` duas vezes.
    app.dispatch(Action::Right, &tx);
    app.dispatch(Action::Right, &tx);

    // Move para o campo de rótulo, apaga o padrão e digita um novo.
    app.dispatch(Action::Down, &tx);
    for _ in 0.."PENDRIVE".len() {
        app.dispatch(Action::StorageModalBackspace, &tx);
    }
    for c in "MEUPEN".chars() {
        app.dispatch(Action::StorageModalChar(c), &tx);
    }

    // Avança para o campo de confirmação e confirma.
    app.dispatch(Action::Enter, &tx);
    app.dispatch(Action::Enter, &tx);

    assert!(matches!(app.storage_modal, StorageModal::None));
    match rx.try_recv() {
        Ok(Action::StorageFormat {
            device_id,
            fs_type,
            label,
        }) => {
            assert_eq!(device_id, "/drives/usb-target");
            assert_eq!(fs_type, "ext4");
            assert_eq!(label, "MEUPEN");
        }
        other => panic!("esperava StorageFormat, obteve {other:?}"),
    }
}

#[test]
fn format_modal_esc_closes_without_sending_action() {
    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFormatOpen, &tx);
    assert!(app.storage_modal_open());

    // `Esc` chega ao `App` como `Action::ToggleConfig` (ver `events/input.rs`).
    app.dispatch(Action::ToggleConfig, &tx);

    assert!(matches!(app.storage_modal, StorageModal::None));
    assert!(rx.try_recv().is_err());
}

// ---------------------------------------------------------------------------
// Épico H — máquina de estados do wizard do ISO Flasher
// ---------------------------------------------------------------------------

#[test]
fn flasher_open_is_refused_for_system_disk() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.active = Tab::Storage;
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));
    app.storage_selected = 0; // drive de sistema.

    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFlasherOpen, &tx);

    assert!(matches!(app.storage_modal, StorageModal::None));
    assert!(app.toast.is_some());
}

fn type_path(app: &mut App, tx: &tokio::sync::broadcast::Sender<Action>, path: &str) {
    for c in path.chars() {
        app.dispatch(Action::StorageModalChar(c), tx);
    }
}

#[test]
fn flasher_rejects_iso_larger_than_target_capacity() {
    let iso = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(iso.path(), vec![0u8; 4096]).unwrap();

    // Alvo com capacidade menor que o arquivo (4096 bytes) força o erro de
    // tamanho antes mesmo de qualquer I/O de gravação.
    let mut app = app_with_usb_target("/dev/sdz", 1024);
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFlasherOpen, &tx);

    type_path(&mut app, &tx, iso.path().to_str().unwrap());
    app.dispatch(Action::Enter, &tx);

    match &app.storage_modal {
        StorageModal::Flasher(s) => match &s.stage {
            FlasherStage::SelectIso { error, .. } => {
                assert!(error.is_some(), "esperava erro de tamanho, obteve None");
            }
            other => panic!("esperava continuar em SelectIso com erro, obteve {other:?}"),
        },
        other => panic!("esperava modal do flasher, obteve {other:?}"),
    }
}

#[test]
fn flasher_full_wizard_reaches_flashing_only_after_typed_confirmation_matches() {
    let iso = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(iso.path(), vec![0xABu8; 4096]).unwrap();

    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFlasherOpen, &tx);

    type_path(&mut app, &tx, iso.path().to_str().unwrap());
    app.dispatch(Action::Enter, &tx); // SelectIso -> Ready
    match &app.storage_modal {
        StorageModal::Flasher(s) => assert!(matches!(s.stage, FlasherStage::Ready { .. })),
        other => panic!("esperava Ready, obteve {other:?}"),
    }

    app.dispatch(Action::Enter, &tx); // Ready -> Confirm1
    app.dispatch(Action::Enter, &tx); // Confirm1 -> Confirm2

    // Confirmação digitada incorreta: NÃO deve iniciar a gravação.
    type_path(&mut app, &tx, "/dev/wrong");
    app.dispatch(Action::Enter, &tx);
    assert!(rx.try_recv().is_err(), "gravação não deveria ter iniciado");
    match &app.storage_modal {
        StorageModal::Flasher(s) => assert!(matches!(s.stage, FlasherStage::Confirm2 { .. })),
        other => panic!("esperava permanecer em Confirm2, obteve {other:?}"),
    }

    // Limpa o campo digitado e confirma com o nó correto.
    for _ in 0.."/dev/wrong".len() {
        app.dispatch(Action::StorageModalBackspace, &tx);
    }
    type_path(&mut app, &tx, "/dev/sdz");
    app.dispatch(Action::Enter, &tx);

    match rx.try_recv() {
        Ok(Action::StorageFlashIso { device_id, iso_path }) => {
            assert_eq!(device_id, "/drives/usb-target");
            assert_eq!(iso_path, iso.path().to_str().unwrap());
        }
        other => panic!("esperava StorageFlashIso, obteve {other:?}"),
    }
    match &app.storage_modal {
        StorageModal::Flasher(s) => assert!(matches!(s.stage, FlasherStage::Flashing { .. })),
        other => panic!("esperava Flashing, obteve {other:?}"),
    }
}

#[test]
fn flasher_esc_during_flash_sends_cancel_and_closes_modal() {
    let iso = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(iso.path(), vec![0xABu8; 4096]).unwrap();

    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFlasherOpen, &tx);
    type_path(&mut app, &tx, iso.path().to_str().unwrap());
    app.dispatch(Action::Enter, &tx); // -> Ready
    app.dispatch(Action::Enter, &tx); // -> Confirm1
    app.dispatch(Action::Enter, &tx); // -> Confirm2
    type_path(&mut app, &tx, "/dev/sdz");
    app.dispatch(Action::Enter, &tx); // -> Flashing (envia StorageFlashIso)
    let _ = rx.try_recv();

    app.dispatch(Action::ToggleConfig, &tx); // Esc

    assert!(matches!(app.storage_modal, StorageModal::None));
    match rx.try_recv() {
        Ok(Action::StorageFlashCancel { device_id }) => {
            assert_eq!(device_id, "/drives/usb-target");
        }
        other => panic!("esperava StorageFlashCancel, obteve {other:?}"),
    }
}

#[test]
fn flash_progress_event_updates_flashing_stage_fields() {
    let iso = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(iso.path(), vec![0xABu8; 4096]).unwrap();

    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFlasherOpen, &tx);
    type_path(&mut app, &tx, iso.path().to_str().unwrap());
    app.dispatch(Action::Enter, &tx);
    app.dispatch(Action::Enter, &tx);
    app.dispatch(Action::Enter, &tx);
    type_path(&mut app, &tx, "/dev/sdz");
    app.dispatch(Action::Enter, &tx);

    app.handle_event(AppEvent::StorageFlashProgress {
        bytes_written: 2048,
        total_bytes: 4096,
        speed_mbps: 12.5,
        eta_secs: 3,
    });

    match &app.storage_modal {
        StorageModal::Flasher(s) => match &s.stage {
            FlasherStage::Flashing {
                bytes_written,
                total_bytes,
                speed_mbps,
                eta_secs,
            } => {
                assert_eq!(*bytes_written, 2048);
                assert_eq!(*total_bytes, 4096);
                assert!((*speed_mbps - 12.5).abs() < f64::EPSILON);
                assert_eq!(*eta_secs, 3);
            }
            other => panic!("esperava Flashing, obteve {other:?}"),
        },
        other => panic!("esperava modal do flasher, obteve {other:?}"),
    }
}

#[test]
fn flash_done_event_transitions_to_done_stage_with_result() {
    let iso = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(iso.path(), vec![0xABu8; 4096]).unwrap();

    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFlasherOpen, &tx);
    type_path(&mut app, &tx, iso.path().to_str().unwrap());
    app.dispatch(Action::Enter, &tx);
    app.dispatch(Action::Enter, &tx);
    app.dispatch(Action::Enter, &tx);
    type_path(&mut app, &tx, "/dev/sdz");
    app.dispatch(Action::Enter, &tx);

    app.handle_event(AppEvent::StorageFlashDone {
        device_id: "/drives/usb-target".to_string(),
        result: Ok("gravação concluída com sucesso".to_string()),
    });

    match &app.storage_modal {
        StorageModal::Flasher(s) => match &s.stage {
            FlasherStage::Done { ok, .. } => assert!(*ok),
            other => panic!("esperava Done, obteve {other:?}"),
        },
        other => panic!("esperava modal do flasher, obteve {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// Invariante de segurança: nenhuma ação destrutiva chega ao backend para um
// alvo `is_system == true`, mesmo tentando abrir os modais diretamente.
// ---------------------------------------------------------------------------

#[test]
fn system_disk_never_reachable_for_format_or_flash_via_app_dispatch() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.active = Tab::Storage;
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));
    app.storage_selected = 0; // linha do drive de sistema.

    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFormatOpen, &tx);
    app.dispatch(Action::StorageFlasherOpen, &tx);

    assert!(matches!(app.storage_modal, StorageModal::None));
    assert!(rx.try_recv().is_err());
}

// ---------------------------------------------------------------------------
// Render dos modais sem pânico.
// ---------------------------------------------------------------------------

#[test]
fn render_format_and_flasher_modals_without_panic() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

    app.dispatch(Action::StorageFormatOpen, &tx);
    terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
    app.dispatch(Action::ToggleConfig, &tx); // fecha

    app.dispatch(Action::StorageFlasherOpen, &tx);
    terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
}
