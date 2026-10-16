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

/// Build a GPT dual-partition disk image (Ventoy-style): a data partition
/// followed by a FAT32 ESP. Both partitions are formatted FAT32 here (the data
/// partition stands in for exFAT, which cannot be created without root/loop in
/// a unit test) so we can verify the end-to-end GPT + ESP + `search` boot flow.
/// The bootloader lives on the ESP; the marker, theme and a dummy ISO live on
/// the data partition, exactly as `prepare_multiboot_dual` arranges them.
fn create_dual_boot_disk(
    disk_path: &std::path::Path,
    project_dir: &std::path::Path,
) -> hal9001::backend::storage::DualPartitionLayout {
    let disk_size: u64 = 320 * 1024 * 1024;
    let esp_size: u64 = 64 * 1024 * 1024;

    std::fs::File::create(disk_path)
        .and_then(|f| {
            f.set_len(disk_size)?;
            Ok(())
        })
        .expect("create disk image file");

    let layout = hal9001::backend::storage::create_gpt_dual(
        disk_path.to_str().unwrap(),
        disk_size,
        esp_size,
    )
    .expect("create_gpt_dual");

    // Format both partitions FAT32 at their byte offsets.
    hal9001::backend::storage::format_fat32_partition(
        disk_path.to_str().unwrap(),
        "HAL9001ESP",
        layout.esp_offset,
    )
    .expect("format ESP");
    hal9001::backend::storage::format_fat32_partition(
        disk_path.to_str().unwrap(),
        "HAL9001",
        layout.data_offset,
    )
    .expect("format data");

    // --- ESP: bootloader + grub.cfg -------------------------------------
    {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(disk_path)
            .unwrap();
        let mut wrapper = OffsetFile::new(file, layout.esp_offset);
        let fs = fatfs::FileSystem::new(&mut wrapper, fatfs::FsOptions::new()).expect("open ESP");
        let root = fs.root_dir();
        let boot = root.create_dir("EFI").unwrap().create_dir("BOOT").unwrap();
        let efi = std::fs::read(project_dir.join("assets/multiboot/BOOTX64.EFI"))
            .expect("read BOOTX64.EFI");
        boot.create_file("BOOTX64.EFI")
            .unwrap()
            .write_all(&efi)
            .unwrap();
        let grub = root.create_dir("boot").unwrap().create_dir("grub").unwrap();
        let cfg = std::fs::read(project_dir.join("assets/multiboot/grub.cfg")).unwrap();
        grub.create_file("grub.cfg")
            .unwrap()
            .write_all(&cfg)
            .unwrap();
    }

    // --- Data (mb_root): marker + theme + grub.cfg + a dummy ISO ---------
    {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(disk_path)
            .unwrap();
        let mut wrapper = OffsetFile::new(file, layout.data_offset);
        let fs = fatfs::FileSystem::new(&mut wrapper, fatfs::FsOptions::new()).expect("open data");
        let root = fs.root_dir();

        let isos = root.create_dir("ISOs").unwrap();
        isos.create_file(".hal9001-multiboot").unwrap();
        // A dummy ISO so GRUB produces a "Boot:" menu entry from the data part.
        isos.create_file("dummy.iso")
            .unwrap()
            .write_all(b"not a real iso")
            .unwrap();

        let grub = root.create_dir("boot").unwrap().create_dir("grub").unwrap();
        let cfg = std::fs::read(project_dir.join("assets/multiboot/grub.cfg")).unwrap();
        grub.create_file("grub.cfg")
            .unwrap()
            .write_all(&cfg)
            .unwrap();
        let theme = grub
            .create_dir("themes")
            .unwrap()
            .create_dir("hal9001")
            .unwrap();
        for name in ["theme.txt", "ascii.pf2", "unicode.pf2"] {
            let data =
                std::fs::read(project_dir.join(format!("assets/multiboot/themes/hal9001/{name}")))
                    .unwrap();
            theme.create_file(name).unwrap().write_all(&data).unwrap();
        }
    }

    layout
}

