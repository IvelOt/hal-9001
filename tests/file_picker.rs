use std::path::PathBuf;
use std::time::SystemTime;

use hal9001::app::{App, FilePickerOutcome, FilePickerPurpose, FilePickerState, StorageModal};
use hal9001::config::Config;
use hal9001::events::Action;
use hal9001::ui::file_picker::{
    is_flashable_image, is_pickable_for, is_pickable_image, sort_entries, FileEntry,
};

fn entry(name: &str, is_dir: bool) -> FileEntry {
    FileEntry {
        name: name.to_string(),
        path: PathBuf::from(name),
        is_dir,
        size: 0,
        modified: None,
    }
}

#[test]
fn sort_entries_puts_directories_before_files() {
    let mut entries = vec![
        entry("zeta.iso", false),
        entry("Alpha", true),
        entry("beta.txt", false),
        entry("omega", true),
    ];
    sort_entries(&mut entries);
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["Alpha", "omega", "beta.txt", "zeta.iso"]);
}

#[test]
fn sort_entries_is_case_insensitive_within_groups() {
    let mut entries = vec![entry("banana.iso", false), entry("Apple.iso", false)];
    sort_entries(&mut entries);
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["Apple.iso", "banana.iso"]);
}

#[test]
fn is_pickable_image_recognizes_iso_img_vhd_raw_case_insensitively() {
    assert!(is_pickable_image("archlinux.iso"));
    assert!(is_pickable_image("Windows.ISO"));
    assert!(is_pickable_image("disk.img"));
    assert!(is_pickable_image("disk.IMG"));
    assert!(is_pickable_image("machine.vhd"));
    assert!(is_pickable_image("machine.VHD"));
    assert!(is_pickable_image("card.raw"));
    assert!(is_pickable_image("card.RAW"));
}

#[test]
fn is_pickable_image_rejects_compressed_and_other_extensions() {
    assert!(!is_pickable_image("readme.txt"));
    assert!(!is_pickable_image("noextension"));
    assert!(!is_pickable_image("archlinux.img.gz"));
    assert!(!is_pickable_image("archive.zip"));
}

#[test]
fn is_flashable_image_recognizes_compressed_extensions_case_insensitively() {
    assert!(is_flashable_image("archlinux.img.gz"));
    assert!(is_flashable_image("Fedora.ISO.GZ"));
    assert!(is_flashable_image("sdcard.raw.gz"));
    assert!(is_flashable_image("generic.gz"));
    assert!(is_flashable_image("archive.zip"));
    assert!(is_flashable_image("archive.xz"));
    assert!(is_flashable_image("archive.zst"));

    assert!(is_flashable_image("archlinux.iso"));
    assert!(is_flashable_image("card.raw"));
}

#[test]
fn is_flashable_image_rejects_unrelated_extensions() {
    assert!(!is_flashable_image("readme.txt"));
    assert!(!is_flashable_image("noextension"));
}

#[test]
fn is_pickable_for_restricts_compressed_images_to_the_flasher_purpose() {
    let flasher = FilePickerPurpose::FlasherIso {
        device_id: "dev".to_string(),
        target_label: "pendrive".to_string(),
        target_dev_node: "/dev/sdz".to_string(),
        target_size: 0,
    };
    let multiboot = FilePickerPurpose::MultibootAddIso {
        device_id: "dev".to_string(),
        target_label: "pendrive".to_string(),
    };

    assert!(is_pickable_for(&flasher, "archlinux.img.gz"));
    assert!(is_pickable_for(&flasher, "archlinux.iso"));
    assert!(!is_pickable_for(&multiboot, "archlinux.img.gz"));
    assert!(is_pickable_for(&multiboot, "archlinux.iso"));
}

fn unique_temp_dir(tag: &str) -> PathBuf {
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "hal9001-filepicker-test-{tag}-{}-{n}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn flasher_purpose() -> FilePickerPurpose {
    FilePickerPurpose::FlasherIso {
        device_id: "/drives/test".to_string(),
        target_label: "Test Drive".to_string(),
        target_dev_node: "/dev/sdz".to_string(),
        target_size: 8 * 1024 * 1024 * 1024,
    }
}

