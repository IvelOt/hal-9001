//! Pure-Rust ISO9660 / Joliet reader.
//!
//! Reads the Primary Volume Descriptor (sector 16 / LBA 16, byte offset
//! 32768), walks Directory Records, and extracts file bytes. No external C
//! runtime or dynamic library is involved — only `std::fs` + manual binary
//! parsing, consistent with the project's MBR/GPT/CRC32 handling in
//! `storage.rs`.
//!
//! Scope (v1): ISO9660 *plain* (ASCII identifiers) plus Joliet Supplementary
//! Volume Descriptor (UCS-2BE identifiers). Rock Ridge is intentionally left
//! out: the Windows boot files this reader must locate (`bootmgr.efi`, `EFI/`,
//! `sources/boot.wim`, …) use plain ASCII names that ISO9660 plain already
//! exposes.

use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Default ISO9660 logical block size. Virtually every `.iso` image on disk
/// uses 2048-byte sectors; the PVD lives at logical block 16, i.e. byte 32768.
const DEFAULT_SECTOR_SIZE: u64 = 2048;

/// Signature of every ISO9660 volume descriptor, at bytes 1..=5.
const CD001: &[u8; 5] = b"CD001";

/// Volume descriptor type codes.
const VD_TYPE_PRIMARY: u8 = 1;
const VD_TYPE_SUPPLEMENTARY: u8 = 2;
const VD_TYPE_TERMINATOR: u8 = 255;

/// Joliet escape sequence `%/E` (level 3, UCS-2) at descriptor offset 88.
const JOLIET_ESCAPE: &[u8; 3] = b"%/E";

/// File flag bits from a Directory Record.
const FILE_FLAG_DIRECTORY: u8 = 0x02;
const FILE_FLAG_MULTI_EXTENT: u8 = 0x80;

/// A single contiguous extent (run of sectors) of a file. ISO9660 files can be
/// fragmented into several extents; each is described by its own directory
/// record (with the multi-extent flag set on all but the last).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extent {
    pub lba: u32,
    pub len: u32,
}

/// A directory entry discovered by [`IsoReader`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
    /// Location of the first extent (for a directory, the extent to read).
    pub lba: u32,
    /// Total data length across all extents.
    pub len: u32,
    /// All extents that make up the file, in order. Single-extent files have
    /// exactly one entry here.
    pub extents: Vec<Extent>,
}

impl DirEntry {
    fn root(name: &str, lba: u32, len: u32) -> Self {
        Self {
            name: name.to_string(),
            is_dir: true,
            lba,
            len,
            extents: vec![Extent { lba, len }],
        }
    }
}

/// Handle to an open ISO9660 image.
pub struct IsoReader {
    file: std::fs::File,
    sector_size: u64,
    /// Root directory of the Primary Volume Descriptor (ASCII names).
    root: DirEntry,
    /// Root directory of the Joliet SVD, when present (UCS-2BE names).
    joliet_root: Option<DirEntry>,
    /// Prefer Joliet names when a Supplementary Volume Descriptor exists.
    use_joliet: bool,
}

impl IsoReader {
    /// Open an image, locate the PVD (and, optionally, a Joliet SVD) and read
    /// the root directory record.
    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let mut file = std::fs::File::open(path)?;

        // Read the PVD at sector 16 (offset 32768), assuming 2048-byte blocks.
        let mut pvd = [0u8; DEFAULT_SECTOR_SIZE as usize];
        file.seek(SeekFrom::Start(16 * DEFAULT_SECTOR_SIZE))?;
        file.read_exact(&mut pvd)?;

        if pvd[0] != VD_TYPE_PRIMARY || &pvd[1..6] != CD001 {
            anyhow::bail!("not an ISO9660 image (missing PVD 'CD001' at sector 16)");
        }

        let sector_size = read_both_u16(&pvd, 128) as u64;
        let sector_size = if (512..=4096).contains(&sector_size) {
            sector_size
        } else {
            DEFAULT_SECTOR_SIZE
        };

        let (root_lba, root_len) = parse_root_record(&pvd)?;

        // Scan the volume descriptor sequence for a Joliet SVD. The sequence
        // starts at sector 16 and continues until a Terminator (type 255).
        let mut joliet_root = None;
        let mut sector_index = 17u64;
        let mut descriptor = vec![0u8; sector_size as usize];
        loop {
            file.seek(SeekFrom::Start(sector_index * sector_size))?;
            if file.read_exact(&mut descriptor).is_err() {
                break;
            }
            let vd_type = descriptor[0];
            if vd_type == VD_TYPE_TERMINATOR {
                break;
            }
            if vd_type == VD_TYPE_SUPPLEMENTARY
                && &descriptor[1..6] == CD001
                && &descriptor[88..91] == JOLIET_ESCAPE
            {
                if let Ok((lba, len)) = parse_root_record(&descriptor) {
                    joliet_root = Some(DirEntry::root("", lba, len));
                    break;
                }
            }
            sector_index += 1;
            if sector_index > 64 {
                break;
            }
        }

