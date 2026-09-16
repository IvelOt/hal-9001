//! Pure-Rust image inspection: physical format detection (magic bytes) and
//! operating-system classification.
//!
//! This is the deterministic heart of the multi-boot provisioning pipeline. It
//! receives a `PathBuf` and returns an [`InspectedImage`] without touching
//! D-Bus, the network, or any external C library — only `std::fs` and the
//! [`crate::backend::iso_reader::IsoReader`] for ISO9660, plus `fatfs` for
//! listing FAT partitions inside partitioned disk images.

use std::path::{Path, PathBuf};

use crate::backend::iso_reader::IsoReader;

const SECTOR: u64 = 512;
/// Maximum file size representable on FAT32 (4 GiB - 1 byte).
pub const FAT32_MAX_FILE: u64 = 4 * 1024 * 1024 * 1024 - 1;

/// Physical on-disk format of an image, detected from magic bytes only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    /// `"CD001"` at sector 16. Optional El Torito boot catalog.
    Iso9660,
    /// `"BEA01"`/`"NSR0x"`/`"TEA01"` at sector 16 — UDF (or ISO+UDF hybrid).
    Udf,
    /// ISO9660 hybrid image with an appended MBR/GPT partition table.
    IsoHybrid {
        has_gpt: bool,
        has_mbr: bool,
    },
    /// A partitioned disk image (e.g. Tails `.img`).
    PartitionedDisk {
        table: PartitionTableKind,
    },
    /// A "superfloppy" disk image with no partition table.
    Raw,
    Unknown,
}

/// Kind of partition table found in a disk image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PartitionTableKind {
    Mbr,
    Gpt,
    None,
}

/// A partition discovered inside a partitioned disk image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartitionProbe {
    /// 1-based index, matching the GRUB suffix (`(loop,gpt1)` / `(loop,1)`).
    pub index: u32,
    pub table: PartitionTableKind,
    /// Filesystem hint: `"fat32" | "exfat" | "ntfs" | "ext" | "?"`.
    pub fs_hint: String,
    pub start_lba: u64,
    pub size: u64,
    /// Relevant boot paths found inside the partition (e.g. `/EFI/debian/grub.cfg`).
    pub boot_hints: Vec<String>,
}

/// Broad class of operating system detected inside an image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OsClass {
    /// Windows PE (HBCD, WinPE) or a full Windows Installer.
    Windows {
        is_installer: bool,
        boot_wim: bool,
        install_wim_size: Option<u64>,
    },
    /// A Linux Live/Installer ISO9660 image.
    Linux(LinuxFlavor),
    /// A partitioned disk image carrying its own boot configuration.
    PartitionedImage {
        partitions: Vec<PartitionProbe>,
    },
    Unknown,
}

/// Linux family detected from the ISO9660 directory tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinuxFlavor {
    DebianInstaller,
    UbuntuCasper,
    DebianLive,
    ArchLike,
    NixOs,
    GrubInternal,
    Unknown,
}

impl LinuxFlavor {
    /// Short, language-stable technical label (proper nouns / family names)
    /// used in detection toasts.
    pub fn label(self) -> &'static str {
        match self {
            LinuxFlavor::DebianInstaller => "Debian Installer",
            LinuxFlavor::UbuntuCasper => "Ubuntu/Casper",
            LinuxFlavor::DebianLive => "Debian Live",
            LinuxFlavor::ArchLike => "Arch",
            LinuxFlavor::NixOs => "NixOS",
            LinuxFlavor::GrubInternal => "GRUB",
            LinuxFlavor::Unknown => "Linux",
        }
    }
}

/// Options controlling [`inspect`]. The pure reader path is always used; the
/// loop-mount fallback is reserved for future opt-in and is not implemented in
/// v1 (kept here so callers can express the intent).
#[derive(Debug, Clone, Copy, Default)]
pub struct ProbeOpts {
    /// When true, allow a `mount -o loop,ro` sudo fallback if the pure reader
    /// cannot parse the image. Unused in v1 (pure Rust only).
    pub allow_loop_mount: bool,
}

