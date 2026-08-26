use std::path::{Path, PathBuf};

const MARKER_FILE: &str = ".hal9001-multiboot";
const ISOS_DIR: &str = "ISOs";

const BOOTX64_EFI: &[u8] = include_bytes!("../../assets/multiboot/BOOTX64.EFI");

const GRUB_CFG: &str = include_str!("../../assets/multiboot/grub.cfg");

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

pub fn prepare_multiboot(mount_point: &Path) -> anyhow::Result<()> {
    let isos = isos_dir(mount_point);
    std::fs::create_dir_all(&isos)?;

    let efi_dir = mount_point.join("EFI").join("BOOT");
    std::fs::create_dir_all(&efi_dir)?;
    std::fs::write(efi_dir.join("BOOTX64.EFI"), BOOTX64_EFI)?;

    let grub_dir = mount_point.join("boot").join("grub");
    std::fs::create_dir_all(&grub_dir)?;
    std::fs::write(grub_dir.join("grub.cfg"), GRUB_CFG)?;

    let marker = marker_path(mount_point);
    if !marker.is_file() {
        std::fs::write(&marker, b"")?;
    }

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
    fn marker_path_is_under_isos_dir() {
        let mount = Path::new("/mnt/pendrive");
        assert_eq!(
            marker_path(mount),
            Path::new("/mnt/pendrive/ISOs/.hal9001-multiboot")
        );
    }
}