#[test]
fn dual_boot_disk_has_valid_gpt_esp_and_data() {
    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let disk_path = std::env::temp_dir().join("hal9001_dual_boot_struct_test.img");

    let layout = create_dual_boot_disk(&disk_path, &project_dir);

    let disk = std::fs::read(&disk_path).unwrap();
    let _ = std::fs::remove_file(&disk_path);

    // Protective MBR + primary/backup GPT signatures.
    assert_eq!(disk[450], 0xEE, "protective MBR type");
    assert_eq!(&disk[512..520], b"EFI PART", "primary GPT signature");
    let last = disk.len() - 512;
    assert_eq!(&disk[last..last + 8], b"EFI PART", "backup GPT signature");

    // Both partitions carry a FAT32 boot sector signature at their offsets.
    let esp = layout.esp_offset as usize;
    assert_eq!(
        u16::from_le_bytes([disk[esp + 510], disk[esp + 511]]),
        0xAA55,
        "ESP FAT signature"
    );
    let data = layout.data_offset as usize;
    assert_eq!(
        u16::from_le_bytes([disk[data + 510], disk[data + 511]]),
        0xAA55,
        "data FAT signature"
    );

    // The bootloader is on the ESP; the marker is on the data partition.
    let esp_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open({
            std::fs::write(std::env::temp_dir().join("hal9001_dual_reopen.img"), &disk).unwrap();
            std::env::temp_dir().join("hal9001_dual_reopen.img")
        })
        .unwrap();
    let mut esp_wrap = OffsetFile::new(esp_file, layout.esp_offset);
    let esp_fs = fatfs::FileSystem::new(&mut esp_wrap, fatfs::FsOptions::new()).unwrap();
    assert!(
        esp_fs.root_dir().open_file("EFI/BOOT/BOOTX64.EFI").is_ok(),
        "ESP missing BOOTX64.EFI"
    );
    assert!(
        esp_fs
            .root_dir()
            .open_file("ISOs/.hal9001-multiboot")
            .is_err(),
        "ESP should not carry the ISO marker"
    );
    drop(esp_fs);
    drop(esp_wrap);

    let data_file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(std::env::temp_dir().join("hal9001_dual_reopen.img"))
        .unwrap();
    let mut data_wrap = OffsetFile::new(data_file, layout.data_offset);
    let data_fs = fatfs::FileSystem::new(&mut data_wrap, fatfs::FsOptions::new()).unwrap();
    assert!(
        data_fs
            .root_dir()
            .open_file("ISOs/.hal9001-multiboot")
            .is_ok(),
        "data partition missing ISO marker"
    );
    drop(data_fs);
    drop(data_wrap);
    let _ = std::fs::remove_file(std::env::temp_dir().join("hal9001_dual_reopen.img"));
}