/// The provisioning decision derived from inspection. Stanzas are produced by
/// [`crate::backend::grub_gen`]; this enum binds a class to its plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvisioningPlan {
    /// Loopback + kernel/configfile stanza (Linux ISO).
    LoopbackGrub { stanza: String },
    /// Loopback with an explicit partition (partitioned `.img`).
    LoopbackPartition { stanza: String },
    /// Native extraction of Windows boot files + `chainloader`.
    WindowsExtract { stanza: String, plan: WindowsPlan },
    /// Unrecognized image — use the legacy generic cascade.
    GenericFallback { stanza: String },
}

/// Windows provisioning strategy, decided by `install.wim/esd` size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowsPlan {
    /// Everything fits in FAT32 (the classic WinPE / HBCD case).
    ExtractAllToFat32,
    /// `install.wim` exceeds the 4 GiB FAT32 file limit.
    SplitWim { size: u64 },
    /// No install media detected — only extract the boot chain.
    ExtractBootOnly,
}

/// The full result of inspecting an image.
#[derive(Debug, Clone, PartialEq)]
pub struct InspectedImage {
    pub path: PathBuf,
    pub format: ImageFormat,
    pub os: OsClass,
}

impl InspectedImage {
    /// The provisioning plan for this image, computed by `grub_gen`.
    pub fn provisioning(&self) -> ProvisioningPlan {
        crate::backend::grub_gen::plan_for(self)
    }
}

/// Read `out` (512 bytes) at logical block `lba`.
fn read_sector(file: &mut std::fs::File, lba: u64, out: &mut [u8; 512]) -> std::io::Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    file.seek(SeekFrom::Start(lba * SECTOR))?;
    file.read_exact(out)
}

/// True if `mbr` (a 512-byte sector 0) contains at least one real partition
/// entry (type != 0 and != 0xEE).
fn mbr_has_real_partitions(mbr: &[u8; 512]) -> bool {
    for entry in 0..4 {
        let base = 446 + entry * 16;
        let ptype = mbr[base + 4];
        if ptype != 0 && ptype != 0xEE {
            return true;
        }
    }
    false
}

/// Layer 1: detect the physical format from magic bytes.
pub fn probe_format(path: &Path) -> anyhow::Result<ImageFormat> {
    use std::io::{Read, Seek, SeekFrom};

    let mut f = std::fs::File::open(path)?;
    let mut s0 = [0u8; 512];
    let mut s1 = [0u8; 512];
    let mut pvd = [0u8; 512];
    read_sector(&mut f, 0, &mut s0)?;
    read_sector(&mut f, 1, &mut s1)?;
    // The ISO9660/UDF volume descriptor sits at *logical* sector 16 with a
    // 2048-byte block size, i.e. byte offset 32768 (not 512 * 16).
    f.seek(SeekFrom::Start(16 * 2048))?;
    f.read_exact(&mut pvd)?;

    let is_gpt = &s1[0..8] == b"EFI PART";
    let protective_mbr = s0[450] == 0xEE;
    let mbr_sig = s0[510] == 0x55 && s0[511] == 0xAA;
    let has_mbr_partitions = mbr_sig && mbr_has_real_partitions(&s0);

    let is_iso = &pvd[1..6] == b"CD001";
    let is_udf = &pvd[0..5] == b"BEA01" || &pvd[1..6] == b"NSR02" || &pvd[1..6] == b"NSR03";

    if is_iso || is_udf {
        if protective_mbr || is_gpt || has_mbr_partitions {
            return Ok(ImageFormat::IsoHybrid {
                has_gpt: is_gpt,
                has_mbr: has_mbr_partitions,
            });
        }
        return Ok(if is_udf && !is_iso {
            ImageFormat::Udf
        } else {
            ImageFormat::Iso9660
        });
    }

    if is_gpt || protective_mbr {
        return Ok(ImageFormat::PartitionedDisk {
            table: PartitionTableKind::Gpt,
        });
    }
    if has_mbr_partitions {
        return Ok(ImageFormat::PartitionedDisk {
            table: PartitionTableKind::Mbr,
        });
    }
    Ok(ImageFormat::Raw)
}

