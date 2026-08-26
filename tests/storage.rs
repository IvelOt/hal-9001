
use hal9001::app::{App, DiskAnalyzerState, FlasherStage, FormatField, StorageModal, Tab};
use hal9001::backend::storage::{
    build_ventoy_entries, compute_speed_eta, detect_ventoy, format_fat32_pure_rust,
    gzip_uncompressed_size_hint, is_gzip_file, is_iso_or_img, is_no_usb_device_error,
    is_not_authorized_error, is_permission_denied_error, is_sudo_auth_failure, is_system_disk,
    mkfs_command, parse_dd_bytes_copied, parse_proc_mounts, parse_proc_swaps, primary_partition,
    resolve_block_object_path, skips_power_off, sudo_invocation, ventoy_data_partition, BusType,
    DriveInfo, FsKind, PartitionInfo, StorageSnapshot,
};
use hal9001::config::Config;
use hal9001::events::{Action, AppEvent, DeviceId, SudoPasswordRequest};

fn drive(removable: bool, bus: BusType) -> DriveInfo {
    DriveInfo {
        id: DeviceId("/org/freedesktop/UDisks2/drives/test".into()),
        dev_node: "/dev/sda".into(),
        block_path: Some("/org/freedesktop/UDisks2/block_devices/sda".into()),
        model: "Test Drive".into(),
        vendor: "ACME".into(),
        size: 512 * 1024 * 1024 * 1024,
        removable,
        ejectable: removable,
        can_power_off: bus != BusType::Mmc,
        bus,
        rotational: false,
        is_system: false,
        is_ventoy: false,
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

#[test]
fn root_mount_is_always_system() {

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
fn drive_only_list_has_one_row_per_drive() {

    let snap = mock_snapshot();
    assert_eq!(snap.drives.len(), 2);
}

#[test]
fn primary_partition_prefers_mounted_non_system() {
    let snap = mock_snapshot();
    let usb = &snap.drives[1];
    let p = primary_partition(usb).expect("deveria resolver a partição primária");
    assert_eq!(p.mount_points, vec!["/run/media/user/USB".to_string()]);
}

#[test]
fn system_disk_never_reachable_for_eject_via_app_dispatch() {

    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));
    app.active = Tab::Storage;
    app.storage_selected = 0;

    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageEjectSelected, &tx);

    assert!(rx.try_recv().is_err());

    assert!(app.toast.is_some());
}

#[test]
fn non_system_drive_ejects_through_dispatch() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));
    app.active = Tab::Storage;
    app.storage_selected = 1;

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
    app.storage_selected = 1;

    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageMountToggleSelected, &tx);

    match rx.try_recv() {
        Ok(Action::StorageUnmount(_)) => {}
        other => panic!("esperava StorageUnmount, obteve {other:?}"),
    }
}

#[test]
fn storage_selection_clamps_to_drive_count_after_shrink() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));
    app.storage_selected = 999;
    assert_eq!(app.storage_drive_index(), Some(1));
}

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

#[test]
fn compute_speed_eta_reports_zero_when_no_bytes_transferred() {
    let (speed, eta) = compute_speed_eta(0, 1.0, 0, 100);
    assert_eq!(speed, 0.0);
    assert_eq!(eta, 0);
}

#[test]
fn compute_speed_eta_computes_rate_and_remaining_time() {

    let window_bytes = 4 * 1024 * 1024;
    let total = 20 * 1024 * 1024;
    let written = 4 * 1024 * 1024;
    let (speed, eta) = compute_speed_eta(window_bytes, 1.0, written, total);
    assert!(
        (speed - 4.0).abs() < 0.01,
        "esperava ~4.0 MB/s, obteve {speed}"
    );
    assert_eq!(eta, 4);
}

#[test]
fn compute_speed_eta_never_divides_by_a_zero_window() {

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
    app.storage_selected = 0;
    app
}

#[test]
fn resolve_block_object_path_converts_drive_path_to_its_block_device_path() {
    let mut d = drive(true, BusType::Usb);
    d.id = DeviceId("/org/freedesktop/UDisks2/drives/Kingston_1234".into());
    d.dev_node = "/dev/sdz".into();
    d.block_path = Some("/org/freedesktop/UDisks2/block_devices/sdz".into());
    let snap = StorageSnapshot {
        udisks_available: true,
        drives: vec![d],
    };

    let resolved = resolve_block_object_path(
        &snap,
        &DeviceId("/org/freedesktop/UDisks2/drives/Kingston_1234".into()),
    )
    .expect("esperava resolver o bloco raiz do drive");

    assert_eq!(resolved, "/org/freedesktop/UDisks2/block_devices/sdz");

    assert!(resolved.starts_with("/org/freedesktop/UDisks2/block_devices/"));
}

