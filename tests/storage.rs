//! Testes do Módulo 4 (Armazenamento/Discos): parsing puro, trava de
//! segurança `is_system_disk` e render da aba sem pânico.

use hal9001::app::{App, Tab};
use hal9001::backend::storage::{
    is_system_disk, parse_proc_mounts, parse_proc_swaps, BusType, DriveInfo, FsKind, PartitionInfo,
    StorageRow, StorageSnapshot,
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