/// Inspect an image: physical format + OS classification.
pub fn inspect(path: &Path, _opts: &ProbeOpts) -> anyhow::Result<InspectedImage> {
    let format = probe_format(path)?;
    let os = match format {
        ImageFormat::Iso9660 | ImageFormat::IsoHybrid { .. } | ImageFormat::Udf => {
            classify_iso(path, format)?
        }
        ImageFormat::PartitionedDisk { table } => classify_partitioned(path, table)?,
        _ => OsClass::Unknown,
    };
    Ok(InspectedImage {
        path: path.to_path_buf(),
        format,
        os,
    })
}

fn classify_iso(path: &Path, format: ImageFormat) -> anyhow::Result<OsClass> {
    // UDF-only images can't be listed by the ISO9660 reader; fall back to
    // "Unknown" (the generic GRUB cascade still attempts them at boot).
    if format == ImageFormat::Udf {
        return Ok(OsClass::Unknown);
    }
    let mut reader = IsoReader::open(path)?;
    let paths = reader.list_paths()?;
    if let Some(win) = detect_windows(&paths, &mut reader)? {
        return Ok(OsClass::Windows {
            is_installer: win.0,
            boot_wim: win.1,
            install_wim_size: win.2,
        });
    }
    Ok(OsClass::Linux(detect_linux(&paths)))
}

/// Detect a Windows PE / Installer from the ISO path list.
/// Returns `(is_installer, boot_wim, install_wim_size)`.
fn detect_windows(
    entries: &[String],
    reader: &mut IsoReader,
) -> anyhow::Result<Option<(bool, bool, Option<u64>)>> {
    let lower: Vec<String> = entries.iter().map(|e| e.to_ascii_lowercase()).collect();
    let has = |needle: &str| lower.iter().any(|e| e.contains(needle));

    let bootmgr = has("/bootmgr.efi");
    let bootmgfw = has("/efi/microsoft/boot/bootmgfw.efi");
    let bootx64 = has("/efi/boot/bootx64.efi");
    let boot_wim = has("/sources/boot.wim");
    let install_wim = has("/sources/install.wim");
    let install_esd = has("/sources/install.esd");

    let is_windows = bootmgr
        || bootmgfw
        || (bootx64 && boot_wim)
        || (boot_wim && install_wim)
        || (boot_wim && install_esd);

    if !is_windows {
        return Ok(None);
    }

    let is_installer = install_wim || install_esd;
    let install_wim_size = if install_wim {
        reader.find("/sources/install.wim")?.map(|e| e.len as u64)
    } else if install_esd {
        reader.find("/sources/install.esd")?.map(|e| e.len as u64)
    } else {
        None
    };

    Ok(Some((is_installer, boot_wim, install_wim_size)))
}

/// Detect the Linux flavor from the ISO path list (priority order mirrors the
/// legacy `grub.cfg` cascade).
fn detect_linux(entries: &[String]) -> LinuxFlavor {
    let lower: Vec<String> = entries.iter().map(|e| e.to_ascii_lowercase()).collect();
    let has = |n: &str| lower.iter().any(|e| e.contains(n));

    if has("/install.amd/vmlinuz") && has("/install.amd/initrd.gz") {
        LinuxFlavor::DebianInstaller
    } else if has("/casper/vmlinuz") && has("/casper/initrd") {
        LinuxFlavor::UbuntuCasper
    } else if has("/live/vmlinuz") && has("/live/initrd.img") {
        LinuxFlavor::DebianLive
    } else if has("/arch/boot/x86_64/vmlinuz-linux") {
        LinuxFlavor::ArchLike
    } else if has("/boot/bzimage") && has("/boot/initrd") {
        LinuxFlavor::NixOs
    } else if has("/boot/grub/grub.cfg") || has("/efi/boot/grub.cfg") {
        LinuxFlavor::GrubInternal
    } else {
        LinuxFlavor::Unknown
    }
}

/// Classify a partitioned disk image (`.img`): enumerate its GPT/MBR partitions
/// and, for FAT partitions, list the filesystem to discover boot hints.
fn classify_partitioned(path: &Path, table: PartitionTableKind) -> anyhow::Result<OsClass> {
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    let partitions = match table {
        PartitionTableKind::Gpt => read_gpt_partitions(&mut file)?,
        PartitionTableKind::Mbr => read_mbr_partitions(&mut file)?,
        PartitionTableKind::None => Vec::new(),
    };

    let mut probes = Vec::new();
    for (index, start_lba, size_sectors) in partitions {
        let size = size_sectors * SECTOR;
        let fs_hint = probe_partition_fs(&mut file, start_lba);
        let mut boot_hints = Vec::new();
        if fs_hint == "fat32" || fs_hint == "fat16" || fs_hint == "fat12" {
            boot_hints = list_fat_boot_hints(&mut file, start_lba, size);
        }
        probes.push(PartitionProbe {
            index,
            table,
            fs_hint,
            start_lba,
            size,
            boot_hints,
        });
    }

    if probes.is_empty() {
        Ok(OsClass::Unknown)
    } else {
        Ok(OsClass::PartitionedImage { partitions: probes })
    }
}

