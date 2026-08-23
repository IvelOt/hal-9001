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

- `BOOTX64.EFI` — **PLACEHOLDER, NOT A REAL BOOTLOADER**. This repository
  cannot build/vendor a real signed GRUB UEFI binary (that requires the
  GRUB toolchain, target platform images, and — for Secure Boot support —
  a signing step outside the scope of this codebase). The file committed
  here is a small stub so `prepare_multiboot`'s file-copy logic and its
  tests have a concrete artifact to exercise.
  **Before cutting a release, replace this file with a real
  `grubx64.efi`/`BOOTX64.EFI`** built with, e.g.:

  ```sh
  grub-mkstandalone \
    --format=x86_64-efi \
    --output=BOOTX64.EFI \
    --modules="part_gpt part_msdos fat iso9660 loopback chain regexp search normal" \
    "boot/grub/grub.cfg=assets/multiboot/grub.cfg"
  ```

  or by copying the `grubx64.efi` shipped by your distro's `grub-efi-amd64`
  / `grub2-efi-x64` package and renaming it to `BOOTX64.EFI`.

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
