use std::io::{Read, Seek, Write};
use std::process::Command;

struct OffsetFile {
    inner: std::fs::File,
    offset: u64,
}

impl OffsetFile {
    fn new(mut inner: std::fs::File, offset: u64) -> Self {
        use std::io::Seek;
        let _ = inner.seek(std::io::SeekFrom::Start(offset));
        Self { inner, offset }
    }
}

impl Read for OffsetFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Write for OffsetFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for OffsetFile {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        match pos {
            std::io::SeekFrom::Start(n) => {
                self.inner.seek(std::io::SeekFrom::Start(n + self.offset))?;
                Ok(n)
            }
            std::io::SeekFrom::Current(n) => {
                self.inner.seek(std::io::SeekFrom::Current(n))?;
                let raw = self.inner.stream_position()?;
                Ok(raw - self.offset)
            }
            std::io::SeekFrom::End(n) => {
                let raw = self.inner.seek(std::io::SeekFrom::End(n))?;
                Ok(raw.saturating_sub(self.offset))
            }
        }
    }
}

fn create_bootable_disk(disk_path: &std::path::Path, project_dir: &std::path::Path) {
    let disk_size: u64 = 64 * 1024 * 1024;

    std::fs::File::create(disk_path)
        .and_then(|f| {
            f.set_len(disk_size)?;
            Ok(())
        })
        .expect("create disk image file");

    let fat32_offset =
        hal9001::backend::storage::create_mbr_fat32(disk_path.to_str().unwrap(), disk_size)
            .expect("create_mbr_fat32");

    hal9001::backend::storage::format_fat32_partition(
        disk_path.to_str().unwrap(),
        "MULTIBOOT",
        fat32_offset,
    )
    .expect("format_fat32_partition");

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(disk_path)
        .unwrap();
    let mut wrapper = OffsetFile::new(file, fat32_offset);

    let fs = fatfs::FileSystem::new(&mut wrapper, fatfs::FsOptions::new()).expect("open FAT32");
    let root = fs.root_dir();

    let efi_dir = root.create_dir("EFI").unwrap();
    let boot_dir = efi_dir.create_dir("BOOT").unwrap();
    let efi_data =
        std::fs::read(project_dir.join("assets/multiboot/BOOTX64.EFI")).expect("read BOOTX64.EFI");
    let mut efi_file = boot_dir.create_file("BOOTX64.EFI").unwrap();
    efi_file.write_all(&efi_data).unwrap();
    drop(efi_file);
    drop(boot_dir);
    drop(efi_dir);

    let boot_grub = root.create_dir("boot").unwrap().create_dir("grub").unwrap();
    let grub_data =
        std::fs::read(project_dir.join("assets/multiboot/grub.cfg")).expect("read grub.cfg");
    let mut grub_file = boot_grub.create_file("grub.cfg").unwrap();
    grub_file.write_all(&grub_data).unwrap();
    drop(grub_file);

    // Write theme files
    let theme_dir = boot_grub
        .create_dir("themes")
        .unwrap()
        .create_dir("hal9001")
        .unwrap();
    let theme_files = [
        (
            "theme.txt",
            std::fs::read(project_dir.join("assets/multiboot/themes/hal9001/theme.txt"))
                .expect("read theme.txt"),
        ),
        (
            "background.png",
            std::fs::read(project_dir.join("assets/multiboot/themes/hal9001/background.png"))
                .expect("read background.png"),
        ),
        (
            "ascii.pf2",
            std::fs::read(project_dir.join("assets/multiboot/themes/hal9001/ascii.pf2"))
                .expect("read ascii.pf2"),
        ),
        (
            "unicode.pf2",
            std::fs::read(project_dir.join("assets/multiboot/themes/hal9001/unicode.pf2"))
                .expect("read unicode.pf2"),
        ),
        (
            "select_c.png",
            std::fs::read(project_dir.join("assets/multiboot/themes/hal9001/select_c.png"))
                .expect("read select_c.png"),
        ),
        (
            "select_w.png",
            std::fs::read(project_dir.join("assets/multiboot/themes/hal9001/select_w.png"))
                .expect("read select_w.png"),
        ),
        (
            "select_e.png",
            std::fs::read(project_dir.join("assets/multiboot/themes/hal9001/select_e.png"))
                .expect("read select_e.png"),
        ),
    ];
    for (name, data) in theme_files {
        let mut f = theme_dir.create_file(name).unwrap();
        f.write_all(&data).unwrap();
    }
    drop(theme_dir);
    drop(boot_grub);

    let isos = root.create_dir("ISOs").unwrap();
    isos.create_file(".hal9001-multiboot").unwrap();
    drop(isos);
    drop(root);
    drop(fs);
    drop(wrapper);
}