#[test]
fn resolve_block_object_path_passes_through_an_already_resolved_block_device_path() {
    let mut d = drive(true, BusType::Usb);
    d.id = DeviceId("/org/freedesktop/UDisks2/drives/Kingston_1234".into());
    d.block_path = Some("/org/freedesktop/UDisks2/block_devices/sdz".into());
    let part = partition(vec![], false);
    let part_id = part.id.clone();
    d.partitions = vec![part];
    let snap = StorageSnapshot {
        udisks_available: true,
        drives: vec![d],
    };

    let resolved = resolve_block_object_path(&snap, &part_id).expect("esperava o próprio caminho");
    assert_eq!(resolved, part_id.0);
}

#[test]
fn resolve_block_object_path_returns_none_for_a_drive_with_unknown_block_path() {
    let mut d = drive(true, BusType::Usb);
    d.id = DeviceId("/org/freedesktop/UDisks2/drives/no_block_known".into());
    d.block_path = None;
    let snap = StorageSnapshot {
        udisks_available: true,
        drives: vec![d],
    };

    assert_eq!(
        resolve_block_object_path(
            &snap,
            &DeviceId("/org/freedesktop/UDisks2/drives/no_block_known".into())
        ),
        None
    );
}

#[test]
fn resolve_block_object_path_never_resolves_a_drive_id_to_itself() {

    let mut d = drive(true, BusType::Usb);
    d.id = DeviceId("/org/freedesktop/UDisks2/drives/anything".into());
    d.block_path = Some("/org/freedesktop/UDisks2/block_devices/sdq".into());
    let snap = StorageSnapshot {
        udisks_available: true,
        drives: vec![d.clone()],
    };

    let resolved = resolve_block_object_path(&snap, &d.id);
    assert_ne!(resolved.as_deref(), Some(d.id.0.as_str()));
}

#[test]
fn is_permission_denied_error_detects_io_permission_denied() {
    let io_err = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");
    let err: anyhow::Error = io_err.into();
    assert!(is_permission_denied_error(&err));
}

#[test]
fn is_permission_denied_error_detects_message_text_fallback() {
    let err = anyhow::anyhow!("dd: failed to open '/dev/sdz': Permission denied");
    assert!(is_permission_denied_error(&err));
}

#[test]
fn is_permission_denied_error_is_false_for_unrelated_errors() {
    let err = anyhow::anyhow!("dispositivo não encontrado");
    assert!(!is_permission_denied_error(&err));
}

#[test]
fn is_not_authorized_error_detects_polkit_refusal_variants() {
    for msg in [
        "GDBus.Error:org.freedesktop.PolicyKit1.Error.NotAuthorized: Not authorized",
        "No polkit agent available to authenticate",
        "Authentication is required to format the device",
    ] {
        let err = anyhow::anyhow!(msg.to_string());
        assert!(is_not_authorized_error(&err), "esperava match para: {msg}");
    }
}

#[test]
fn is_not_authorized_error_is_false_for_missing_mkfs() {
    let err = anyhow::anyhow!("mkfs.vfat: command not found");
    assert!(!is_not_authorized_error(&err));
}

#[test]
fn mkfs_command_builds_vfat_args_with_label_and_fat32_flag() {
    let (bin, args) = mkfs_command("vfat", "PENDRIVE", "/dev/sdz1").expect("vfat mapeado");
    assert_eq!(bin, "mkfs.vfat");
    assert_eq!(args, vec!["-F", "32", "-n", "PENDRIVE", "/dev/sdz1"]);
}

#[test]
fn mkfs_command_builds_ext4_args_with_force_flag() {
    let (bin, args) = mkfs_command("ext4", "DATA", "/dev/sdz1").expect("ext4 mapeado");
    assert_eq!(bin, "mkfs.ext4");
    assert_eq!(args, vec!["-F", "-L", "DATA", "/dev/sdz1"]);
}

#[test]
fn mkfs_command_omits_label_flag_when_label_is_empty() {
    let (bin, args) = mkfs_command("exfat", "", "/dev/sdz1").expect("exfat mapeado");
    assert_eq!(bin, "mkfs.exfat");
    assert_eq!(args, vec!["/dev/sdz1"]);
}

#[test]
fn mkfs_command_returns_none_for_unmapped_fs_type() {
    assert_eq!(mkfs_command("zfs", "X", "/dev/sdz1"), None);
}

#[test]
fn sudo_invocation_uses_dash_n_without_dashes_k_or_s_when_cached() {
    let args = sudo_invocation(true, "mkfs.vfat", &["-F".to_string(), "32".to_string()]);
    assert_eq!(args, vec!["-n", "--", "mkfs.vfat", "-F", "32"]);
}

