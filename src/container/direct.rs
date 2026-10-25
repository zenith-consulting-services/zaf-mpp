//! Zero-copy stream access for an in-memory OLE2 compound document that
//! `cfb` has already opened (and so validated).
//!
//! `cfb` reads a stream by rebuilding its sector chain on every buffer
//! refill and copying sector by sector, with mini streams resolved one 64
//! byte mini sector at a time; for a typical MPP file that is most of the
//! parse time. This module re-reads the same header, DIFAT, FAT, MiniFAT
//! and directory straight from the byte slice, mirroring the way
//! `cfb::CompoundFile::open` builds them in its permissive mode
//! (including its repairs), and returns a stream's bytes as a borrowed
//! slice when its sectors are contiguous, or one copy when they are not.
//!
//! It is only ever a fast path: any lookup or chain it is not certain
//! about returns `None`, and the caller reads that stream through `cfb`
//! instead, so errors and edge cases behave exactly as they did before.

use std::borrow::Cow;
use std::cmp::Ordering;

const HEADER_LEN: usize = 512;
const DIR_ENTRY_LEN: usize = 128;
const MINI_SECTOR_LEN: usize = 64;
const MINI_STREAM_CUTOFF: u64 = 4096;
const NUM_DIFAT_ENTRIES_IN_HEADER: usize = 109;
const MAX_REGULAR_SECTOR: u32 = 0xFFFF_FFFA;
const DIFAT_SECTOR: u32 = 0xFFFF_FFFC;
const FAT_SECTOR: u32 = 0xFFFF_FFFD;
const END_OF_CHAIN: u32 = 0xFFFF_FFFE;
const FREE_SECTOR: u32 = 0xFFFF_FFFF;
const NO_STREAM: u32 = 0xFFFF_FFFF;
const ROOT_STREAM_ID: u32 = 0;
const OBJ_TYPE_STREAM: u8 = 2;

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn u64_at(bytes: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(
        bytes.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

struct DirEntry {
    name: String,
    obj_type: u8,
    left_sibling: u32,
    right_sibling: u32,
    child: u32,
    start_sector: u32,
    stream_len: u64,
}

pub(crate) struct DirectReader<'a> {
    bytes: &'a [u8],
    sector_len: usize,
    num_sectors: u32,
    fat: Vec<u32>,
    minifat: Vec<u32>,
    /// Regular sector chain holding the mini stream (the root entry's).
    mini_stream_sectors: Vec<u32>,
    dir_entries: Vec<DirEntry>,
}

impl<'a> DirectReader<'a> {
    /// Build the tables for `bytes`, which `cfb` has already opened
    /// successfully. `None` if anything does not line up.
    pub(crate) fn new(bytes: &'a [u8]) -> Option<DirectReader<'a>> {
        if bytes.len() < HEADER_LEN {
            return None;
        }
        let (sector_len, stream_len_mask) = match u16_at(bytes, 0x1A)? {
            3 => (512usize, 0xFFFF_FFFFu64),
            4 => (4096usize, u64::MAX),
            _ => return None,
        };
        if bytes.len() < sector_len {
            return None;
        }
        let num_sectors = u32::try_from(bytes.len().div_ceil(sector_len) - 1).ok()?;
        let num_fat_sectors = u32_at(bytes, 0x2C)?;
        let first_dir_sector = u32_at(bytes, 0x30)?;
        let first_minifat_sector = u32_at(bytes, 0x3C)?;
        let mut first_difat_sector = u32_at(bytes, 0x44)?;
        if first_difat_sector == FREE_SECTOR {
            first_difat_sector = END_OF_CHAIN;
        }

        let mut reader = DirectReader {
            bytes,
            sector_len,
            num_sectors,
            fat: Vec::new(),
            minifat: Vec::new(),
            mini_stream_sectors: Vec::new(),
            dir_entries: Vec::new(),
        };

        // DIFAT: header entries up to the first FREE_SECTOR, then the
        // DIFAT sector chain.
        let mut difat = vec![FREE_SECTOR; NUM_DIFAT_ENTRIES_IN_HEADER];
        for (i, entry) in difat.iter_mut().enumerate() {
            let next = u32_at(bytes, 0x4C + i * 4)?;
            if next == FREE_SECTOR {
                break;
            } else if next > MAX_REGULAR_SECTOR {
                return None;
            }
            *entry = next;
        }
        let mut difat_sector_ids = Vec::new();
        let mut current = first_difat_sector;
        while current != END_OF_CHAIN && current != FREE_SECTOR {
            if current >= num_sectors
                || difat_sector_ids.len() > num_sectors as usize
                || difat_sector_ids.contains(&current)
            {
                return None;
            }
            difat_sector_ids.push(current);
            let sector = reader.sector(current)?;
            let entries = sector_len / 4;
            for i in 0..entries - 1 {
                let next = u32_at(sector, i * 4)?;
                if next != FREE_SECTOR && next > MAX_REGULAR_SECTOR {
                    return None;
                }
                difat.push(next);
            }
            current = u32_at(sector, (entries - 1) * 4)?;
        }
        while difat.len() > NUM_DIFAT_ENTRIES_IN_HEADER
            && difat.len() > num_fat_sectors as usize
            && difat.last() == Some(&0)
        {
            difat.pop();
        }
        while difat.last() == Some(&FREE_SECTOR) {
            difat.pop();
        }

        // FAT, with the same permissive trimming, padding and repairs as
        // `cfb` applies.
        let mut fat = Vec::with_capacity(difat.len() * (sector_len / 4));
        for &sector_id in &difat {
            if sector_id >= num_sectors {
                return None;
            }
            let sector = reader.sector(sector_id)?;
            fat.extend(
                sector
                    .chunks_exact(4)
                    .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])),
            );
        }
        while fat.len() > num_sectors as usize {
            match fat.last() {
                Some(&0) | Some(&DIFAT_SECTOR) | Some(&FAT_SECTOR) | Some(&FREE_SECTOR) => {
                    fat.pop();
                }
                _ => break,
            }
        }
        fat.resize(fat.len().max(num_sectors as usize), FREE_SECTOR);
        if fat.len() > num_sectors as usize {
            return None;
        }
        for &id in &difat_sector_ids {
            *fat.get_mut(id as usize)? = DIFAT_SECTOR;
        }
        for &id in &difat {
            *fat.get_mut(id as usize)? = FAT_SECTOR;
        }
        reader.fat = fat;

        // Directory.
        let dir_chain = reader.chain(first_dir_sector)?;
        let mut dir_entries = Vec::with_capacity(dir_chain.len() * (sector_len / DIR_ENTRY_LEN));
        for &sector_id in &dir_chain {
            let sector = reader.sector(sector_id)?;
            for raw in sector.chunks_exact(DIR_ENTRY_LEN) {
                dir_entries.push(read_dir_entry(raw, stream_len_mask)?);
            }
        }
        reader.dir_entries = dir_entries;

        // MiniFAT, trimmed of trailing FREE_SECTOR entries and truncated
        // to the mini stream's length, as `cfb` does.
        let minifat_chain = reader.chain(first_minifat_sector)?;
        let mut minifat = Vec::with_capacity(minifat_chain.len() * (sector_len / 4));
        for &sector_id in &minifat_chain {
            let sector = reader.sector(sector_id)?;
            minifat.extend(
                sector
                    .chunks_exact(4)
                    .map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])),
            );
        }
        while minifat.last() == Some(&FREE_SECTOR) {
            minifat.pop();
        }
        let root = reader.dir_entries.first()?;
        let root_mini_sectors = root.stream_len / MINI_SECTOR_LEN as u64;
        if root_mini_sectors < minifat.len() as u64 {
            minifat.truncate(root_mini_sectors as usize);
        }
        let root_start = root.start_sector;
        reader.minifat = minifat;
        reader.mini_stream_sectors = reader.chain(root_start)?;

        Some(reader)
    }

    /// The full sector `sector_id`, if it lies entirely within the file.
    fn sector(&self, sector_id: u32) -> Option<&'a [u8]> {
        if sector_id >= self.num_sectors {
            return None;
        }
        let start = (sector_id as usize + 1).checked_mul(self.sector_len)?;
        self.bytes.get(start..start.checked_add(self.sector_len)?)
    }

    /// Every sector in the FAT chain starting at `start`, following the
    /// same rules as `cfb`'s `Allocator::next`.
    fn chain(&self, start: u32) -> Option<Vec<u32>> {
        follow_chain(&self.fat, start)
    }

    /// Directory entry for a `/`-separated path, found the way
    /// `cfb`'s `stream_id_for_name_chain` does: a binary-tree descent of
    /// each storage's children by CFB name order.
    fn find(&self, path: &str) -> Option<&DirEntry> {
        let mut id = ROOT_STREAM_ID;
        for name in path.split('/').filter(|n| !n.is_empty()) {
            if name == "." || name == ".." {
                return None;
            }
            id = self.dir_entries.get(id as usize)?.child;
            let mut steps = 0usize;
            loop {
                if id == NO_STREAM || steps > self.dir_entries.len() {
                    return None;
                }
                steps += 1;
                let entry = self.dir_entries.get(id as usize)?;
                match compare_names(name, &entry.name)? {
                    Ordering::Equal => break,
                    Ordering::Less => id = entry.left_sibling,
                    Ordering::Greater => id = entry.right_sibling,
                }
            }
        }
        self.dir_entries.get(id as usize)
    }

    /// The bytes of the stream at `path`, or `None` to have the caller
    /// read it through `cfb` (missing, not a stream, or any chain this
    /// reader cannot vouch for).
    pub(crate) fn stream(&self, path: &str) -> Option<Cow<'a, [u8]>> {
        let entry = self.find(path)?;
        if entry.obj_type != OBJ_TYPE_STREAM {
            return None;
        }
        let len = usize::try_from(entry.stream_len).ok()?;
        if len == 0 {
            return Some(Cow::Borrowed(&[]));
        }
        // File offset of each piece of the stream, in order.
        let (piece_len, offsets): (usize, Vec<usize>) = if entry.stream_len < MINI_STREAM_CUTOFF {
            let chain = follow_chain(&self.minifat, entry.start_sector)?;
            let needed = len.div_ceil(MINI_SECTOR_LEN);
            if chain.len() < needed {
                return None;
            }
            let per_sector = self.sector_len / MINI_SECTOR_LEN;
            let mut offsets = Vec::with_capacity(needed);
            for &mini_id in &chain[..needed] {
                let sector_id = *self
                    .mini_stream_sectors
                    .get(mini_id as usize / per_sector)?;
                let sector = self.sector(sector_id)?;
                let within = (mini_id as usize % per_sector) * MINI_SECTOR_LEN;
                let base = sector.as_ptr() as usize - self.bytes.as_ptr() as usize;
                offsets.push(base + within);
            }
            (MINI_SECTOR_LEN, offsets)
        } else {
            let chain = self.chain(entry.start_sector)?;
            let needed = len.div_ceil(self.sector_len);
            if chain.len() < needed {
                return None;
            }
            let mut offsets = Vec::with_capacity(needed);
            for &sector_id in &chain[..needed] {
                let sector = self.sector(sector_id)?;
                offsets.push(sector.as_ptr() as usize - self.bytes.as_ptr() as usize);
            }
            (self.sector_len, offsets)
        };

        let contiguous = offsets.windows(2).all(|w| w[1] == w[0] + piece_len);
        if contiguous {
            let start = offsets[0];
            return self.bytes.get(start..start + len).map(Cow::Borrowed);
        }
        let mut out = Vec::with_capacity(len);
        for &offset in &offsets {
            let take = piece_len.min(len - out.len());
            out.extend_from_slice(self.bytes.get(offset..offset + take)?);
        }
        Some(Cow::Owned(out))
    }
}