/// Read GPT partition entries (index, first_lba, size_sectors).
fn read_gpt_partitions(file: &mut std::fs::File) -> anyhow::Result<Vec<(u32, u64, u64)>> {
    use std::io::{Read, Seek, SeekFrom};

    let mut header = [0u8; 512];
    file.seek(SeekFrom::Start(SECTOR))?;
    file.read_exact(&mut header)?;
    if &header[0..8] != b"EFI PART" {
        return Ok(Vec::new());
    }
    let entries_lba = u64::from_le_bytes(header[72..80].try_into()?);
    let num_entries = u32::from_le_bytes(header[80..84].try_into()?);
    let entry_size = u32::from_le_bytes(header[84..88].try_into()?);

    let mut out = Vec::new();
    let mut index = 0u32;
    for i in 0..num_entries {
        let entry_offset = entries_lba * SECTOR + i as u64 * entry_size as u64;
        let mut entry = vec![0u8; entry_size as usize];
        file.seek(SeekFrom::Start(entry_offset))?;
        file.read_exact(&mut entry)?;
        let first_lba = u64::from_le_bytes(entry[32..40].try_into()?);
        let last_lba = u64::from_le_bytes(entry[40..48].try_into()?);
        // Unused entry (zero type GUID) — skip.
        if first_lba == 0 && last_lba == 0 {
            continue;
        }
        index += 1;
        let size = last_lba.saturating_sub(first_lba) + 1;
        out.push((index, first_lba, size));
    }
    Ok(out)
}

/// Read MBR partition entries (index, first_lba, size_sectors).
fn read_mbr_partitions(file: &mut std::fs::File) -> anyhow::Result<Vec<(u32, u64, u64)>> {
    use std::io::{Read, Seek, SeekFrom};

    let mut mbr = [0u8; 512];
    file.seek(SeekFrom::Start(0))?;
    file.read_exact(&mut mbr)?;
    if mbr[510] != 0x55 || mbr[511] != 0xAA {
        return Ok(Vec::new());
    }

    let mut out = Vec::new();
    for entry in 0..4 {
        let base = 446 + entry * 16;
        let ptype = mbr[base + 4];
        if ptype == 0 || ptype == 0xEE {
            continue;
        }
        let first_lba = u32::from_le_bytes(mbr[base + 8..base + 12].try_into()?) as u64;
        let size = u32::from_le_bytes(mbr[base + 12..base + 16].try_into()?) as u64;
        out.push((entry as u32 + 1, first_lba, size));
    }
    Ok(out)
}

/// Identify the filesystem of a partition from its boot-sector / superblock
/// magic.
fn probe_partition_fs(file: &mut std::fs::File, start_lba: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};

    let mut boot = [0u8; 4096];
    let n = file
        .seek(SeekFrom::Start(start_lba * SECTOR))
        .and_then(|_| file.read(&mut boot))
        .unwrap_or(0);
    if n < 512 {
        return "?".to_string();
    }

    // FAT: signature 0x55AA at 510 + OEM/type string at offset 82.
    if boot[510] == 0x55 && boot[511] == 0xAA {
        let type_str = &boot[82..90];
        if type_str.starts_with(b"FAT32") {
            return "fat32".to_string();
        }
        if type_str.starts_with(b"FAT16") {
            return "fat16".to_string();
        }
        if type_str.starts_with(b"FAT12") {
            return "fat12".to_string();
        }
        // Some FAT variants omit the type string; the signature alone is a
        // strong hint of a FAT family filesystem.
        return "fat".to_string();
    }
    if &boot[3..11] == b"NTFS    " {
        return "ntfs".to_string();
    }
    if &boot[3..11] == b"EXFAT   " {
        return "exfat".to_string();
    }
    // ext2/3/4 superblock magic (0xEF53) at byte offset 1024 + 56 = 1080.
    if n > 1082 && boot[1080] == 0x53 && boot[1081] == 0xEF {
        return "ext".to_string();
    }
    "?".to_string()
}