#[test]
fn sudo_invocation_uses_dash_s_dash_k_when_password_required() {
    let args = sudo_invocation(false, "dd", &["if=x".to_string(), "of=y".to_string()]);
    assert_eq!(args, vec!["-S", "-k", "--", "dd", "if=x", "of=y"]);
}

#[test]
fn is_sudo_auth_failure_detects_incorrect_password_variants() {
    for msg in [
        "Sorry, try again.",
        "sudo: 1 incorrect password attempt",
        "sudo: no password was provided",
    ] {
        assert!(is_sudo_auth_failure(msg), "esperava match para: {msg}");
    }
}

#[test]
fn is_sudo_auth_failure_is_false_for_unrelated_stderr() {
    assert!(!is_sudo_auth_failure(
        "dd: failed to open '/dev/sdz': No space left on device"
    ));
}

#[test]
fn parse_dd_bytes_copied_extracts_leading_byte_count() {
    let line = "104857600 bytes (105 MB, 100 MiB) copied, 1 s, 100 MB/s";
    assert_eq!(parse_dd_bytes_copied(line), Some(104_857_600));
}

#[test]
fn parse_dd_bytes_copied_ignores_records_in_out_lines() {
    assert_eq!(parse_dd_bytes_copied("25+0 records in"), None);
    assert_eq!(parse_dd_bytes_copied(""), None);
}

#[test]
fn sudo_prompt_open_populates_label_and_retry_error() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    let (respond, _rx) = tokio::sync::oneshot::channel();
    app.open_sudo_prompt(SudoPasswordRequest {
        label: "Formatar /dev/sdb1".to_string(),
        retry_error: Some("Senha incorreta".to_string()),
        respond,
    });
    assert!(app.sudo_prompt_open());
    let state = app.sudo_prompt.as_ref().expect("modal aberto");
    assert_eq!(state.label, "Formatar /dev/sdb1");
    assert_eq!(state.password, "");
    assert_eq!(state.error.as_deref(), Some("Senha incorreta"));
}

#[test]
fn sudo_prompt_enter_sends_typed_password_and_closes_modal() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    let (respond, mut rx) = tokio::sync::oneshot::channel();
    app.open_sudo_prompt(SudoPasswordRequest {
        label: "Gravar ISO em /dev/sdb".to_string(),
        retry_error: None,
        respond,
    });
    let (action_tx, _rx2) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageModalChar('h'), &action_tx);
    app.dispatch(Action::StorageModalChar('i'), &action_tx);
    app.dispatch(Action::StorageModalBackspace, &action_tx);
    app.dispatch(Action::StorageModalChar('i'), &action_tx);
    app.dispatch(Action::Enter, &action_tx);

    assert!(!app.sudo_prompt_open());
    assert_eq!(
        rx.try_recv().expect("resposta enviada"),
        Some("hi".to_string())
    );
}

#[test]
fn sudo_prompt_esc_cancels_with_none_and_closes_modal() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    let (respond, mut rx) = tokio::sync::oneshot::channel();
    app.open_sudo_prompt(SudoPasswordRequest {
        label: "Preparar multi-boot em /dev/sdb".to_string(),
        retry_error: None,
        respond,
    });
    let (action_tx, _rx2) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageModalChar('x'), &action_tx);
    app.dispatch(Action::ToggleConfig, &action_tx);

    assert!(!app.sudo_prompt_open());
    assert_eq!(rx.try_recv().expect("resposta enviada"), None);
}

#[test]
fn sudo_prompt_takes_priority_over_an_already_open_storage_modal() {
    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (action_tx, _rx2) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFormatOpen, &action_tx);
    assert!(app.storage_modal_open());

    let (respond, mut rx) = tokio::sync::oneshot::channel();
    app.open_sudo_prompt(SudoPasswordRequest {
        label: "Formatar /dev/sdb1".to_string(),
        retry_error: None,
        respond,
    });

    app.dispatch(Action::StorageModalChar('z'), &action_tx);
    app.dispatch(Action::Enter, &action_tx);

    assert!(!app.sudo_prompt_open());
    assert_eq!(
        rx.try_recv().expect("resposta enviada"),
        Some("z".to_string())
    );
    assert!(matches!(app.storage_modal, StorageModal::Format(_)));
}

