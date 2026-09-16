//! GRUB stanza generation from an inspected image.
//!
//! Turns a classified [`crate::backend::image_probe::OsClass`] into the exact
//! `menuentry` body (a `.cfg` fragment) that the multiboot orchestrator
//! `configfile`s at boot time. Pre-computing the stanza at provisioning time
//! removes the fragile per-boot cascade from the hot path — each image boots
//! via a single deterministic recipe.

use crate::backend::image_probe::{
    InspectedImage, LinuxFlavor, OsClass, PartitionProbe, PartitionTableKind, ProvisioningPlan,
    WindowsPlan,
};

/// Build the full provisioning plan (stanza + strategy) for an image.
pub fn plan_for(img: &InspectedImage) -> ProvisioningPlan {
    let file_name = img
        .path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "image.iso".to_string());

    match &img.os {
        OsClass::Linux(flavor) => ProvisioningPlan::LoopbackGrub {
            stanza: grub_for_linux(&file_name, *flavor),
        },
        OsClass::PartitionedImage { partitions } => ProvisioningPlan::LoopbackPartition {
            stanza: grub_for_partitioned(&file_name, partitions),
        },
        OsClass::Windows { .. } => ProvisioningPlan::WindowsExtract {
            stanza: grub_for_windows(),
            plan: crate::backend::image_probe::windows_provision_plan(&img.os),
        },
        OsClass::Unknown => ProvisioningPlan::GenericFallback {
            stanza: generic_fallback(&file_name),
        },
    }
}

/// Emit a direct, deterministic loopback stanza for a Linux ISO — no cascade.
pub fn grub_for_linux(file_name: &str, flavor: LinuxFlavor) -> String {
    let iso = format!("/ISOs/{file_name}");
    match flavor {
        LinuxFlavor::DebianInstaller => format!(
            r#"set iso_path="($mb_root){iso}"
loopback loop "$iso_path"
set root=(loop)
linux /install.amd/vmlinuz findiso={iso} priority=low vga=788
initrd /install.amd/initrd.gz
loopback --delete loop
"#
        ),
        LinuxFlavor::UbuntuCasper => format!(
            r#"set iso_path="($mb_root){iso}"
loopback loop "$iso_path"
set root=(loop)
linux /casper/vmlinuz boot=casper iso-scan/filename={iso} quiet splash
initrd /casper/initrd
loopback --delete loop
"#
        ),
        LinuxFlavor::DebianLive => format!(
            r#"set iso_path="($mb_root){iso}"
loopback loop "$iso_path"
set root=(loop)
linux /live/vmlinuz boot=live findiso={iso} components quiet splash
initrd /live/initrd.img
loopback --delete loop
"#
        ),
        LinuxFlavor::ArchLike => format!(
            r#"set iso_path="($mb_root){iso}"
loopback loop "$iso_path"
set root=(loop)
linux /arch/boot/x86_64/vmlinuz-linux img_dev=$mb_root img_loop={iso}
initrd /arch/boot/x86_64/initramfs-linux.img
loopback --delete loop
"#
        ),
        LinuxFlavor::NixOs => format!(
            r#"set iso_path="($mb_root){iso}"
loopback loop "$iso_path"
set root=(loop)
linux /boot/bzImage findiso={iso}
initrd /boot/initrd
loopback --delete loop
"#
        ),
        LinuxFlavor::GrubInternal => format!(
            r#"set iso_path="($mb_root){iso}"
loopback loop "$iso_path"
set root=(loop)
if [ -f /EFI/BOOT/grub.cfg ]; then
    configfile /EFI/BOOT/grub.cfg
elif [ -f /boot/grub/grub.cfg ]; then
    configfile /boot/grub/grub.cfg
fi
loopback --delete loop
"#
        ),
        LinuxFlavor::Unknown => generic_fallback(file_name),
    }
}

