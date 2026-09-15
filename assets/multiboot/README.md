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
  embedded at compile time via `include_bytes!`. It bundles the full
  x86_64-efi module set (~260 `.mod` files), which crucially includes the
  filesystem drivers the multi-boot needs to read the data partition:
  `fat exfat ntfs ext2 iso9660`, plus `part_gpt part_msdos loopback chain
  regexp search normal`. This is why the binary is ~6 MB. It boots on any
  UEFI firmware and loads the embedded `grub.cfg` to present the multi-boot
  ISO menu.

  Because `exfat`/`ntfs`/`ext2` are embedded, GRUB can read a large exFAT
  (or NTFS/ext) **data partition** holding ISOs > 4 GiB — the FAT32 4 GiB
  per-file limit only applies to the small ESP that carries this binary.

- `themes/hal9001/` — **HAL-9001 retro-futuristic ASCII terminal theme**.
  A custom GRUB boot menu theme with a terminal aesthetic inspired by
  the HAL-9001 interface. All files are embedded at compile time via
  `include_bytes!` / `include_str!` and written to the drive during
  `prepare_multiboot`.

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
├── EFI/BOOT/BOOTX64.EFI           <- from assets/multiboot/BOOTX64.EFI
├── boot/grub/
│   ├── grub.cfg                   <- from assets/multiboot/grub.cfg
│   └── themes/hal9001/            <- retro-futuristic ASCII terminal theme
│       ├── theme.txt
│       ├── background.png
│       ├── ascii.pf2
│       ├── unicode.pf2
│       ├── select_c.png
│       ├── select_w.png
│       └── select_e.png
└── ISOs/
    ├── .hal9001-multiboot         <- marker file, written by prepare_multiboot
    └── *.iso / *.img              <- user-managed via the in-app ISO manager (key G)
```

`prepare_multiboot` never touches pre-existing files under `ISOs/` other
than the marker — user ISOs are always left alone.

## Dual-partition (Ventoy-style) layout

Formatting an entire disk with the **Multi-Boot (exFAT + ESP)** option
(`fs_type = "multiboot-dual"`) writes a GPT table (see `create_gpt_dual` in
`src/backend/storage.rs`) with two partitions:

```
Partition 1  exFAT   HAL9001-DATA   bulk of the disk   <- ISOs (any size) + user files
Partition 2  FAT32   HAL9001-ESP    128 MiB at the end <- EFI System Partition (ESP)
```

`prepare_multiboot_dual` then lays the files out so each is on the partition
that can actually serve it:

- **ESP (FAT32)** — `EFI/BOOT/BOOTX64.EFI` (the only file UEFI firmware reads,
  which is why it must be FAT), plus a copy of `boot/grub/grub.cfg`.
- **Data (exFAT = `mb_root`)** — `ISOs/` + `.hal9001-multiboot` marker, the
  theme under `boot/grub/themes/hal9001/`, and `boot/grub/grub.cfg`. The bundled
  `grub.cfg` `search`es for the marker to set `$mb_root`, then loads the theme
  and ISOs relative to it, so everything it references at runtime lives here.

This lifts the FAT32 4 GiB per-file limit for ISOs/movies while keeping a
firmware-readable FAT ESP for the bootloader.