#[test]
fn format_open_is_refused_for_system_disk_and_no_modal_opens() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.active = Tab::Storage;
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));
    app.storage_selected = 0;

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
fn format_modal_cycles_fs_edits_label_and_sends_action_on_enter() {
    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFormatOpen, &tx);

    app.dispatch(Action::Right, &tx);
    app.dispatch(Action::Right, &tx);

    app.dispatch(Action::Down, &tx);
    for _ in 0.."PENDRIVE".len() {
        app.dispatch(Action::StorageModalBackspace, &tx);
    }
    for c in "MEUPEN".chars() {
        app.dispatch(Action::StorageModalChar(c), &tx);
    }

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
fn format_modal_enter_on_fs_field_formats_immediately() {

    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFormatOpen, &tx);

    match &app.storage_modal {
        StorageModal::Format(s) => assert_eq!(s.field, FormatField::Fs),
        other => panic!("esperava modal de formatação, obteve {other:?}"),
    }

    app.dispatch(Action::Enter, &tx);

    assert!(matches!(app.storage_modal, StorageModal::None));
    match rx.try_recv() {
        Ok(Action::StorageFormat {
            device_id,
            fs_type,
            label,
        }) => {
            assert_eq!(device_id, "/drives/usb-target");
            assert_eq!(fs_type, "vfat");
            assert_eq!(label, "PENDRIVE");
        }
        other => panic!("esperava StorageFormat, obteve {other:?}"),
    }

    assert!(app.toast.is_some());
}

#[test]
fn format_modal_tab_and_shift_tab_cycle_field_focus() {
    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFormatOpen, &tx);

    let field_of = |app: &App| match &app.storage_modal {
        StorageModal::Format(s) => s.field,
        other => panic!("esperava modal de formatação, obteve {other:?}"),
    };

    assert_eq!(field_of(&app), FormatField::Fs);
    app.dispatch(Action::NextTab, &tx);
    assert_eq!(field_of(&app), FormatField::Label);
    app.dispatch(Action::NextTab, &tx);
    assert_eq!(field_of(&app), FormatField::Confirm);
    app.dispatch(Action::NextTab, &tx);
    assert_eq!(field_of(&app), FormatField::Fs);

    app.dispatch(Action::PrevTab, &tx);
    assert_eq!(field_of(&app), FormatField::Confirm);
}

#[test]
fn format_modal_esc_closes_without_sending_action() {
    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFormatOpen, &tx);
    assert!(app.storage_modal_open());

    app.dispatch(Action::ToggleConfig, &tx);

    assert!(matches!(app.storage_modal, StorageModal::None));
    assert!(rx.try_recv().is_err());
}

#[test]
fn flasher_open_is_refused_for_system_disk() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.active = Tab::Storage;
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));
    app.storage_selected = 0;

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

fn open_flasher_and_pick_iso(
    app: &mut App,
    tx: &tokio::sync::broadcast::Sender<Action>,
    iso_path: &std::path::Path,
) {
    app.dispatch(Action::StorageFlasherOpen, tx);
    match &mut app.storage_modal {
        StorageModal::FilePicker(s) => {
            s.cwd = iso_path.parent().unwrap().to_path_buf();
            s.reload();
            s.selected = s
                .entries
                .iter()
                .position(|e| e.path == iso_path)
                .expect("arquivo ISO não encontrado na listagem do seletor");
        }
        other => panic!("esperava FilePicker (seletor Yazi) ao abrir o flasher, obteve {other:?}"),
    }
    app.dispatch(Action::Enter, tx);
}

#[test]
fn flasher_rejects_iso_larger_than_target_capacity() {
    let iso = tempfile::Builder::new().suffix(".iso").tempfile().unwrap();
    std::fs::write(iso.path(), vec![0u8; 4096]).unwrap();

    let mut app = app_with_usb_target("/dev/sdz", 1024);
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    open_flasher_and_pick_iso(&mut app, &tx, iso.path());

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
    let iso = tempfile::Builder::new().suffix(".iso").tempfile().unwrap();
    std::fs::write(iso.path(), vec![0xABu8; 4096]).unwrap();

    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    open_flasher_and_pick_iso(&mut app, &tx, iso.path());
    match &app.storage_modal {
        StorageModal::Flasher(s) => assert!(matches!(s.stage, FlasherStage::Ready { .. })),
        other => panic!("esperava Ready, obteve {other:?}"),
    }

    app.dispatch(Action::Enter, &tx);
    app.dispatch(Action::Enter, &tx);

    type_path(&mut app, &tx, "/dev/wrong");
    app.dispatch(Action::Enter, &tx);
    assert!(rx.try_recv().is_err(), "gravação não deveria ter iniciado");
    match &app.storage_modal {
        StorageModal::Flasher(s) => assert!(matches!(s.stage, FlasherStage::Confirm2 { .. })),
        other => panic!("esperava permanecer em Confirm2, obteve {other:?}"),
    }

    for _ in 0.."/dev/wrong".len() {
        app.dispatch(Action::StorageModalBackspace, &tx);
    }
    type_path(&mut app, &tx, "/dev/sdz");
    app.dispatch(Action::Enter, &tx);

    match rx.try_recv() {
        Ok(Action::StorageFlashIso {
            device_id,
            iso_path,
        }) => {
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
    let iso = tempfile::Builder::new().suffix(".iso").tempfile().unwrap();
    std::fs::write(iso.path(), vec![0xABu8; 4096]).unwrap();

    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    open_flasher_and_pick_iso(&mut app, &tx, iso.path());
    app.dispatch(Action::Enter, &tx);
    app.dispatch(Action::Enter, &tx);
    type_path(&mut app, &tx, "/dev/sdz");
    app.dispatch(Action::Enter, &tx);
    let _ = rx.try_recv();

    app.dispatch(Action::ToggleConfig, &tx);

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
    let iso = tempfile::Builder::new().suffix(".iso").tempfile().unwrap();
    std::fs::write(iso.path(), vec![0xABu8; 4096]).unwrap();

    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    open_flasher_and_pick_iso(&mut app, &tx, iso.path());
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
    let iso = tempfile::Builder::new().suffix(".iso").tempfile().unwrap();
    std::fs::write(iso.path(), vec![0xABu8; 4096]).unwrap();

    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    open_flasher_and_pick_iso(&mut app, &tx, iso.path());
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

#[test]
fn system_disk_never_reachable_for_format_or_flash_via_app_dispatch() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.active = Tab::Storage;
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));
    app.storage_selected = 0;

    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageFormatOpen, &tx);
    app.dispatch(Action::StorageFlasherOpen, &tx);

    assert!(matches!(app.storage_modal, StorageModal::None));
    assert!(rx.try_recv().is_err());
}