#[test]
fn qemu_uefi_boot_dual_partition_loads_grub_menu() {
    let ovmf_path = std::path::Path::new("/usr/share/edk2/x64/OVMF.4m.fd");
    if !ovmf_path.exists() {
        eprintln!(
            "OVMF firmware not found at {} -- skipping QEMU dual-boot test",
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
        eprintln!("qemu-system-x86_64 not available -- skipping QEMU dual-boot test");
        return;
    }

    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let disk_path = std::env::temp_dir().join("hal9001_qemu_dual_boot_test.img");
    let serial_log = std::env::temp_dir().join("hal9001_qemu_dual_serial.log");
    let _ = std::fs::remove_file(&serial_log);

    create_dual_boot_disk(&disk_path, &project_dir);

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

    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(25);
    let matched = |content: &str| {
        content.contains("GNU GRUB")
            || content.contains("GRUB")
            || content.contains("HAL-9001")
            || content.contains("Multi-Boot")
            || content.contains("Boot:")
    };
    loop {
        if start.elapsed() >= timeout {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
        if let Ok(content) = std::fs::read_to_string(&serial_log) {
            if matched(&content) {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&disk_path);
                let _ = std::fs::remove_file(&serial_log);
                return;
            }
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    let log_content = std::fs::read_to_string(&serial_log).unwrap_or_default();
    let _ = std::fs::remove_file(&disk_path);
    let _ = std::fs::remove_file(&serial_log);
    eprintln!("Dual-boot serial log:\n{log_content}");
    assert!(
        matched(&log_content),
        "QEMU dual-partition serial log did not contain GRUB menu within 25s.\nLog:\n{log_content}"
    );
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

fn write_both_u16(dst: &mut [u8], val: u16) {
    dst[0..2].copy_from_slice(&val.to_le_bytes());
    dst[2..4].copy_from_slice(&val.to_be_bytes());
}

fn write_both_u32(dst: &mut [u8], val: u32) {
    dst[0..4].copy_from_slice(&val.to_le_bytes());
    dst[4..8].copy_from_slice(&val.to_be_bytes());
}

fn write_iso_dir_record(
    dst: &mut [u8],
    lba: u32,
    data_len: u32,
    is_dir: bool,
    ident: &[u8],
) -> usize {
    let rec_len = (33 + ident.len() + usize::from(ident.len() % 2 == 0)) as u8;
    dst[0] = rec_len;
    dst[1] = 0;
    write_both_u32(&mut dst[2..10], lba);
    write_both_u32(&mut dst[10..18], data_len);
    dst[25] = if is_dir { 0x02 } else { 0x00 };
    write_both_u16(&mut dst[28..32], 1);
    dst[32] = ident.len() as u8;
    dst[33..33 + ident.len()].copy_from_slice(ident);
    rec_len as usize
}

/// Construct a minimal valid ISO9660 image in memory containing `/boot/grub/loopback.cfg`.
fn build_minimal_iso9660_with_loopback_cfg(volume_label: &str, loopback_cfg: &[u8]) -> Vec<u8> {
    const SECTOR: usize = 2048;
    const TOTAL_SECTORS: u32 = 24;
    let mut iso = vec![0u8; TOTAL_SECTORS as usize * SECTOR];

    // Sector 16: Primary Volume Descriptor
    {
        let pvd = &mut iso[16 * SECTOR..17 * SECTOR];
        pvd[0] = 1;
        pvd[1..6].copy_from_slice(b"CD001");
        pvd[6] = 1;
        pvd[8..40].fill(b' ');
        pvd[8..16].copy_from_slice(b"HAL-9001");
        pvd[40..72].fill(b' ');
        let lbl = volume_label.as_bytes();
        let copy_len = lbl.len().min(32);
        pvd[40..40 + copy_len].copy_from_slice(&lbl[..copy_len]);
        write_both_u32(&mut pvd[80..88], TOTAL_SECTORS);
        write_both_u16(&mut pvd[120..124], 1);
        write_both_u16(&mut pvd[124..128], 1);
        write_both_u16(&mut pvd[128..132], SECTOR as u16);
        write_iso_dir_record(&mut pvd[156..190], 18, SECTOR as u32, true, &[0]);
    }

    // Sector 17: Volume Descriptor Set Terminator
    {
        let term = &mut iso[17 * SECTOR..18 * SECTOR];
        term[0] = 255;
        term[1..6].copy_from_slice(b"CD001");
        term[6] = 1;
    }

    // Sector 18: Root directory (`/`) -> contains `BOOT` at LBA 19
    {
        let dir = &mut iso[18 * SECTOR..19 * SECTOR];
        let mut off = 0;
        off += write_iso_dir_record(&mut dir[off..], 18, SECTOR as u32, true, &[0]);
        off += write_iso_dir_record(&mut dir[off..], 18, SECTOR as u32, true, &[1]);
        let _ = write_iso_dir_record(&mut dir[off..], 19, SECTOR as u32, true, b"BOOT");
    }

    // Sector 19: `/boot` directory -> contains `GRUB` at LBA 20
    {
        let dir = &mut iso[19 * SECTOR..20 * SECTOR];
        let mut off = 0;
        off += write_iso_dir_record(&mut dir[off..], 19, SECTOR as u32, true, &[0]);
        off += write_iso_dir_record(&mut dir[off..], 18, SECTOR as u32, true, &[1]);
        let _ = write_iso_dir_record(&mut dir[off..], 20, SECTOR as u32, true, b"GRUB");
    }

    // Sector 20: `/boot/grub` directory -> contains `LOOPBACK.CFG;1` at LBA 21
    {
        let dir = &mut iso[20 * SECTOR..21 * SECTOR];
        let mut off = 0;
        off += write_iso_dir_record(&mut dir[off..], 20, SECTOR as u32, true, &[0]);
        off += write_iso_dir_record(&mut dir[off..], 19, SECTOR as u32, true, &[1]);
        let _ = write_iso_dir_record(
            &mut dir[off..],
            21,
            loopback_cfg.len() as u32,
            false,
            b"LOOPBACK.CFG;1",
        );
    }

    // Sector 21: `/boot/grub/loopback.cfg` file content
    {
        let dst = &mut iso[21 * SECTOR..21 * SECTOR + loopback_cfg.len()];
        dst.copy_from_slice(loopback_cfg);
    }

    iso
}

#[test]
fn qemu_uefi_boot_executes_iso_loopback_without_net_or_device_errors() {
    let ovmf_path = std::path::Path::new("/usr/share/edk2/x64/OVMF.4m.fd");
    if !ovmf_path.exists() {
        eprintln!(
            "OVMF firmware not found at {} -- skipping QEMU ISO loopback test",
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
        eprintln!("qemu-system-x86_64 not available -- skipping QEMU ISO loopback test");
        return;
    }

    let project_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let disk_path = std::env::temp_dir().join("hal9001_qemu_iso_loopback_test.img");
    let serial_log = std::env::temp_dir().join("hal9001_qemu_iso_loopback_serial.log");
    let _ = std::fs::remove_file(&serial_log);

    let layout = create_dual_boot_disk(&disk_path, &project_dir);

    // Replace dummy.iso on the data partition with a real ISO9660 image containing
    // `/boot/grub/loopback.cfg`, and switch grub.cfg to serial + timeout=1 so GRUB
    // automatically boots Entry 0 (`Boot: dummy`) and executes `(loop)/boot/grub/loopback.cfg`.
    let loopback_cfg = br#"
echo "HAL9001_LOOPBACK_OK path=$iso_path label=$iso_label uuid=$mb_uuid imgdev=$imgdevpath"
"#;
    let iso_bytes = build_minimal_iso9660_with_loopback_cfg("HAL9001_ISO", loopback_cfg);

    let raw_grub_cfg =
        std::fs::read_to_string(project_dir.join("assets/multiboot/grub.cfg")).unwrap();
    let automated_grub_cfg = raw_grub_cfg
        .replace("set timeout=15", "set timeout=1")
        .replace(
            "terminal_output gfxterm",
            "serial --unit=0 --speed=115200\nterminal_input serial console\nterminal_output serial console",
        );

    for offset in [layout.esp_offset, layout.data_offset] {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&disk_path)
            .unwrap();
        let mut wrapper = OffsetFile::new(file, offset);
        let fs = fatfs::FileSystem::new(&mut wrapper, fatfs::FsOptions::new()).unwrap();
        let root = fs.root_dir();
        let mut cfg_file = root.open_file("boot/grub/grub.cfg").unwrap();
        cfg_file.truncate().unwrap();
        cfg_file.write_all(automated_grub_cfg.as_bytes()).unwrap();
        if offset == layout.data_offset {
            let mut iso_file = root.open_file("ISOs/dummy.iso").unwrap();
            iso_file.truncate().unwrap();
            iso_file.write_all(&iso_bytes).unwrap();
        }
    }

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

    let start = std::time::Instant::now();
    let timeout = std::time::Duration::from_secs(25);
    loop {
        if start.elapsed() >= timeout {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(400));
        if let Ok(content) = std::fs::read_to_string(&serial_log) {
            if content.contains("HAL9001_LOOPBACK_OK") {
                break;
            }
        }
    }

    let _ = child.kill();
    let _ = child.wait();
    let log_content = std::fs::read_to_string(&serial_log).unwrap_or_default();
    let _ = std::fs::remove_file(&disk_path);
    let _ = std::fs::remove_file(&serial_log);

    assert!(
        log_content.contains("HAL9001_LOOPBACK_OK path=/ISOs/dummy.iso label=HAL9001_ISO"),
        "Expected loopback.cfg inside ISO to execute with relative iso_path and probed iso_label.\nSerial log:\n{log_content}"
    );
    assert!(
        !log_content.contains("no server is specified"),
        "GRUB emitted 'no server is specified' error!\nSerial log:\n{log_content}"
    );
    assert!(
        !log_content.contains("you need to load the kernel first"),
        "GRUB emitted 'you need to load the kernel first' error!\nSerial log:\n{log_content}"
    );
    assert!(
        !log_content.contains("device not found"),
        "GRUB emitted 'device not found' error!\nSerial log:\n{log_content}"
    );
}
