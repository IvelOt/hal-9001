use std::path::{Path, PathBuf};

const MARKER_FILE: &str = ".hal9001-multiboot";
const ISOS_DIR: &str = "ISOs";

const BOOTX64_EFI: &[u8] = include_bytes!("../../assets/multiboot/BOOTX64.EFI");

const GRUB_CFG: &str = include_str!("../../assets/multiboot/grub.cfg");

const THEME_TXT: &str = include_str!("../../assets/multiboot/themes/hal9001/theme.txt");
const THEME_BACKGROUND: &[u8] =
    include_bytes!("../../assets/multiboot/themes/hal9001/background.png");
const THEME_ASCII: &[u8] = include_bytes!("../../assets/multiboot/themes/hal9001/ascii.pf2");
const THEME_UNICODE: &[u8] = include_bytes!("../../assets/multiboot/themes/hal9001/unicode.pf2");
const THEME_SELECT_C: &[u8] = include_bytes!("../../assets/multiboot/themes/hal9001/select_c.png");
const THEME_SELECT_W: &[u8] = include_bytes!("../../assets/multiboot/themes/hal9001/select_w.png");
const THEME_SELECT_E: &[u8] = include_bytes!("../../assets/multiboot/themes/hal9001/select_e.png");

fn isos_dir(mount_point: &Path) -> PathBuf {
    mount_point.join(ISOS_DIR)
}

fn marker_path(mount_point: &Path) -> PathBuf {
    isos_dir(mount_point).join(MARKER_FILE)
}

pub fn is_multiboot_installed(mount_point: &str) -> bool {
    marker_path(Path::new(mount_point)).is_file()
}

pub fn count_isos(mount_point: &str) -> usize {
    let dir = isos_dir(Path::new(mount_point));
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return 0;
    };
    entries
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| crate::backend::storage::is_iso_or_img(&e.file_name().to_string_lossy()))
        .count()
}

/// Write the UEFI bootloader (`EFI/BOOT/BOOTX64.EFI`). This is the only file
/// the firmware itself must read, so it must live on a FAT (ESP) partition.
fn write_efi_binary(mount_point: &Path) -> anyhow::Result<()> {
    let efi_dir = mount_point.join("EFI").join("BOOT");
    std::fs::create_dir_all(&efi_dir)?;
    std::fs::write(efi_dir.join("BOOTX64.EFI"), BOOTX64_EFI)?;
    Ok(())
}

/// Write `boot/grub/grub.cfg`.
fn write_grub_cfg(mount_point: &Path) -> anyhow::Result<()> {
    let grub_dir = mount_point.join("boot").join("grub");
    std::fs::create_dir_all(&grub_dir)?;
    std::fs::write(grub_dir.join("grub.cfg"), GRUB_CFG)?;
    Ok(())
}

/// Write the HAL-9001 GRUB theme. The bundled `grub.cfg` references the theme
/// as `($mb_root)/boot/grub/themes/hal9001/…`, i.e. relative to whichever
/// partition holds `/ISOs/.hal9001-multiboot`, so the theme must live on the
/// data partition (the same one that holds the ISOs).
fn write_theme(mount_point: &Path) -> anyhow::Result<()> {
    let theme_dir = mount_point
        .join("boot")
        .join("grub")
        .join("themes")
        .join("hal9001");
    std::fs::create_dir_all(&theme_dir)?;
    std::fs::write(theme_dir.join("theme.txt"), THEME_TXT)?;
    std::fs::write(theme_dir.join("background.png"), THEME_BACKGROUND)?;
    std::fs::write(theme_dir.join("ascii.pf2"), THEME_ASCII)?;
    std::fs::write(theme_dir.join("unicode.pf2"), THEME_UNICODE)?;
    std::fs::write(theme_dir.join("select_c.png"), THEME_SELECT_C)?;
    std::fs::write(theme_dir.join("select_w.png"), THEME_SELECT_W)?;
    std::fs::write(theme_dir.join("select_e.png"), THEME_SELECT_E)?;
    Ok(())
}

/// Create the `ISOs/` directory and its `.hal9001-multiboot` marker without
/// disturbing any user ISOs already present.
fn write_isos_marker(mount_point: &Path) -> anyhow::Result<()> {
    let isos = isos_dir(mount_point);
    std::fs::create_dir_all(&isos)?;
    let marker = marker_path(mount_point);
    if !marker.is_file() {
        std::fs::write(&marker, b"")?;
    }
    Ok(())
}