#[test]
fn render_format_and_flasher_modals_without_panic() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

    app.dispatch(Action::StorageFormatOpen, &tx);
    terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
    app.dispatch(Action::ToggleConfig, &tx);

    app.dispatch(Action::StorageFlasherOpen, &tx);
    terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
}

fn usb_target_snapshot_with_partition(dev_node: &str, size: u64) -> StorageSnapshot {
    let mut d = drive(true, BusType::Usb);
    d.id = DeviceId("/drives/usb-target".into());
    d.dev_node = dev_node.to_string();
    d.size = size;
    d.is_system = false;
    let mut p = partition(vec![], false);
    p.id = DeviceId("/block_devices/usb-target1".into());
    p.dev_node = format!("{dev_node}1");
    p.fs = FsKind::Vfat;
    p.label = "MULTIBOOT".to_string();
    d.partitions = vec![p];
    StorageSnapshot {
        udisks_available: true,
        drives: vec![d],
    }
}

fn app_with_usb_target_partitioned(dev_node: &str, size: u64) -> App {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.active = Tab::Storage;
    app.handle_event(AppEvent::Storage(Box::new(usb_target_snapshot_with_partition(
        dev_node, size,
    ))));
    app.storage_selected = 0;
    app
}

#[test]
fn multiboot_prepare_open_is_refused_for_system_disk() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.active = Tab::Storage;
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));
    app.storage_selected = 0;

    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageMultibootPrepareOpen, &tx);

    assert!(app.toast.is_some());
    assert!(rx.try_recv().is_err());
}

