//! Shared test helpers: a minimal synthetic ISO9660 image builder used by the
//! `iso_reader`, `image_probe`, `grub_gen` and `windows_provision` test suites.

#![allow(dead_code)]

use std::io::Write;

pub const SECTOR: usize = 2048;

/// A file or directory to place in the synthetic image.
#[derive(Clone)]
pub struct Entry {
    pub path: String,
    pub is_dir: bool,
    pub content: Vec<u8>,
}

impl Entry {
    pub fn file(path: &str, content: Vec<u8>) -> Self {
        Self {
            path: path.to_string(),
            is_dir: false,
            content,
        }
    }

    pub fn dir(path: &str) -> Self {
        Self {
            path: path.to_string(),
            is_dir: true,
            content: Vec::new(),
        }
    }
}

#[derive(Clone)]
struct Node {
    name: String,
    is_dir: bool,
    content: Vec<u8>,
    children: Vec<Node>,
    lba: u32,
    len: u32,
}

impl Node {
    fn dir(name: &str) -> Self {
        Self {
            name: name.to_string(),
            is_dir: true,
            content: Vec::new(),
            children: Vec::new(),
            lba: 0,
            len: 0,
        }
    }

    fn file(name: &str, content: Vec<u8>) -> Self {
        Self {
            name: name.to_string(),
            is_dir: false,
            content,
            children: Vec::new(),
            lba: 0,
            len: 0,
        }
    }

    fn dir_extent_len(&self) -> u32 {
        let raw: usize = self.children.iter().map(|c| 33 + c.name.len()).sum();
        round_up_2048(raw as u32)
    }
}

fn round_up_2048(n: u32) -> u32 {
    n.div_ceil(SECTOR as u32) * SECTOR as u32
}

fn insert(node: &mut Node, entry: &Entry) {
    let comps: Vec<&str> = entry
        .path
        .trim_start_matches('/')
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    insert_rec(node, &comps, entry);
}

fn insert_rec(node: &mut Node, comps: &[&str], entry: &Entry) {
    if comps.len() == 1 {
        let name = comps[0];
        if entry.is_dir {
            node.children.push(Node::dir(name));
        } else {
            node.children.push(Node::file(name, entry.content.clone()));
        }
        return;
    }
    let name = comps[0];
    if let Some(child) = node
        .children
        .iter_mut()
        .find(|c| c.name == name && c.is_dir)
    {
        insert_rec(child, &comps[1..], entry);
    } else {
        let mut dir = Node::dir(name);
        insert_rec(&mut dir, &comps[1..], entry);
        node.children.push(dir);
    }
}

fn assign_lbas(node: &mut Node, next_lba: &mut u32) {
    node.lba = *next_lba;
    node.len = if node.is_dir {
        node.dir_extent_len()
    } else {
        node.content.len() as u32
    };
    *next_lba += round_up_2048(node.len) / SECTOR as u32;
    for child in node.children.iter_mut() {
        assign_lbas(child, next_lba);
    }
}

/// Build a minimal ISO9660 image containing `entries` (paths are absolute,
/// e.g. `/sources/boot.wim`). The PVD is at sector 16 and the root directory
/// follows; no Joliet/Rock Ridge.
pub fn build_iso(entries: &[Entry]) -> Vec<u8> {
    let mut root = Node::dir("");
    for e in entries {
        insert(&mut root, e);
    }

    let mut next_lba = 24u32;
    assign_lbas(&mut root, &mut next_lba);

    let total_sectors = next_lba as usize;
    let mut image = vec![0u8; total_sectors * SECTOR];

    write_pvd(&mut image, root.lba, root.len);
    write_terminator(&mut image);
    write_node(&mut image, &root, 0);

    image
}

fn write_pvd(image: &mut [u8], root_lba: u32, root_len: u32) {
    let off = 16 * SECTOR;
    let pvd = &mut image[off..off + SECTOR];
    pvd[0] = 1;
    pvd[1..6].copy_from_slice(b"CD001");
    pvd[6] = 1;
    // Logical block size = 2048 (both-endian).
    pvd[128..130].copy_from_slice(&(SECTOR as u16).to_le_bytes());
    pvd[130..132].copy_from_slice(&(SECTOR as u16).to_be_bytes());
    // Root directory record at offset 156.
    let mut rec = [0u8; 34];
    rec[0] = 34;
    rec[2..6].copy_from_slice(&root_lba.to_le_bytes());
    rec[6..10].copy_from_slice(&root_lba.to_be_bytes());
    rec[10..14].copy_from_slice(&root_len.to_le_bytes());
    rec[14..18].copy_from_slice(&root_len.to_be_bytes());
    rec[25] = 0x02;
    rec[32] = 1;
    rec[33] = 0x00;
    pvd[156..190].copy_from_slice(&rec);
}