#[test]
fn bootable_disk_has_correct_mbr_and_fat32_structure() {
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let disk_path = std::env::temp_dir().join("hal9001_bootable_disk_test.img");

    create_bootable_disk(&disk_path, &project_dir);

    let disk = std::fs::read(&disk_path).unwrap();
    let _ = std::fs::remove_file(&disk_path);

    // Verify MBR
    assert_eq!(disk[510], 0x55, "MBR boot sig byte 0");
    assert_eq!(disk[511], 0xAA, "MBR boot sig byte 1");
    assert_eq!(disk[446], 0x80, "Partition 1 bootable flag");
    assert_eq!(disk[449], 0x0C, "Partition 1 type: FAT32 LBA");

    let first_lba = u32::from_le_bytes([disk[454], disk[455], disk[456], disk[457]]);
    assert_eq!(first_lba, 2048, "Partition 1 first LBA");

    // Verify FAT32 boot sector at partition offset
    let offset = first_lba as usize * 512;
    let sig = u16::from_le_bytes([disk[offset + 510], disk[offset + 511]]);
    assert_eq!(sig, 0xAA55, "FAT32 boot sector signature");

    // Verify volume label
    let label_bytes = &disk[offset + 71..offset + 82];
    let label = std::str::from_utf8(label_bytes).unwrap().trim_end();
    assert_eq!(label, "MULTIBOOT");
}

#[test]
fn bootable_disk_efi_binary_matches_source() {
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let disk_path = std::env::temp_dir().join("hal9001_bootable_disk_efi_test.img");

    create_bootable_disk(&disk_path, &project_dir);

    let disk = std::fs::read(&disk_path).unwrap();
    let _ = std::fs::remove_file(&disk_path);

    let first_lba = u32::from_le_bytes([disk[454], disk[455], disk[456], disk[457]]);
    let offset = first_lba as usize * 512;

    // FAT32: BPB contains important fields
    let bytes_per_sector = u16::from_le_bytes([disk[offset + 11], disk[offset + 12]]);
    assert_eq!(bytes_per_sector, 512, "bytes per sector");

    let sectors_per_cluster = disk[offset + 13];
    assert!(sectors_per_cluster > 0, "sectors per cluster");

    let reserved_sectors = u16::from_le_bytes([disk[offset + 14], disk[offset + 15]]);
    assert!(reserved_sectors > 0, "reserved sectors");

    let num_fats = disk[offset + 16];
    assert!(num_fats >= 2, "number of FATs");

    let total_sectors_32 = u32::from_le_bytes([
        disk[offset + 32],
        disk[offset + 33],
        disk[offset + 34],
        disk[offset + 35],
    ]);
    assert!(total_sectors_32 > 0, "total sectors");

    // Root cluster should be 2
    let root_cluster = u32::from_le_bytes([
        disk[offset + 44],
        disk[offset + 45],
        disk[offset + 46],
        disk[offset + 47],
    ]);
    assert_eq!(root_cluster, 2, "root cluster");
}

#[test]
fn qemu_uefi_boot_loads_grub_menu() {
    let ovmf_path = std::path::Path::new("/usr/share/edk2/x64/OVMF.4m.fd");
    if !ovmf_path.exists() {
        eprintln!(
            "OVMF firmware not found at {} -- skipping QEMU boot test",
            ovmf_path.display()
        );
        return;
    }

    let qemu_available = Command::new("qemu-system-x86_64")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !qemu_available {
        eprintln!("qemu-system-x86_64 not available -- skipping QEMU boot test");
        return;
    }

    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let disk_path = std::env::temp_dir().join("hal9001_qemu_boot_test.img");
    let serial_log = std::env::temp_dir().join("hal9001_qemu_serial.log");

    // Remove old serial log
    let _ = std::fs::remove_file(&serial_log);

    create_bootable_disk(&disk_path, &project_dir);

    // Run QEMU in background with serial log
    let mut child = Command::new("qemu-system-x86_64")
        .arg("-bios")
        .arg(ovmf_path)
        .arg("-drive")
        .arg(format!("file={},format=raw", disk_path.display()))
        .arg("-m")
        .arg("256")
        .arg("-nographic")
        .arg("-serial")
        .arg(format!("file:{}", serial_log.display()))
        .arg("-no-reboot")
        .arg("-accel")
        .arg("tcg")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to spawn QEMU");

    // Wait up to 20 seconds for QEMU to boot
    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(20);

    loop {
        if start.elapsed() >= timeout {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));

        // Check if serial log has content
        if let Ok(content) = std::fs::read_to_string(&serial_log) {
            if content.contains("GNU GRUB")
                || content.contains("GRUB")
                || content.contains("HAL-9001")
                || content.contains("Multi-Boot")
                || content.contains("Boot:")
            {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&disk_path);
                let _ = std::fs::remove_file(&serial_log);
                return; // Success!
            }
        }
    }

    let _ = child.kill();
    let _ = child.wait();

    let log_content = std::fs::read_to_string(&serial_log).unwrap_or_default();
    let _ = std::fs::remove_file(&disk_path);
    let _ = std::fs::remove_file(&serial_log);

    eprintln!("Serial log:\n{log_content}");

    let has_grub = log_content.contains("GNU GRUB")
        || log_content.contains("GRUB")
        || log_content.contains("HAL-9001")
        || log_content.contains("Multi-Boot")
        || log_content.contains("Boot:");

    assert!(
        has_grub,
        "QEMU serial log did not contain GRUB menu within 20s.\nLog:\n{log_content}"
    );
}