#[test]
fn multiboot_prepare_open_sends_action_with_primary_partition_id() {
    let mut app = app_with_usb_target_partitioned("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageMultibootPrepareOpen, &tx);

    match rx.try_recv() {
        Ok(Action::StorageMultibootPrepare { device_id }) => {
            assert_eq!(device_id, "/block_devices/usb-target1");
        }
        other => panic!("esperava StorageMultibootPrepare, obteve {other:?}"),
    }
}

#[test]
fn multiboot_prepare_open_toasts_when_no_partition_available() {

    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let (tx, mut rx) = tokio::sync::broadcast::channel(8);
    app.dispatch(Action::StorageMultibootPrepareOpen, &tx);

    assert!(app.toast.is_some());
    assert!(rx.try_recv().is_err());
}

#[test]
fn multiboot_prepare_action_is_blocked_by_is_system_target() {

    let snap = mock_snapshot();
    let system_partition_id = snap.drives[0].partitions[0].id.clone();
    assert!(snap.is_system_target(&system_partition_id));
}

fn labeled_partition(label: &str, dev_node: &str, size: u64) -> PartitionInfo {
    let mut p = partition(vec![], false);
    p.label = label.to_string();
    p.dev_node = dev_node.to_string();
    p.size = size;
    p
}

#[test]
fn detect_ventoy_recognizes_ventoy_and_vtoyefi_labels_case_insensitively() {
    assert!(detect_ventoy(&[labeled_partition(
        "Ventoy",
        "/dev/sdz1",
        1
    )]));
    assert!(detect_ventoy(&[labeled_partition(
        "VENTOY",
        "/dev/sdz1",
        1
    )]));
    assert!(detect_ventoy(&[labeled_partition(
        "VTOYEFI",
        "/dev/sdz2",
        1
    )]));
    assert!(detect_ventoy(&[labeled_partition(
        "vtoyefi",
        "/dev/sdz2",
        1
    )]));
}

#[test]
fn detect_ventoy_is_false_for_unrelated_labels() {
    assert!(!detect_ventoy(&[labeled_partition(
        "KINGSTON",
        "/dev/sdz1",
        1
    )]));
    assert!(!detect_ventoy(&[]));
}

#[test]
fn ventoy_data_partition_picks_the_non_efi_partition() {
    let mut d = drive(true, BusType::Usb);
    d.partitions = vec![
        labeled_partition("VTOYEFI", "/dev/sdz1", 32 * 1024 * 1024),
        labeled_partition("Ventoy", "/dev/sdz2", 30 * 1024 * 1024 * 1024),
    ];
    let data = ventoy_data_partition(&d).expect("esperava a partição de dados");
    assert_eq!(data.dev_node, "/dev/sdz2");
}

#[test]
fn ventoy_data_partition_falls_back_to_largest_non_efi_when_unlabeled() {
    let mut d = drive(true, BusType::Usb);
    d.partitions = vec![
        labeled_partition("VTOYEFI", "/dev/sdz1", 32 * 1024 * 1024),
        labeled_partition("", "/dev/sdz2", 30 * 1024 * 1024 * 1024),
    ];
    let data = ventoy_data_partition(&d).expect("esperava a partição de dados");
    assert_eq!(data.dev_node, "/dev/sdz2");
}

#[test]
fn friendly_label_appends_dev_node_for_mmc_drives_without_a_partition_label() {
    let mut d = drive(true, BusType::Mmc);
    d.model = "SD16G".into();
    d.vendor = "".into();
    d.dev_node = "/dev/mmcblk0".into();
    d.partitions = vec![labeled_partition("", "/dev/mmcblk0p1", 16 * 1024 * 1024 * 1024)];
    assert_eq!(d.friendly_label(), "SD16G (/dev/mmcblk0)");
}

#[test]
fn friendly_label_prefers_the_partition_label_even_for_mmc_drives() {
    let mut d = drive(true, BusType::Mmc);
    d.model = "SD16G".into();
    d.dev_node = "/dev/mmcblk0".into();
    d.partitions = vec![labeled_partition(
        "MEUCARTAO",
        "/dev/mmcblk0p1",
        16 * 1024 * 1024 * 1024,
    )];
    assert_eq!(d.friendly_label(), "MEUCARTAO");
}

#[test]
fn mmc_drives_always_skip_power_off_even_when_can_power_off_is_true() {
    let mut d = drive(true, BusType::Mmc);

    d.can_power_off = true;
    assert!(skips_power_off(&d));
}

#[test]
fn usb_drives_without_can_power_off_skip_power_off() {
    let mut d = drive(true, BusType::Usb);
    d.can_power_off = false;
    assert!(skips_power_off(&d));
}

#[test]
fn usb_drives_with_can_power_off_do_not_skip_power_off() {
    let mut d = drive(true, BusType::Usb);
    d.can_power_off = true;
    assert!(!skips_power_off(&d));
}

#[test]
fn no_usb_device_error_is_recognized_case_insensitively() {
    let err = anyhow::anyhow!("org.freedesktop.UDisks2.Error.Failed: No usb device");
    assert!(is_no_usb_device_error(&err));
    let err_lower = anyhow::anyhow!("no usb device");
    assert!(is_no_usb_device_error(&err_lower));
}

#[test]
fn unrelated_power_off_errors_are_not_treated_as_no_usb_device() {
    let err = anyhow::anyhow!("org.freedesktop.UDisks2.Error.Failed: Device is busy");
    assert!(!is_no_usb_device_error(&err));
}

#[test]
fn is_iso_or_img_is_case_insensitive_and_rejects_other_extensions() {
    assert!(is_iso_or_img("archlinux.iso"));
    assert!(is_iso_or_img("Windows.ISO"));
    assert!(is_iso_or_img("disk.img"));
    assert!(!is_iso_or_img("readme.txt"));
    assert!(!is_iso_or_img("archive.iso.zip"));
}

fn write_gzip_fixture(dir: &std::path::Path, name: &str, payload: &[u8]) -> std::path::PathBuf {
    use std::io::Write;
    let path = dir.join(name);
    let mut encoder =
        flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(payload).expect("write payload");
    let compressed = encoder.finish().expect("finish gzip stream");
    std::fs::write(&path, compressed).expect("write gzip fixture");
    path
}

#[test]
fn is_gzip_file_detects_the_gzip_magic_bytes() {
    let dir = tempfile::tempdir().expect("tempdir");
    let gz_path = write_gzip_fixture(dir.path(), "image.img.gz", b"conteudo de teste gzip");
    assert!(is_gzip_file(&gz_path).expect("read magic"));

    let raw_path = dir.path().join("image.img");
    std::fs::write(&raw_path, b"nao e gzip").expect("write raw fixture");
    assert!(!is_gzip_file(&raw_path).expect("read magic"));
}

#[test]
fn is_gzip_file_handles_files_shorter_than_the_magic() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tiny_path = dir.path().join("tiny");
    std::fs::write(&tiny_path, [0x1f]).expect("write tiny fixture");
    assert!(!is_gzip_file(&tiny_path).expect("read magic"));

    let empty_path = dir.path().join("empty");
    std::fs::write(&empty_path, []).expect("write empty fixture");
    assert!(!is_gzip_file(&empty_path).expect("read magic"));
}