        Ok(Self {
            file,
            sector_size,
            root: DirEntry::root("", root_lba, root_len),
            use_joliet: joliet_root.is_some(),
            joliet_root,
        })
    }

    /// The root directory entry.
    fn root(&self) -> &DirEntry {
        self.joliet_root.as_ref().unwrap_or(&self.root)
    }

    /// List the immediate children of `dir`.
    pub fn read_dir(&mut self, dir: &DirEntry) -> anyhow::Result<Vec<DirEntry>> {
        if !dir.is_dir {
            anyhow::bail!("{} is not a directory", dir.name);
        }
        let data = self.read_extent(dir.lba, dir.len)?;
        let raw = parse_directory_records(&data, self.use_joliet);
        Ok(group_multi_extent(raw))
    }

    /// Walk the tree and return the full normalized path of every entry, as
    /// `/`-prefixed lowercase-agnostic strings (e.g. `/sources/boot.wim`).
    /// Directories are listed too, with a trailing `/`.
    pub fn list_paths(&mut self) -> anyhow::Result<Vec<String>> {
        let mut out = Vec::new();
        let root = self.root().clone();
        self.walk(&root, "/", &mut out)?;
        Ok(out)
    }

    fn walk(&mut self, dir: &DirEntry, prefix: &str, out: &mut Vec<String>) -> anyhow::Result<()> {
        let children = self.read_dir(dir)?;
        for child in children {
            if child.name == "." || child.name == ".." || child.name.is_empty() {
                continue;
            }
            let path = format!("{prefix}{}", child.name);
            if child.is_dir {
                out.push(format!("{path}/"));
                // Cap recursion depth defensively against cyclic/malformed trees.
                if path.matches('/').count() < 32 {
                    self.walk(&child, &format!("{path}/"), out)?;
                }
            } else {
                out.push(path);
            }
        }
        Ok(())
    }

    /// Locate a file by its `/`-separated, case-insensitive path (without a
    /// leading slash). Returns the entry if found.
    pub fn find(&mut self, path: &str) -> anyhow::Result<Option<DirEntry>> {
        let parts: Vec<&str> = path
            .trim_start_matches('/')
            .split('/')
            .filter(|p| !p.is_empty())
            .collect();
        if parts.is_empty() {
            return Ok(Some(self.root().clone()));
        }
        let mut current = self.root().clone();
        for (i, part) in parts.iter().enumerate() {
            let is_last = i == parts.len() - 1;
            let children = self.read_dir(&current)?;
            let found = children
                .into_iter()
                .find(|c| c.is_dir != is_last && c.name.eq_ignore_ascii_case(part));
            match found {
                Some(entry) if is_last => return Ok(Some(entry)),
                Some(entry) => current = entry,
                None => return Ok(None),
            }
        }
        Ok(None)
    }

    /// Extract `file` to `out`, invoking `on_progress(bytes_written, total)`
    /// as bytes are copied.
    pub fn extract(
        &mut self,
        file: &DirEntry,
        out: &mut impl Write,
        mut on_progress: impl FnMut(u64, u64),
    ) -> anyhow::Result<()> {
        let total = file.len as u64;
        let mut written = 0u64;
        for extent in &file.extents {
            let data = self.read_extent(extent.lba, extent.len)?;
            out.write_all(&data)?;
            written += data.len() as u64;
            on_progress(written, total);
        }
        Ok(())
    }

    /// Read `len` bytes starting at logical block `lba`.
    fn read_extent(&mut self, lba: u32, len: u32) -> anyhow::Result<Vec<u8>> {
        let offset = lba as u64 * self.sector_size;
        let mut buf = vec![0u8; len as usize];
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(&mut buf)?;
        Ok(buf)
    }
}

/// Parse the root directory record from a Volume Descriptor (PVD or SVD).
/// Returns `(extent_lba, data_len)`.
fn parse_root_record(descriptor: &[u8]) -> anyhow::Result<(u32, u32)> {
    // The root directory record starts at offset 156 of the descriptor.
    let root = &descriptor[156..190];
    let lba = u32::from_le_bytes([root[2], root[3], root[4], root[5]]);
    let len = u32::from_le_bytes([root[10], root[11], root[12], root[13]]);
    Ok((lba, len))
}

/// Read a both-endian little/big 16-bit value at `offset` (little endian half).
fn read_both_u16(buf: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([buf[offset], buf[offset + 1]])
}

/// A raw, ungrouped directory record before multi-extent merging.
struct RawRecord {
    name: String,
    is_dir: bool,
    multi_extent: bool,
    lba: u32,
    len: u32,
}

