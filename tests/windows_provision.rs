mod common;

use common::{build_iso, Entry};
use hal9001::backend::image_probe::WindowsPlan;
use hal9001::backend::windows_provision::extract_windows_to_mount;

fn windows_iso() -> Vec<u8> {
    build_iso(&[
        Entry::file("/bootmgr.efi", b"BOOTMGR-EFI".to_vec()),
        Entry::file("/EFI/Microsoft/Boot/bootmgfw.efi", b"BOOTMGFW".to_vec()),
        Entry::file("/EFI/Microsoft/Boot/BCD", b"BCD-DATA".to_vec()),
        Entry::file("/EFI/Boot/bootx64.efi", b"BOOTX64".to_vec()),
        Entry::file("/sources/boot.wim", b"BOOT-WIM".to_vec()),
        Entry::file("/sources/install.wim", b"INSTALL-WIM".to_vec()),
    ])
}

#[test]
fn extract_all_to_fat32_writes_boot_chain_and_install_wim() {
    let image = windows_iso();
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), &image).unwrap();
    let mount = tempfile::tempdir().unwrap();

    let mut reader = hal9001::backend::iso_reader::IsoReader::open(tmp.path()).unwrap();
    let result = extract_windows_to_mount(
        &mut reader,
        mount.path(),
        WindowsPlan::ExtractAllToFat32,
        |_, _| {},
    )
    .unwrap();

    assert!(result.files_written >= 5);
    assert!(!result.wim_split);
    assert_eq!(
        std::fs::read(mount.path().join("bootmgr.efi")).unwrap(),
        b"BOOTMGR-EFI"
    );
    assert_eq!(
        std::fs::read(mount.path().join("EFI/Microsoft/Boot/bootmgfw.efi")).unwrap(),
        b"BOOTMGFW"
    );
    assert_eq!(
        std::fs::read(mount.path().join("sources/install.wim")).unwrap(),
        b"INSTALL-WIM"
    );
}

#[test]
fn extract_boot_only_skips_install_wim() {
    let image = windows_iso();
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), &image).unwrap();
    let mount = tempfile::tempdir().unwrap();

    let mut reader = hal9001::backend::iso_reader::IsoReader::open(tmp.path()).unwrap();
    let result = extract_windows_to_mount(
        &mut reader,
        mount.path(),
        WindowsPlan::ExtractBootOnly,
        |_, _| {},
    )
    .unwrap();

    assert!(!mount.path().join("sources/install.wim").exists());
    assert!(mount.path().join("sources/boot.wim").exists());
    assert!(result.files_written >= 4);
}

#[test]
fn split_wim_marks_flag_and_skips_install_wim() {
    let image = windows_iso();
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), &image).unwrap();
    let mount = tempfile::tempdir().unwrap();

    let mut reader = hal9001::backend::iso_reader::IsoReader::open(tmp.path()).unwrap();
    let result = extract_windows_to_mount(
        &mut reader,
        mount.path(),
        WindowsPlan::SplitWim {
            size: 5 * 1024 * 1024 * 1024,
        },
        |_, _| {},
    )
    .unwrap();

    assert!(result.wim_split);
    assert!(!mount.path().join("sources/install.wim").exists());
    assert!(mount.path().join("bootmgr.efi").exists());
}
