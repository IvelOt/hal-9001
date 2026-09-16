mod common;

use common::{build_gpt_fat_disk, build_iso, Entry};
use hal9001::backend::image_probe::{
    inspect, probe_format, ImageFormat, LinuxFlavor, OsClass, PartitionTableKind, ProbeOpts,
    WindowsPlan,
};

fn write_temp(bytes: &[u8]) -> tempfile::NamedTempFile {
    let f = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(f.path(), bytes).unwrap();
    f
}

#[test]
fn probe_format_detects_gpt() {
    let mut disk = vec![0u8; 512 * 65];
    disk[450] = 0xEE; // protective MBR
    disk[510] = 0x55;
    disk[511] = 0xAA;
    disk[512..520].copy_from_slice(b"EFI PART");
    let f = write_temp(&disk);
    assert_eq!(
        probe_format(f.path()).unwrap(),
        ImageFormat::PartitionedDisk {
            table: PartitionTableKind::Gpt
        }
    );
}

#[test]
fn probe_format_detects_mbr() {
    let mut disk = vec![0u8; 512 * 65];
    disk[510] = 0x55;
    disk[511] = 0xAA;
    disk[450] = 0x83; // Linux partition type in entry 1
    let f = write_temp(&disk);
    assert_eq!(
        probe_format(f.path()).unwrap(),
        ImageFormat::PartitionedDisk {
            table: PartitionTableKind::Mbr
        }
    );
}

#[test]
fn probe_format_detects_iso_and_hybrid() {
    let iso = build_iso(&[Entry::file("/file.txt", b"x".to_vec())]);
    let f = write_temp(&iso);
    assert_eq!(probe_format(f.path()).unwrap(), ImageFormat::Iso9660);

    // Hybrid: CD001 at sector 16 + protective MBR at sector 0.
    let mut hybrid = iso.clone();
    hybrid[450] = 0xEE;
    hybrid[510] = 0x55;
    hybrid[511] = 0xAA;
    let f = write_temp(&hybrid);
    assert!(matches!(
        probe_format(f.path()).unwrap(),
        ImageFormat::IsoHybrid { .. }
    ));
}

#[test]
fn probe_format_detects_udf() {
    let mut disk = vec![0u8; 512 * 65];
    disk[16 * 2048..16 * 2048 + 5].copy_from_slice(b"BEA01");
    let f = write_temp(&disk);
    assert_eq!(probe_format(f.path()).unwrap(), ImageFormat::Udf);
}

#[test]
fn probe_format_detects_raw() {
    let disk = vec![0u8; 512 * 65];
    let f = write_temp(&disk);
    assert_eq!(probe_format(f.path()).unwrap(), ImageFormat::Raw);
}

#[test]
fn inspect_detects_debian_installer() {
    let image = build_iso(&[
        Entry::file("/install.amd/vmlinuz", b"v".to_vec()),
        Entry::file("/install.amd/initrd.gz", b"i".to_vec()),
    ]);
    let f = write_temp(&image);
    let inspected = inspect(f.path(), &ProbeOpts::default()).unwrap();
    assert_eq!(inspected.os, OsClass::Linux(LinuxFlavor::DebianInstaller));
}

#[test]
fn inspect_detects_ubuntu_casper() {
    let image = build_iso(&[
        Entry::file("/casper/vmlinuz", b"v".to_vec()),
        Entry::file("/casper/initrd", b"i".to_vec()),
    ]);
    let f = write_temp(&image);
    let inspected = inspect(f.path(), &ProbeOpts::default()).unwrap();
    assert_eq!(inspected.os, OsClass::Linux(LinuxFlavor::UbuntuCasper));
}

#[test]
fn inspect_detects_windows_pe() {
    let image = build_iso(&[
        Entry::file("/bootmgr.efi", b"mgr".to_vec()),
        Entry::file("/sources/boot.wim", b"boot".to_vec()),
    ]);
    let f = write_temp(&image);
    let inspected = inspect(f.path(), &ProbeOpts::default()).unwrap();
    match inspected.os {
        OsClass::Windows {
            is_installer,
            boot_wim,
            install_wim_size,
        } => {
            assert!(!is_installer);
            assert!(boot_wim);
            assert_eq!(install_wim_size, None);
        }
        other => panic!("expected Windows, got {other:?}"),
    }
}

#[test]
fn inspect_detects_windows_installer_with_size() {
    let image = build_iso(&[
        Entry::file("/bootmgr.efi", b"mgr".to_vec()),
        Entry::file("/sources/boot.wim", b"boot".to_vec()),
        Entry::file("/sources/install.wim", vec![0u8; 100]),
    ]);
    let f = write_temp(&image);
    let inspected = inspect(f.path(), &ProbeOpts::default()).unwrap();
    match inspected.os {
        OsClass::Windows {
            is_installer,
            install_wim_size,
            ..
        } => {
            assert!(is_installer);
            assert_eq!(install_wim_size, Some(100));
        }
        other => panic!("expected Windows, got {other:?}"),
    }
}

#[test]
fn windows_plan_split_when_wim_over_4gib() {
    use hal9001::backend::image_probe::windows_provision_plan;
    let small = OsClass::Windows {
        is_installer: true,
        boot_wim: true,
        install_wim_size: Some(1_000_000),
    };
    assert_eq!(
        windows_provision_plan(&small),
        WindowsPlan::ExtractAllToFat32
    );

    let big = OsClass::Windows {
        is_installer: true,
        boot_wim: true,
        install_wim_size: Some(5 * 1024 * 1024 * 1024),
    };
    assert!(matches!(
        windows_provision_plan(&big),
        WindowsPlan::SplitWim { .. }
    ));
}

#[test]
fn inspect_detects_partitioned_image() {
    let (dir, _) = build_gpt_fat_disk(&[("/EFI/debian/grub.cfg", b"grub".to_vec())]);
    let disk = dir.path().join("disk.img");
    let inspected = inspect(&disk, &ProbeOpts::default()).unwrap();
    match inspected.os {
        OsClass::PartitionedImage { partitions } => {
            assert!(!partitions.is_empty());
            let mut hints: Vec<String> = partitions
                .iter()
                .flat_map(|p| p.boot_hints.iter().cloned())
                .collect();
            hints.retain(|h| h.eq_ignore_ascii_case("/EFI/debian/grub.cfg"));
            assert_eq!(hints.len(), 1, "expected /EFI/debian/grub.cfg boot hint");
        }
        other => panic!("expected PartitionedImage, got {other:?}"),
    }
}
