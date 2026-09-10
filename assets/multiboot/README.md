# assets/multiboot

Files embedded by `prepare_multiboot` (`src/backend/multiboot.rs`) onto the
data partition of a drive prepared for lightweight multi-boot from the
Storage tab (key `B` — "Preparar Multi-Boot").

## Files

- `grub.cfg` — **real, working GRUB configuration**. Copied verbatim to
  `<mount>/boot/grub/grub.cfg`. Scans `<mount>/ISOs/*.iso` at boot time and
  builds a boot menu entry per image via the `loopback` module (no
  extraction needed). See the comments inside the file for the exact
  boot strategy and how to add a per-distro override.

- `BOOTX64.EFI` — **real GRUB UEFI standalone binary** (x86_64-efi,
  PE32+ executable). Generated with `grub-mkstandalone` via Docker and
  embedded at compile time via `include_bytes!`. Contains the following
  GRUB modules: `part_gpt part_msdos fat iso9660 loopback chain regexp
  search normal`. This binary boots on any UEFI firmware and loads the
  embedded `grub.cfg` to present the multi-boot ISO menu.

## MBR partition table

When the user formats an **entire disk** (e.g. `/dev/sdb`) as FAT32 from the
Storage tab, the HAL-9001 automatically writes an MBR partition table with a
single bootable FAT32 (LBA) partition before formatting. This prevents the
"superfloppy" problem where BIOS/UEFI firmware cannot recognize the device
as bootable. The partition starts at sector 2048 (1 MiB aligned) for
optimal performance. See `create_mbr_fat32` in `src/backend/storage.rs`.

## Deployed layout

```
<mount>/
├── EFI/BOOT/BOOTX64.EFI     <- from assets/multiboot/BOOTX64.EFI
├── boot/grub/grub.cfg       <- from assets/multiboot/grub.cfg
└── ISOs/
    ├── .hal9001-multiboot   <- marker file, written by prepare_multiboot
    └── *.iso / *.img        <- user-managed via the in-app ISO manager (key G)
```

`prepare_multiboot` never touches pre-existing files under `ISOs/` other
than the marker — user ISOs are always left alone.