/// Emit a loopback stanza for a partitioned disk image (`.img`) that roots into
/// an explicit partition — `(loop,gptN)` / `(loop,N)` — and hands control to the
/// boot configuration found inside that partition.
pub fn grub_for_partitioned(file_name: &str, parts: &[PartitionProbe]) -> String {
    let iso = format!("/ISOs/{file_name}");
    let mut stanza = format!(
        r#"set iso_path="($mb_root){iso}"
loopback loop "$iso_path"
"#
    );

    for p in parts {
        let dev = match p.table {
            PartitionTableKind::Gpt => format!("(loop,gpt{})", p.index),
            PartitionTableKind::Mbr => format!("(loop,{})", p.index),
            PartitionTableKind::None => continue,
        };
        for hint in &p.boot_hints {
            let hint = hint.trim_end_matches('/').to_string();
            stanza.push_str(&format!(
                r#"if [ -f {dev}{hint} ]; then
    set root={dev}
    configfile {hint}
    loopback --delete loop
fi
"#,
                dev = dev,
                hint = hint,
            ));
        }
    }

    stanza.push_str("loopback --delete loop\n");
    stanza
}

/// Emit a native `chainloader` stanza for Windows boot files extracted to the
/// real FAT32 partition. The firmware reads the ESP natively, so the boot
/// manager must live there (never on a GRUB loopback).
pub fn grub_for_windows() -> String {
    r#"if [ -f ($mb_root)/EFI/Microsoft/Boot/bootmgfw.efi ]; then
    chainloader ($mb_root)/EFI/Microsoft/Boot/bootmgfw.efi
elif [ -f ($mb_root)/bootmgr.efi ]; then
    chainloader ($mb_root)/bootmgr.efi
elif [ -f ($mb_root)/EFI/BOOT/bootx64.efi ]; then
    chainloader ($mb_root)/EFI/BOOT/bootx64.efi
fi
boot
"#
    .to_string()
}

/// The legacy generic cascade, preserved as the fallback for images the probe
/// could not classify.
pub fn generic_fallback(file_name: &str) -> String {
    format!(
        r#"set iso_path="($mb_root)/ISOs/{file_name}"
loopback loop "$iso_path"
set root=(loop)

if [ -f /install.amd/vmlinuz ]; then
    linux /install.amd/vmlinuz findiso=$iso_path priority=low vga=788
    initrd /install.amd/initrd.gz
elif [ -f /casper/vmlinuz ]; then
    linux /casper/vmlinuz boot=casper iso-scan/filename=$iso_path quiet splash
    initrd /casper/initrd
elif [ -f /live/vmlinuz ]; then
    linux /live/vmlinuz boot=live findiso=$iso_path components quiet splash
    initrd /live/initrd.img
elif [ -f /arch/boot/x86_64/vmlinuz-linux ]; then
    linux /arch/boot/x86_64/vmlinuz-linux img_dev=$mb_root img_loop=$iso_path
    initrd /arch/boot/x86_64/initramfs-linux.img
elif [ -f /boot/bzImage ]; then
    linux /boot/bzImage findiso=$iso_path
    initrd /boot/initrd
elif [ -f /boot/grub/grub.cfg ]; then
    configfile /boot/grub/grub.cfg
elif [ -f /EFI/BOOT/grub.cfg ]; then
    configfile /EFI/BOOT/grub.cfg
fi
loopback --delete loop
"#
    )
}

/// Convenience: the provisioning plan's GRUB stanza.
pub fn stanza_for(plan: &ProvisioningPlan) -> &str {
    match plan {
        ProvisioningPlan::LoopbackGrub { stanza }
        | ProvisioningPlan::LoopbackPartition { stanza }
        | ProvisioningPlan::WindowsExtract { stanza, .. }
        | ProvisioningPlan::GenericFallback { stanza } => stanza,
    }
}

/// Convenience: the Windows provisioning plan embedded in a `WindowsExtract`.
pub fn windows_plan_of(plan: &ProvisioningPlan) -> Option<WindowsPlan> {
    match plan {
        ProvisioningPlan::WindowsExtract { plan, .. } => Some(*plan),
        _ => None,
    }
}