fn write_terminator(image: &mut [u8]) {
    let off = 17 * SECTOR;
    image[off] = 255;
}

fn write_node(image: &mut [u8], node: &Node, _depth: u32) {
    if node.is_dir {
        let mut extent = vec![0u8; node.len as usize];
        let mut cursor = 0usize;
        for child in &node.children {
            let rec = encode_record(&child.name, child.lba, child.len, child.is_dir);
            extent[cursor..cursor + rec.len()].copy_from_slice(&rec);
            cursor += rec.len();
        }
        let off = node.lba as usize * SECTOR;
        image[off..off + extent.len()].copy_from_slice(&extent);

        for child in &node.children {
            write_node(image, child, _depth + 1);
        }
    } else if !node.content.is_empty() {
        let off = node.lba as usize * SECTOR;
        image[off..off + node.content.len()].copy_from_slice(&node.content);
    }
}

fn encode_record(name: &str, lba: u32, len: u32, is_dir: bool) -> Vec<u8> {
    let mut rec = vec![0u8; 33 + name.len()];
    rec[0] = rec.len() as u8;
    rec[2..6].copy_from_slice(&lba.to_le_bytes());
    rec[6..10].copy_from_slice(&lba.to_be_bytes());
    rec[10..14].copy_from_slice(&len.to_le_bytes());
    rec[14..18].copy_from_slice(&len.to_be_bytes());
    rec[25] = if is_dir { 0x02 } else { 0x00 };
    rec[32] = name.len() as u8;
    rec[33..33 + name.len()].copy_from_slice(name.as_bytes());
    rec
}

/// A read/write/seek view of a sub-region of a file, used to format/write a
/// FAT32 partition at a byte offset inside a whole-disk test image.
pub struct OffsetFile {
    inner: std::fs::File,
    offset: u64,
}

impl OffsetFile {
    pub fn new(mut inner: std::fs::File, offset: u64) -> Self {
        use std::io::Seek;
        let _ = inner.seek(std::io::SeekFrom::Start(offset));
        Self { inner, offset }
    }
}

impl std::io::Read for OffsetFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl std::io::Write for OffsetFile {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl std::io::Seek for OffsetFile {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        match pos {
            std::io::SeekFrom::Start(n) => {
                self.inner.seek(std::io::SeekFrom::Start(n + self.offset))?;
                Ok(n)
            }
            std::io::SeekFrom::Current(n) => {
                self.inner.seek(std::io::SeekFrom::Current(n))?;
                let raw = self.inner.stream_position()?;
                Ok(raw - self.offset)
            }
            std::io::SeekFrom::End(n) => {
                let raw = self.inner.seek(std::io::SeekFrom::End(n))?;
                Ok(raw.saturating_sub(self.offset))
            }
        }
    }
}

/// Build a GPT disk image with a single FAT32 ESP partition holding `files`
/// (absolute paths → bytes). Returns `(tempdir, esp_offset)`; the disk lives at
/// `dir.path()/disk.img` for the lifetime of the returned `TempDir`.
pub fn build_gpt_fat_disk(files: &[(&str, Vec<u8>)]) -> (tempfile::TempDir, u64) {
    let dir = tempfile::tempdir().expect("tempdir");
    let disk_path = dir.path().join("disk.img");
    let disk_size: u64 = 128 * 1024 * 1024;
    let esp_size: u64 = 64 * 1024 * 1024;

    std::fs::File::create(&disk_path)
        .and_then(|f| {
            f.set_len(disk_size)?;
            Ok(())
        })
        .expect("create disk");

    let layout = hal9001::backend::storage::create_gpt_dual(
        disk_path.to_str().unwrap(),
        disk_size,
        esp_size,
    )
    .expect("create_gpt_dual");
    hal9001::backend::storage::format_fat32_partition(
        disk_path.to_str().unwrap(),
        "HAL9001ESP",
        layout.esp_offset,
    )
    .expect("format esp");

    {
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&disk_path)
            .unwrap();
        let mut wrapper = OffsetFile::new(file, layout.esp_offset);
        let fs = fatfs::FileSystem::new(&mut wrapper, fatfs::FsOptions::new()).expect("open fat");
        for (path, content) in files {
            let mut current = fs.root_dir();
            let comps: Vec<&str> = path
                .trim_start_matches('/')
                .split('/')
                .filter(|s| !s.is_empty())
                .collect();
            let (dirs, name) = comps.split_at(comps.len() - 1);
            for d in dirs {
                current = current.create_dir(d).expect("create dir");
            }
            current
                .create_file(name[0])
                .unwrap()
                .write_all(content)
                .unwrap();
        }
    }

    (dir, layout.esp_offset)
}