#[test]
fn gzip_uncompressed_size_hint_matches_the_original_payload_length() {
    let dir = tempfile::tempdir().expect("tempdir");
    let payload = vec![0x42u8; 5 * 1024 * 1024];
    let gz_path = write_gzip_fixture(dir.path(), "image.raw.gz", &payload);
    let hint = gzip_uncompressed_size_hint(&gz_path).expect("footer ISIZE");
    assert_eq!(hint, payload.len() as u64);
}

#[test]
fn build_ventoy_entries_filters_and_sorts_case_insensitively() {
    let raw = vec![
        ("zeta.iso".to_string(), 10, None),
        ("Alpha.ISO".to_string(), 20, None),
        ("notes.txt".to_string(), 30, None),
        ("beta.img".to_string(), 40, None),
    ];
    let entries = build_ventoy_entries(raw);
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["Alpha.ISO", "beta.img", "zeta.iso"]);
}

#[test]
fn render_file_picker_modal_without_panic() {
    use hal9001::app::{FilePickerPurpose, FilePickerState, StorageModal};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    app.storage_modal = StorageModal::FilePicker(FilePickerState::open(
        std::env::temp_dir(),
        FilePickerPurpose::FlasherIso {
            device_id: "/drives/usb-target".to_string(),
            target_label: "Test Drive".to_string(),
            target_dev_node: "/dev/sdz".to_string(),
            target_size: 8 * 1024 * 1024 * 1024,
        },
    ));

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
}

#[test]
fn render_multiboot_iso_manager_modal_in_every_stage_without_panic() {
    use hal9001::app::{MultibootIsoManagerStage, MultibootIsoManagerState, StorageModal};
    use hal9001::backend::storage::VentoyIsoEntry;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let mut app = app_with_usb_target("/dev/sdz", 8 * 1024 * 1024 * 1024);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();

    let base = |stage: MultibootIsoManagerStage| {
        StorageModal::MultibootIsoManager(MultibootIsoManagerState {
            device_id: "/drives/usb-target".to_string(),
            target_label: "Multi-boot USB".to_string(),
            stage,
        })
    };

    let stages = vec![
        MultibootIsoManagerStage::Loading,
        MultibootIsoManagerStage::Listing {
            entries: vec![VentoyIsoEntry {
                name: "archlinux.iso".to_string(),
                size: 900 * 1024 * 1024,
                modified: None,
            }],
            selected: 0,
            free_bytes: Some(4 * 1024 * 1024 * 1024),
        },
        MultibootIsoManagerStage::ConfirmRemove {
            file_name: "archlinux.iso".to_string(),
        },
        MultibootIsoManagerStage::Copying {
            bytes_written: 512,
            total_bytes: 1024,
            file_name: "new.iso".to_string(),
        },
        MultibootIsoManagerStage::Removing {
            file_name: "archlinux.iso".to_string(),
        },
        MultibootIsoManagerStage::Error {
            message: "falha simulada".to_string(),
        },
    ];

    for stage in stages {
        app.storage_modal = base(stage);
        terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
    }
}

#[test]
fn format_fat32_pure_rust_produces_a_mountable_fat32_volume_with_label() {

    let file = tempfile::NamedTempFile::new().unwrap();
    file.as_file().set_len(64 * 1024 * 1024).unwrap();
    let path = file.path().to_str().unwrap();

    format_fat32_pure_rust(path, "meu label").expect("formatação FAT32 deveria ter sucesso");

    let dev = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let fs = fatfs::FileSystem::new(dev, fatfs::FsOptions::new())
        .expect("volume formatado deveria ser montável pela própria fatfs");

    assert_eq!(fs.fat_type(), fatfs::FatType::Fat32);

    assert_eq!(fs.volume_label(), "MEU LABEL");

    let root_entries: Vec<_> = fs.root_dir().iter().collect();
    assert!(
        root_entries.is_empty(),
        "esperava diretório raiz vazio logo após a formatação"
    );
}