/// Walk a FAT or MiniFAT chain to `END_OF_CHAIN`, rejecting the links
/// `cfb` rejects (out-of-range indices, a return to the first sector),
/// plus any chain longer than the table itself.
fn follow_chain(table: &[u32], start: u32) -> Option<Vec<u32>> {
    let mut ids = Vec::new();
    let mut current = start;
    while current != END_OF_CHAIN {
        if ids.len() >= table.len() {
            return None;
        }
        ids.push(current);
        let next = *table.get(current as usize)?;
        if next != END_OF_CHAIN && (next > MAX_REGULAR_SECTOR || next as usize >= table.len()) {
            return None;
        }
        if next == start {
            return None;
        }
        current = next;
    }
    Some(ids)
}

fn read_dir_entry(raw: &[u8], stream_len_mask: u64) -> Option<DirEntry> {
    let name_len_bytes = u16_at(raw, 64)? as usize;
    if name_len_bytes > 64 || !name_len_bytes.is_multiple_of(2) {
        return None;
    }
    let name_len_chars = if name_len_bytes > 0 {
        name_len_bytes / 2 - 1
    } else {
        0
    };
    let units: Vec<u16> = raw[..name_len_chars * 2]
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    let name = String::from_utf16(&units).ok()?;
    let obj_type = raw[66];
    let (start_sector, stream_len) = if obj_type == 1 {
        // Storage: `cfb` treats both as zero.
        (0, 0)
    } else {
        (u32_at(raw, 116)?, u64_at(raw, 120)? & stream_len_mask)
    };
    Some(DirEntry {
        name,
        obj_type,
        left_sibling: u32_at(raw, 68)?,
        right_sibling: u32_at(raw, 72)?,
        child: u32_at(raw, 76)?,
        start_sector,
        stream_len,
    })
}

/// CFB name order, as `cfb::internal::path::compare_names` implements
/// it for ASCII names: shorter names first, then a case-insensitive
/// comparison. `None` for non-ASCII names, whose ordering `cfb` derives
/// from UTF-16 uppercasing rules not reproduced here; the lookup then
/// falls back to `cfb`.
fn compare_names(name1: &str, name2: &str) -> Option<Ordering> {
    if !(name1.is_ascii() && name2.is_ascii()) {
        return None;
    }
    Some(match name1.len().cmp(&name2.len()) {
        Ordering::Equal => name1
            .bytes()
            .map(|b| b.to_ascii_uppercase())
            .cmp(name2.bytes().map(|b| b.to_ascii_uppercase())),
        other => other,
    })
}