/// List boot-relevant paths inside a FAT partition using `fatfs` over an offset
/// wrapper (same pattern as `storage::OffsetFile`).
fn list_fat_boot_hints(file: &mut std::fs::File, start_lba: u64, size: u64) -> Vec<String> {
    let offset = start_lba * SECTOR;
    let mut wrapper = OffsetReader::new(file, offset, size);
    let Ok(fs) = fatfs::FileSystem::new(&mut wrapper, fatfs::FsOptions::new()) else {
        return Vec::new();
    };
    let root = fs.root_dir();
    let mut out = Vec::new();
    collect_fat_paths(&root, "/", &mut out, 0);
    out.into_iter()
        .filter(|p| !p.ends_with('/'))
        .filter(|p| {
            let l = p.to_ascii_lowercase();
            l.ends_with("grub.cfg")
                || l.contains("bootmgr")
                || l.contains("vmlinuz")
                || l.contains("initrd")
        })
        .collect()
}

fn collect_fat_paths(
    dir: &fatfs::Dir<&mut OffsetReader<'_>>,
    prefix: &str,
    out: &mut Vec<String>,
    depth: u32,
) {
    if depth > 8 {
        return;
    }
    for entry in dir.iter().flatten() {
        let name = entry.file_name();
        if name == "." || name == ".." {
            continue;
        }
        let path = format!("{prefix}{name}");
        if entry.is_dir() {
            out.push(format!("{path}/"));
            let sub = entry.to_dir();
            collect_fat_paths(&sub, &format!("{path}/"), out, depth + 1);
        } else {
            out.push(path);
        }
    }
}

/// A read/write/seek view of a sub-region of a file, for `fatfs` over a
/// partition inside a disk image (mirrors `storage::OffsetFile`).
struct OffsetReader<'a> {
    inner: &'a mut std::fs::File,
    offset: u64,
    size: u64,
    pos: u64,
}

impl<'a> OffsetReader<'a> {
    fn new(inner: &'a mut std::fs::File, offset: u64, size: u64) -> Self {
        Self {
            inner,
            offset,
            size,
            pos: 0,
        }
    }
}

impl std::io::Read for OffsetReader<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use std::io::{Seek, SeekFrom};
        let remaining = self.size.saturating_sub(self.pos);
        let n = (buf.len() as u64).min(remaining) as usize;
        if n == 0 {
            return Ok(0);
        }
        self.inner.seek(SeekFrom::Start(self.offset + self.pos))?;
        let read = self.inner.read(&mut buf[..n])?;
        self.pos += read as u64;
        Ok(read)
    }
}

impl std::io::Write for OffsetReader<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        use std::io::{Seek, SeekFrom};
        let remaining = self.size.saturating_sub(self.pos);
        let n = (buf.len() as u64).min(remaining) as usize;
        self.inner.seek(SeekFrom::Start(self.offset + self.pos))?;
        let written = self.inner.write(&buf[..n])?;
        self.pos += written as u64;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl std::io::Seek for OffsetReader<'_> {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        use std::io::SeekFrom;
        let new = match pos {
            SeekFrom::Start(n) => n as i128,
            SeekFrom::Current(n) => self.pos as i128 + n as i128,
            SeekFrom::End(n) => self.size as i128 + n as i128,
        };
        if new < 0 || new > self.size as i128 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "seek out of bounds",
            ));
        }
        self.pos = new as u64;
        Ok(self.pos)
    }
}

/// Decide the Windows provisioning strategy from the `install.wim` size.
pub fn windows_provision_plan(os: &OsClass) -> WindowsPlan {
    if let OsClass::Windows {
        install_wim_size: Some(size),
        ..
    } = os
    {
        if *size <= FAT32_MAX_FILE {
            WindowsPlan::ExtractAllToFat32
        } else {
            WindowsPlan::SplitWim { size: *size }
        }
    } else {
        WindowsPlan::ExtractBootOnly
    }
}
