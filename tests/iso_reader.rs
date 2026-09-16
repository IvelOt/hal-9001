mod common;

use common::{build_iso, Entry};

#[test]
fn lists_nested_paths() {
    let image = build_iso(&[
        Entry::file("/bootmgr.efi", b"EFI-BOOTMGR".to_vec()),
        Entry::file("/sources/boot.wim", b"BOOT-WIM".to_vec()),
        Entry::file("/EFI/BOOT/BOOTX64.EFI", b"EFI-X64".to_vec()),
    ]);
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), &image).unwrap();

    let mut reader = hal9001::backend::iso_reader::IsoReader::open(tmp.path()).unwrap();
    let paths = reader.list_paths().unwrap();

    assert!(paths.iter().any(|p| p == "/bootmgr.efi"));
    assert!(paths.iter().any(|p| p == "/sources/boot.wim"));
    assert!(paths.iter().any(|p| p == "/EFI/BOOT/BOOTX64.EFI"));
    // Directories are listed with a trailing slash.
    assert!(paths.iter().any(|p| p == "/sources/"));
    assert!(paths.iter().any(|p| p == "/EFI/BOOT/"));
}

#[test]
fn extracts_file_bytes() {
    let content = b"hello-windows-boot".to_vec();
    let image = build_iso(&[Entry::file("/sources/boot.wim", content.clone())]);
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), &image).unwrap();

    let mut reader = hal9001::backend::iso_reader::IsoReader::open(tmp.path()).unwrap();
    let entry = reader.find("/sources/boot.wim").unwrap().unwrap();
    let mut out = Vec::new();
    let mut calls = 0u32;
    reader.extract(&entry, &mut out, |_, _| calls += 1).unwrap();
    assert_eq!(out, content);
    assert!(calls >= 1);
}

#[test]
fn find_is_case_insensitive_and_missing_returns_none() {
    let image = build_iso(&[Entry::file("/install.amd/vmlinuz", b"vmlinuz".to_vec())]);
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), &image).unwrap();

    let mut reader = hal9001::backend::iso_reader::IsoReader::open(tmp.path()).unwrap();
    assert!(reader.find("/INSTALL.AMD/VMLINUZ").unwrap().is_some());
    assert!(reader.find("/nope").unwrap().is_none());
}

#[test]
fn rejects_non_iso() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), b"not an iso at all").unwrap();
    assert!(hal9001::backend::iso_reader::IsoReader::open(tmp.path()).is_err());
}