#[test]
fn navigating_into_a_directory_resets_selection_and_reloads() {
    let root = unique_temp_dir("nav");
    std::fs::create_dir_all(root.join("subdir")).unwrap();
    std::fs::write(root.join("subdir/image.iso"), b"fake iso").unwrap();
    std::fs::write(root.join("aaa_first.iso"), b"x").unwrap();

    let mut s = FilePickerState::open(root.clone(), flasher_purpose());

    assert_eq!(s.entries.first().map(|e| e.name.as_str()), Some("subdir"));
    s.selected = 0;

    match s.enter_selected() {
        FilePickerOutcome::None => {}
        other => panic!("esperava navegação para dentro do diretório, obteve {other:?}"),
    }
    assert_eq!(s.cwd, root.join("subdir"));
    assert_eq!(s.selected, 0);
    assert_eq!(s.entries.len(), 1);
    assert_eq!(s.entries[0].name, "image.iso");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn entering_a_pickable_image_returns_picked_outcome() {
    let root = unique_temp_dir("pick");
    std::fs::write(root.join("linux.iso"), b"fake iso data").unwrap();

    let mut s = FilePickerState::open(root.clone(), flasher_purpose());
    s.selected = 0;
    match s.enter_selected() {
        FilePickerOutcome::Picked(paths) => assert_eq!(paths, vec![root.join("linux.iso")]),
        other => panic!("esperava Picked, obteve {other:?}"),
    }

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn unsupported_files_are_filtered_out_of_the_listing_entirely() {
    let root = unique_temp_dir("unsupported");
    std::fs::write(root.join("notes.txt"), b"hello").unwrap();
    std::fs::write(root.join("linux.iso"), b"fake iso data").unwrap();

    let s = FilePickerState::open(root.clone(), flasher_purpose());
    assert_eq!(
        s.entries.len(),
        1,
        "notes.txt não é selecionável e não deve poluir a lista"
    );
    assert_eq!(s.entries[0].name, "linux.iso");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn directories_are_always_listed_regardless_of_extension_filter() {
    let root = unique_temp_dir("dirs-kept");
    std::fs::create_dir_all(root.join("some_dir")).unwrap();
    std::fs::write(root.join("notes.txt"), b"hello").unwrap();

    let s = FilePickerState::open(root.clone(), flasher_purpose());
    let names: Vec<&str> = s.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["some_dir"]);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn hidden_files_are_excluded_by_default_and_shown_after_toggle() {
    let root = unique_temp_dir("hidden");
    std::fs::write(root.join(".hidden.iso"), b"x").unwrap();
    std::fs::write(root.join("visible.iso"), b"x").unwrap();

    let mut s = FilePickerState::open(root.clone(), flasher_purpose());
    assert!(!s.show_hidden);
    let names: Vec<&str> = s.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["visible.iso"]);

    s.show_hidden = true;
    s.reload();
    let names: Vec<&str> = s.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec![".hidden.iso", "visible.iso"]);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn search_query_filters_entries_case_insensitively() {
    let root = unique_temp_dir("search");
    std::fs::write(root.join("Archlinux.iso"), b"x").unwrap();
    std::fs::write(root.join("debian.iso"), b"x").unwrap();

    let mut s = FilePickerState::open(root.clone(), flasher_purpose());
    s.search_query = "arch".to_string();
    s.reload();
    let names: Vec<&str> = s.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["Archlinux.iso"]);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn toggle_selected_marks_and_unmarks_files_and_advances_cursor() {
    let root = unique_temp_dir("multi-select");
    std::fs::write(root.join("a.iso"), b"1").unwrap();
    std::fs::write(root.join("b.iso"), b"2").unwrap();

    let mut s = FilePickerState::open(root.clone(), flasher_purpose());
    assert_eq!(s.selected, 0);

    s.toggle_selected();
    assert!(s.selected_paths.contains(&root.join("a.iso")));
    assert_eq!(s.selected, 1, "deve avançar para a próxima linha");

    s.toggle_selected();
    assert!(s.selected_paths.contains(&root.join("b.iso")));

    s.selected = 0;
    s.toggle_selected();
    assert!(!s.selected_paths.contains(&root.join("a.iso")));

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn entering_with_multiple_marked_files_returns_all_of_them() {
    let root = unique_temp_dir("multi-pick");
    std::fs::write(root.join("a.iso"), b"1").unwrap();
    std::fs::write(root.join("b.iso"), b"2").unwrap();

    let purpose = FilePickerPurpose::MultibootAddIso {
        device_id: "/drives/ventoy".to_string(),
        target_label: "Ventoy USB".to_string(),
    };
    let mut s = FilePickerState::open(root.clone(), purpose);
    s.toggle_selected();
    s.toggle_selected();
    s.selected = 0;

    let mut paths = match s.enter_selected() {
        FilePickerOutcome::Picked(paths) => paths,
        other => panic!("esperava Picked, obteve {other:?}"),
    };
    paths.sort();
    assert_eq!(paths, vec![root.join("a.iso"), root.join("b.iso")]);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn go_up_does_not_panic_at_filesystem_root() {
    let mut s = FilePickerState::open(PathBuf::from("/"), flasher_purpose());
    assert_eq!(s.cwd, PathBuf::from("/"));
    s.go_up();

    assert_eq!(s.cwd, PathBuf::from("/"));
}

#[test]
fn jump_to_nonexistent_directory_surfaces_error_instead_of_panicking() {
    let root = unique_temp_dir("jump");
    let mut s = FilePickerState::open(root.clone(), flasher_purpose());
    assert!(s.error.is_none());

    s.jump_to(root.join("does-not-exist-at-all"));
    assert!(s.error.is_some());

    assert_eq!(s.cwd, root);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn move_up_and_down_clamp_to_entry_bounds() {
    let root = unique_temp_dir("bounds");
    std::fs::write(root.join("a.iso"), b"1").unwrap();
    std::fs::write(root.join("b.iso"), b"2").unwrap();

    let mut s = FilePickerState::open(root.clone(), flasher_purpose());
    assert_eq!(s.entries.len(), 2);

    s.move_up();
    assert_eq!(s.selected, 0, "não deve ficar negativo/estourar por baixo");

    s.move_down();
    s.move_down();
    s.move_down();
    assert_eq!(s.selected, 1, "não deve ultrapassar o último índice");

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn open_falls_back_to_temp_dir_when_start_dir_is_not_a_directory() {
    let bogus = std::env::temp_dir().join("hal9001-does-not-exist-file-picker-start");
    let s = FilePickerState::open(bogus, flasher_purpose());
    assert_eq!(s.cwd, std::env::temp_dir());
}

#[test]
fn ventoy_add_iso_purpose_round_trips_through_purpose_field() {
    let root = unique_temp_dir("purpose");
    std::fs::write(root.join("image.iso"), b"x").unwrap();
    let purpose = FilePickerPurpose::MultibootAddIso {
        device_id: "/drives/ventoy".to_string(),
        target_label: "Ventoy USB".to_string(),
    };
    let s = FilePickerState::open(root.clone(), purpose.clone());
    assert_eq!(s.purpose, purpose);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn file_entry_modified_field_is_populated_when_available() {
    let root = unique_temp_dir("mtime");
    std::fs::write(root.join("a.iso"), b"x").unwrap();
    let s = FilePickerState::open(root.clone(), flasher_purpose());
    let e = s.entries.first().expect("esperava uma entrada");
    assert!(e.modified.is_some());
    assert!(e.modified.unwrap() <= SystemTime::now());

    std::fs::remove_dir_all(&root).ok();
}

fn app_with_file_picker(root: PathBuf, purpose: FilePickerPurpose) -> App {
    let mut app = App::new(Config::default());
    app.storage_modal = StorageModal::FilePicker(FilePickerState::open(root, purpose));
    app
}

#[test]
fn dispatch_dot_key_toggles_hidden_files_and_reloads() {
    let root = unique_temp_dir("dispatch-hidden");
    std::fs::write(root.join(".hidden.iso"), b"x").unwrap();
    std::fs::write(root.join("visible.iso"), b"x").unwrap();

    let mut app = app_with_file_picker(root.clone(), flasher_purpose());
    let (tx, _rx) = tokio::sync::broadcast::channel(8);

    let entries_len = |app: &App| match &app.storage_modal {
        StorageModal::FilePicker(s) => s.entries.len(),
        other => panic!("esperava FilePicker, obteve {other:?}"),
    };
    assert_eq!(entries_len(&app), 1);

    app.dispatch(Action::StorageModalChar('.'), &tx);
    assert_eq!(entries_len(&app), 2);

    app.dispatch(Action::StorageModalChar('.'), &tx);
    assert_eq!(entries_len(&app), 1);

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn dispatch_slash_activates_search_and_filters_entries_as_typed() {
    let root = unique_temp_dir("dispatch-search");
    std::fs::write(root.join("Archlinux.iso"), b"x").unwrap();
    std::fs::write(root.join("debian.iso"), b"x").unwrap();

    let mut app = app_with_file_picker(root.clone(), flasher_purpose());
    let (tx, _rx) = tokio::sync::broadcast::channel(8);

    app.dispatch(Action::StorageModalChar('/'), &tx);
    app.dispatch(Action::StorageModalChar('a'), &tx);
    app.dispatch(Action::StorageModalChar('r'), &tx);
    app.dispatch(Action::StorageModalChar('c'), &tx);
    app.dispatch(Action::StorageModalChar('h'), &tx);

    match &app.storage_modal {
        StorageModal::FilePicker(s) => {
            assert!(s.is_searching);
            assert_eq!(s.search_query, "arch");
            let names: Vec<&str> = s.entries.iter().map(|e| e.name.as_str()).collect();
            assert_eq!(names, vec!["Archlinux.iso"]);
        }
        other => panic!("esperava FilePicker, obteve {other:?}"),
    }

    app.dispatch(Action::StorageModalBackspace, &tx);
    match &app.storage_modal {
        StorageModal::FilePicker(s) => assert_eq!(s.search_query, "arc"),
        other => panic!("esperava FilePicker, obteve {other:?}"),
    }

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn dispatch_space_marks_multiple_isos_and_enter_confirms_all() {
    let root = unique_temp_dir("dispatch-multi");
    std::fs::write(root.join("a.iso"), b"1").unwrap();
    std::fs::write(root.join("b.iso"), b"2").unwrap();

    let purpose = FilePickerPurpose::MultibootAddIso {
        device_id: "/drives/ventoy".to_string(),
        target_label: "Ventoy USB".to_string(),
    };
    let mut app = app_with_file_picker(root.clone(), purpose);
    let (tx, mut rx) = tokio::sync::broadcast::channel(8);

    app.dispatch(Action::StorageModalChar(' '), &tx);
    app.dispatch(Action::StorageModalChar(' '), &tx);
    app.dispatch(Action::Enter, &tx);

    match &app.storage_modal {
        StorageModal::MultibootIsoManager(s) => {
            assert_eq!(s.pending_adds.len(), 1);
        }
        other => panic!("esperava MultibootIsoManager, obteve {other:?}"),
    }

    let first = rx
        .try_recv()
        .expect("esperava uma ação StorageMultibootAddIso");
    match first {
        Action::StorageMultibootAddIso { .. } => {}
        other => panic!("esperava StorageMultibootAddIso, obteve {other:?}"),
    }

    std::fs::remove_dir_all(&root).ok();
}

#[test]
fn last_file_picker_dir_tracks_navigation() {
    let root = unique_temp_dir("dispatch-lastdir");
    std::fs::create_dir_all(root.join("subdir")).unwrap();
    std::fs::write(root.join("subdir/image.iso"), b"x").unwrap();

    let mut app = app_with_file_picker(root.clone(), flasher_purpose());
    let (tx, _rx) = tokio::sync::broadcast::channel(8);
    assert_eq!(app.last_file_picker_dir, None);

    app.dispatch(Action::Enter, &tx);
    assert_eq!(app.last_file_picker_dir, Some(root.join("subdir")));

    std::fs::remove_dir_all(&root).ok();
}
