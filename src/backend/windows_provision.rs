//! Native Windows boot-file extraction into a real FAT32 partition.
//!
//! UEFI `chainloader` requires a *physical* device handle (`EFI_BLOCK_IO`), so a
//! GRUB `(loop)` in memory is never valid for the firmware. The only stable,
//! Secure-Boot-compatible route is to place the Windows boot chain on a real
//! FAT32 ESP that the firmware reads natively. This module does exactly that,
//! reading from an [`IsoReader`] and writing via `std::fs` into the mounted ESP
//! (the same pattern `multiboot.rs` uses to write `BOOTX64.EFI`).

use std::path::Path;

use crate::backend::image_probe::WindowsPlan;
use crate::backend::iso_reader::IsoReader;

/// The Windows boot chain, in order of importance. The boot manager and its
/// dependencies come first; the large `boot.wim`/`install.wim` are extracted
/// last (and only when the plan says so).
const BOOT_CHAIN: &[&str] = &[
    "EFI/Microsoft/Boot/bootmgfw.efi",
    "EFI/Microsoft/Boot/BCD",
    "EFI/Microsoft/Boot/boot.sdi",
    "EFI/Microsoft/Boot/bootfix.bin",
    "EFI/Boot/bootx64.efi",
    "bootmgr.efi",
    "bootmgr",
    "boot/bcd",
    "boot/boot.sdi",
    "boot/bootfix.bin",
    "sources/boot.wim",
];

const INSTALL_SOURCES: &[&str] = &["sources/install.wim", "sources/install.esd"];

/// Result of a Windows extraction.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowsExtraction {
    pub files_written: usize,
    pub bytes_written: u64,
    pub wim_split: bool,
}

/// Extract the Windows boot files from `reader` into the mounted FAT32 ESP at
/// `mount`, following `plan`. `on_progress(bytes_written, total_bytes)` is
/// invoked as each file is copied.
pub fn extract_windows_to_mount(
    reader: &mut IsoReader,
    mount: &Path,
    plan: WindowsPlan,
    mut on_progress: impl FnMut(u64, u64),
) -> anyhow::Result<WindowsExtraction> {
    let mut result = WindowsExtraction {
        wim_split: matches!(plan, WindowsPlan::SplitWim { .. }),
        ..Default::default()
    };

    // 1. Boot chain.
    for rel in BOOT_CHAIN {
        extract_one(reader, mount, rel, &mut result, &mut on_progress)?;
    }

    // 2. install.wim / install.esd — only when it fits in FAT32.
    if plan == WindowsPlan::ExtractAllToFat32 {
        for rel in INSTALL_SOURCES {
            extract_one(reader, mount, rel, &mut result, &mut on_progress)?;
        }
    }

    Ok(result)
}

fn extract_one(
    reader: &mut IsoReader,
    mount: &Path,
    rel: &str,
    result: &mut WindowsExtraction,
    on_progress: &mut impl FnMut(u64, u64),
) -> anyhow::Result<()> {
    let Some(entry) = reader.find(rel)? else {
        return Ok(());
    };
    if entry.is_dir {
        return Ok(());
    }

    let dest = mount.join(rel);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut out = std::fs::File::create(&dest)?;
    let total = entry.len as u64;
    reader.extract(&entry, &mut out, |written, total| {
        on_progress(written, total)
    })?;
    out.sync_all()?;

    result.files_written += 1;
    result.bytes_written += total;
    Ok(())
}