/// Single-partition layout: the boot files, theme, and ISOs all live on one
/// FAT32 partition (which is both the ESP and `mb_root`).
pub fn prepare_multiboot(mount_point: &Path) -> anyhow::Result<()> {
    write_isos_marker(mount_point)?;
    write_efi_binary(mount_point)?;
    write_grub_cfg(mount_point)?;
    write_theme(mount_point)?;
    Ok(())
}

/// Dual-partition (Ventoy-style) layout:
/// - `esp_mount` (FAT32 ESP): the `BOOTX64.EFI` bootloader the firmware loads.
/// - `data_mount` (exFAT/NTFS/ext data partition = `mb_root`): the `ISOs/`
///   directory + marker, the theme, and `grub.cfg` — everything the bundled
///   config resolves relative to `($mb_root)`.
pub fn prepare_multiboot_dual(esp_mount: &Path, data_mount: &Path) -> anyhow::Result<()> {
    write_efi_binary(esp_mount)?;

    write_isos_marker(data_mount)?;
    write_grub_cfg(data_mount)?;
    write_theme(data_mount)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_mount() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn marker_absent_on_fresh_dir() {
        let dir = temp_mount();
        assert!(!is_multiboot_installed(dir.path().to_str().unwrap()));
    }

    #[test]
    fn marker_present_after_prepare() {
        let dir = temp_mount();
        prepare_multiboot(dir.path()).expect("prepare");
        assert!(is_multiboot_installed(dir.path().to_str().unwrap()));
        assert!(dir.path().join("EFI/BOOT/BOOTX64.EFI").is_file());
        assert!(dir.path().join("boot/grub/grub.cfg").is_file());
        assert!(dir.path().join("ISOs/.hal9001-multiboot").is_file());

        // Theme files
        let theme = dir.path().join("boot/grub/themes/hal9001");
        assert!(theme.join("theme.txt").is_file());
        assert!(theme.join("background.png").is_file());
        assert!(theme.join("ascii.pf2").is_file());
        assert!(theme.join("unicode.pf2").is_file());
        assert!(theme.join("select_c.png").is_file());
        assert!(theme.join("select_w.png").is_file());
        assert!(theme.join("select_e.png").is_file());
    }

    #[test]
    fn bootx64_efi_is_a_real_pe32_efi_binary() {
        let efi = BOOTX64_EFI;
        assert!(
            efi.len() > 100_000,
            "EFI binary too small ({}, expected >100KB)",
            efi.len()
        );
        // PE32+ EFI binaries start with "MZ" DOS header
        assert_eq!(
            &efi[0..2],
            b"MZ",
            "BOOTX64.EFI does not start with MZ header"
        );
        // The PE signature offset is at byte 0x3C (60)
        let pe_offset = u32::from_le_bytes([efi[0x3C], efi[0x3D], efi[0x3E], efi[0x3F]]) as usize;
        assert!(pe_offset + 4 <= efi.len(), "PE offset out of bounds");
        assert_eq!(
            &efi[pe_offset..pe_offset + 4],
            b"PE\0\0",
            "PE signature not found at expected offset"
        );
    }

    #[test]
    fn grub_cfg_embedded_matches_source_file() {
        let embedded = GRUB_CFG;
        let source = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/multiboot/grub.cfg"),
        )
        .expect("could not read assets/multiboot/grub.cfg");
        assert_eq!(embedded, source);
    }

    #[test]
    fn count_isos_mixed_file_types() {
        let dir = temp_mount();
        let isos = dir.path().join("ISOs");
        std::fs::create_dir_all(&isos).unwrap();
        std::fs::write(isos.join("debian.iso"), b"x").unwrap();
        std::fs::write(isos.join("Fedora.ISO"), b"x").unwrap();
        std::fs::write(isos.join("rescue.img"), b"x").unwrap();
        std::fs::write(isos.join("notes.txt"), b"x").unwrap();
        std::fs::write(isos.join(".hal9001-multiboot"), b"").unwrap();
        std::fs::create_dir_all(isos.join("subdir.iso")).unwrap();
        assert_eq!(count_isos(dir.path().to_str().unwrap()), 3);
    }

    #[test]
    fn count_isos_missing_dir_is_zero() {
        let dir = temp_mount();
        assert_eq!(count_isos(dir.path().to_str().unwrap()), 0);
    }

    #[test]
    fn prepare_does_not_clobber_existing_isos() {
        let dir = temp_mount();
        let isos = dir.path().join("ISOs");
        std::fs::create_dir_all(&isos).unwrap();
        std::fs::write(isos.join("somefile.iso"), b"user-data-must-survive").unwrap();

        prepare_multiboot(dir.path()).expect("prepare");

        let content = std::fs::read(isos.join("somefile.iso")).unwrap();
        assert_eq!(content, b"user-data-must-survive");
        assert!(is_multiboot_installed(dir.path().to_str().unwrap()));
    }

    #[test]
    fn prepare_is_idempotent() {
        let dir = temp_mount();
        prepare_multiboot(dir.path()).expect("first prepare");

        std::fs::write(dir.path().join("ISOs/my.iso"), b"data").unwrap();
        prepare_multiboot(dir.path()).expect("second prepare");

        assert!(is_multiboot_installed(dir.path().to_str().unwrap()));
        assert_eq!(count_isos(dir.path().to_str().unwrap()), 1);
        assert_eq!(
            std::fs::read(dir.path().join("ISOs/my.iso")).unwrap(),
            b"data"
        );
    }

    #[test]
    fn prepare_dual_splits_files_between_esp_and_data() {
        let esp = temp_mount();
        let data = temp_mount();
        prepare_multiboot_dual(esp.path(), data.path()).expect("prepare dual");

        // ESP holds the firmware-loaded bootloader (must be on FAT).
        assert!(esp.path().join("EFI/BOOT/BOOTX64.EFI").is_file());
        // The ESP must NOT carry grub.cfg or the ISOs/marker.
        assert!(!esp.path().join("boot/grub/grub.cfg").exists());
        assert!(!esp.path().join("ISOs/.hal9001-multiboot").exists());

        // Data partition (mb_root) holds the marker, theme and grub.cfg, since
        // the bundled grub.cfg resolves them relative to ($mb_root).
        assert!(data.path().join("ISOs/.hal9001-multiboot").is_file());
        assert!(data
            .path()
            .join("boot/grub/themes/hal9001/theme.txt")
            .is_file());
        assert!(data.path().join("boot/grub/grub.cfg").is_file());
        assert!(is_multiboot_installed(data.path().to_str().unwrap()));
        // The bootloader binary is not duplicated onto the (exFAT) data part.
        assert!(!data.path().join("EFI/BOOT/BOOTX64.EFI").exists());
    }

    #[test]
    fn prepare_dual_preserves_existing_isos_on_data() {
        let esp = temp_mount();
        let data = temp_mount();
        let isos = data.path().join("ISOs");
        std::fs::create_dir_all(&isos).unwrap();
        std::fs::write(isos.join("bigmovie.iso"), b"keep-me").unwrap();

        prepare_multiboot_dual(esp.path(), data.path()).expect("prepare dual");

        assert_eq!(
            std::fs::read(isos.join("bigmovie.iso")).unwrap(),
            b"keep-me"
        );
        assert_eq!(count_isos(data.path().to_str().unwrap()), 1);
    }

    #[test]
    fn grub_cfg_loads_data_partition_filesystem_modules() {
        // The bundled config must be able to read exFAT/NTFS/ext data
        // partitions, not just FAT, so ISOs > 4 GiB can live off the ESP.
        assert!(GRUB_CFG.contains("insmod exfat"));
        assert!(GRUB_CFG.contains("insmod ntfs"));
        assert!(GRUB_CFG.contains("insmod ext2"));
    }

    #[test]
    fn bootx64_efi_embeds_exfat_and_ntfs_modules() {
        // The standalone bootloader must carry the exFAT/NTFS filesystem
        // drivers, otherwise GRUB cannot read the data partition at boot.
        let has = |needle: &[u8]| BOOTX64_EFI.windows(needle.len()).any(|w| w == needle);
        assert!(has(b"exfat.mod"), "BOOTX64.EFI missing exfat.mod");
        assert!(has(b"ntfs.mod"), "BOOTX64.EFI missing ntfs.mod");
        assert!(has(b"ext2.mod"), "BOOTX64.EFI missing ext2.mod");
    }

    #[test]
    fn marker_path_is_under_isos_dir() {
        let mount = Path::new("/mnt/pendrive");
        assert_eq!(
            marker_path(mount),
            Path::new("/mnt/pendrive/ISOs/.hal9001-multiboot")
        );
    }
}