#[test]
fn format_fat32_pure_rust_allows_writing_files_after_format() {
    let file = tempfile::NamedTempFile::new().unwrap();
    file.as_file().set_len(64 * 1024 * 1024).unwrap();
    let path = file.path().to_str().unwrap();

    format_fat32_pure_rust(path, "PENDRIVE").unwrap();

    let dev = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    let fs = fatfs::FileSystem::new(dev, fatfs::FsOptions::new()).unwrap();
    let root = fs.root_dir();

    use std::io::{Read as _, Write as _};
    let mut f = root.create_file("hello.txt").unwrap();
    f.write_all(b"hal-9001").unwrap();
    drop(f);

    let mut f = root.open_file("hello.txt").unwrap();
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).unwrap();
    assert_eq!(buf, b"hal-9001");
}

#[test]
fn format_fat32_pure_rust_fails_gracefully_for_a_missing_path() {
    let err = format_fat32_pure_rust("/nonexistent/hal9001-test-device", "X")
        .expect_err("caminho inexistente deveria falhar, não pânico");
    assert!(!err.to_string().is_empty());
}

#[test]
fn no_emojis_anywhere_in_the_source_tree() {
    fn is_emoji(c: char) -> bool {
        let cp = c as u32;
        matches!(cp,
            0x1F300..=0x1FAFF
            | 0x2600..=0x27BF
            | 0x1F1E6..=0x1F1FF
        )
    }

    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut offenders: Vec<String> = Vec::new();

    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if matches!(name, "target" | ".git") {
                    continue;
                }
                walk(&path, out);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if matches!(ext, "rs" | "sh" | "toml") {
                    out.push(path);
                }
            }
        }
    }

    let mut files = Vec::new();
    walk(&manifest_dir.join("src"), &mut files);
    walk(&manifest_dir.join("tests"), &mut files);
    walk(&manifest_dir.join("scripts"), &mut files);

    for path in files {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        for (i, line) in text.lines().enumerate() {
            if line.chars().any(is_emoji) {
                offenders.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "emojis encontrados no código-fonte (Zero Emojis Policy):\n{}",
        offenders.join("\n")
    );
}

#[test]
fn needs_continuous_tick_while_disk_analyzer_is_scanning() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.active = Tab::Storage;
    assert!(!app.needs_continuous_tick());

    app.storage_analyzer = Some(DiskAnalyzerState::opening("/tmp".into()));
    assert!(app.needs_continuous_tick());

    if let Some(state) = &mut app.storage_analyzer {
        state.is_scanning = false;
    }
    assert!(!app.needs_continuous_tick());
}

#[test]
fn on_tick_advances_disk_analyzer_spinner_only_while_scanning() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.storage_analyzer = Some(DiskAnalyzerState::opening("/tmp".into()));
    app.on_tick();
    app.on_tick();
    assert_eq!(app.storage_analyzer.as_ref().unwrap().spinner_frame, 2);

    if let Some(state) = &mut app.storage_analyzer {
        state.is_scanning = false;
    }
    app.on_tick();
    assert_eq!(app.storage_analyzer.as_ref().unwrap().spinner_frame, 2);
}

#[test]
fn storage_analyzer_progress_event_updates_scanning_state() {
    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.storage_analyzer = Some(DiskAnalyzerState::opening("/tmp".into()));

    app.handle_event(AppEvent::StorageAnalyzerProgress {
        current_item: "/tmp/foo/bar.txt".to_string(),
        items_scanned: 1420,
        total_bytes: 4_100_000_000,
    });

    let state = app.storage_analyzer.as_ref().unwrap();
    assert_eq!(state.files_scanned, 1420);
    assert_eq!(state.total_bytes, 4_100_000_000);
    assert_eq!(
        state.current_scanning_item.as_deref(),
        Some("/tmp/foo/bar.txt")
    );
}

#[test]
fn render_disk_analyzer_scanning_panel_without_panic() {
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    let mut cfg = Config::default();
    cfg.splash.enabled = false;
    let mut app = App::new(cfg);
    app.active = Tab::Storage;
    app.handle_event(AppEvent::Storage(Box::new(mock_snapshot())));
    app.storage_analyzer = Some(DiskAnalyzerState::opening("/home/user/some/deep/path".into()));
    app.handle_event(AppEvent::StorageAnalyzerProgress {
        current_item: "/home/user/some/deep/path/big_file.bin".to_string(),
        items_scanned: 1420,
        total_bytes: 4_100_000_000,
    });

    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    for frame in 0..12 {
        if let Some(state) = &mut app.storage_analyzer {
            state.spinner_frame = frame;
        }
        terminal.draw(|f| hal9001::ui::draw(&app, f)).unwrap();
    }
}