/// Parse the sequence of directory records from a directory extent.
fn parse_directory_records(data: &[u8], joliet: bool) -> Vec<RawRecord> {
    let mut records = Vec::new();
    let mut offset = 0usize;
    let len = data.len();

    while offset < len {
        let record_len = data[offset] as usize;
        // A zero-length record means "no more records in this sector"; skip to
        // the next sector boundary.
        if record_len == 0 {
            let next = ((offset / DEFAULT_SECTOR_SIZE as usize) + 1) * DEFAULT_SECTOR_SIZE as usize;
            if next <= offset || next >= len {
                break;
            }
            offset = next;
            continue;
        }
        if offset + record_len > len || record_len < 34 {
            break;
        }

        let rec = &data[offset..offset + record_len];
        let ext_lba = u32::from_le_bytes([rec[2], rec[3], rec[4], rec[5]]);
        let data_len = u32::from_le_bytes([rec[10], rec[11], rec[12], rec[13]]);
        let flags = rec[25];
        let name_len = rec[32] as usize;

        let name = if joliet {
            decode_joliet_name(&rec[33..33 + name_len])
        } else {
            String::from_utf8_lossy(&rec[33..33 + name_len]).to_string()
        };

        // Strip the `;1` version suffix common to plain ISO9660 names.
        let name = strip_version_suffix(&name);
        let is_dir = flags & FILE_FLAG_DIRECTORY != 0;
        let multi_extent = flags & FILE_FLAG_MULTI_EXTENT != 0;

        records.push(RawRecord {
            name,
            is_dir,
            multi_extent,
            lba: ext_lba,
            len: data_len,
        });

        offset += record_len;
    }

    records
}

/// Decode a Joliet UCS-2BE file identifier to UTF-8.
fn decode_joliet_name(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect();
    String::from_utf16_lossy(&units)
}

/// Strip the ISO9660 `;N` version suffix (`README.TXT;1` → `README.TXT`).
fn strip_version_suffix(name: &str) -> String {
    if let Some(idx) = name.rfind(';') {
        if name[idx + 1..].chars().all(|c| c.is_ascii_digit()) {
            return name[..idx].to_string();
        }
    }
    name.to_string()
}

/// Merge consecutive multi-extent directory records into single `DirEntry`s.
///
/// A fragmented ISO9660 file is encoded as a run of directory records with the
/// same identifier: all but the last carry the multi-extent flag. The total
/// file length is the sum of the individual extent lengths.
fn group_multi_extent(raw: Vec<RawRecord>) -> Vec<DirEntry> {
    let mut out: Vec<DirEntry> = Vec::new();
    let mut i = 0usize;
    while i < raw.len() {
        let first = &raw[i];
        let mut extents = vec![Extent {
            lba: first.lba,
            len: first.len,
        }];
        let mut total_len = first.len as u64;

        // Only files (not directories) can be multi-extent, and the records
        // must share the same name and be contiguous.
        if first.multi_extent && !first.is_dir {
            let mut j = i + 1;
            while j < raw.len() {
                let next = &raw[j];
                if next.name == first.name && !next.is_dir {
                    extents.push(Extent {
                        lba: next.lba,
                        len: next.len,
                    });
                    total_len += next.len as u64;
                    if !next.multi_extent {
                        break;
                    }
                    j += 1;
                } else {
                    break;
                }
            }
            i = j;
        }

        out.push(DirEntry {
            name: first.name.clone(),
            is_dir: first.is_dir,
            lba: first.lba,
            len: total_len.min(u32::MAX as u64) as u32,
            extents,
        });
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_version_suffix_removes_numeric_suffix() {
        assert_eq!(strip_version_suffix("README.TXT;1"), "README.TXT");
        assert_eq!(strip_version_suffix("boot.wim;1"), "boot.wim");
        assert_eq!(strip_version_suffix("no-suffix"), "no-suffix");
        assert_eq!(strip_version_suffix("file;1;2"), "file;1");
    }

    #[test]
    fn joliet_name_decoding_round_trips() {
        let bytes = "hal9001".encode_utf16().flat_map(|u| u.to_be_bytes());
        let bytes: Vec<u8> = bytes.collect();
        assert_eq!(decode_joliet_name(&bytes), "hal9001");
    }

    #[test]
    fn multi_extent_grouping_sums_extents() {
        let raw = vec![
            RawRecord {
                name: "boot.wim".to_string(),
                is_dir: false,
                multi_extent: true,
                lba: 100,
                len: 1000,
            },
            RawRecord {
                name: "boot.wim".to_string(),
                is_dir: false,
                multi_extent: false,
                lba: 200,
                len: 500,
            },
        ];
        let grouped = group_multi_extent(raw);
        assert_eq!(grouped.len(), 1);
        assert_eq!(grouped[0].len, 1500);
        assert_eq!(grouped[0].extents.len(), 2);
    }

    #[test]
    fn multi_extent_grouping_keeps_unrelated_records() {
        let raw = vec![
            RawRecord {
                name: "a.bin".to_string(),
                is_dir: false,
                multi_extent: false,
                lba: 1,
                len: 10,
            },
            RawRecord {
                name: "b.bin".to_string(),
                is_dir: false,
                multi_extent: false,
                lba: 2,
                len: 20,
            },
        ];
        let grouped = group_multi_extent(raw);
        assert_eq!(grouped.len(), 2);
    }
}
