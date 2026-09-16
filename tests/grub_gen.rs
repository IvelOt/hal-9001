use hal9001::backend::grub_gen::{
    generic_fallback, grub_for_linux, grub_for_partitioned, grub_for_windows,
};
use hal9001::backend::image_probe::{LinuxFlavor, PartitionProbe, PartitionTableKind};

#[test]
fn linux_debian_stanza_is_deterministic_and_direct() {
    let s = grub_for_linux("debian-13.5-amd64.iso", LinuxFlavor::DebianInstaller);
    assert!(s.contains("loopback loop"));
    assert!(s.contains("linux /install.amd/vmlinuz findiso=/ISOs/debian-13.5-amd64.iso"));
    assert!(s.contains("initrd /install.amd/initrd.gz"));
    assert!(
        !s.contains("elif"),
        "no boot-time cascade for a known flavor"
    );
}

#[test]
fn linux_casper_stanza_uses_iso_scan() {
    let s = grub_for_linux("ubuntu.iso", LinuxFlavor::UbuntuCasper);
    assert!(s.contains("boot=casper iso-scan/filename=/ISOs/ubuntu.iso"));
}

#[test]
fn linux_unknown_uses_generic_fallback() {
    let s = grub_for_linux("mystery.iso", LinuxFlavor::Unknown);
    assert!(
        s.contains("elif"),
        "generic fallback keeps the legacy cascade"
    );
    assert_eq!(s, generic_fallback("mystery.iso"));
}

#[test]
fn grub_internal_uses_configfile() {
    let s = grub_for_linux("nixos.iso", LinuxFlavor::GrubInternal);
    assert!(s.contains("configfile /EFI/BOOT/grub.cfg"));
}

#[test]
fn partitioned_stanza_roots_into_gpt_partition() {
    let parts = vec![PartitionProbe {
        index: 1,
        table: PartitionTableKind::Gpt,
        fs_hint: "fat32".to_string(),
        start_lba: 2048,
        size: 1024,
        boot_hints: vec!["/EFI/debian/grub.cfg".to_string()],
    }];
    let s = grub_for_partitioned("tails.img", &parts);
    assert!(s.contains("(loop,gpt1)"));
    assert!(s.contains("configfile /EFI/debian/grub.cfg"));
}

#[test]
fn partitioned_stanza_roots_into_mbr_partition() {
    let parts = vec![PartitionProbe {
        index: 2,
        table: PartitionTableKind::Mbr,
        fs_hint: "fat32".to_string(),
        start_lba: 0,
        size: 1024,
        boot_hints: vec!["/EFI/boot/grub.cfg".to_string()],
    }];
    let s = grub_for_partitioned("rescue.img", &parts);
    assert!(s.contains("(loop,2)"));
}

#[test]
fn windows_stanza_chainloads_bootmgr() {
    let s = grub_for_windows();
    assert!(s.contains("chainloader ($mb_root)/EFI/Microsoft/Boot/bootmgfw.efi"));
    assert!(s.contains("boot"));
}
