//! Shared, bounded ZIP ingestion for user imports and server templates.
#![forbid(unsafe_code)]

use bytes::Bytes;
use core_types::LogicalPath;
use std::{
    collections::BTreeSet,
    io::{Cursor, Read},
};
use thiserror::Error;
use zip::ZipArchive;

pub const MAX_ARCHIVE_BYTES: usize = 32 * 1024 * 1024;
pub const MAX_EXPANDED_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_FILES: usize = 2_000;
const MAX_DEPTH: usize = 32;

#[derive(Copy, Clone, Debug, Error)]
pub enum ArchiveError {
    #[error("archive is too large")]
    ArchiveTooLarge,
    #[error("archive is invalid or unsupported")]
    Invalid,
    #[error("archive contains an unsafe path")]
    UnsafePath,
    #[error("archive contains an unsupported entry")]
    UnsupportedEntry,
    #[error("archive contains a duplicate path")]
    DuplicatePath,
    #[error("archive contains too many files")]
    TooManyFiles,
    #[error("archive contains an oversized file")]
    FileTooLarge,
    #[error("archive exceeds the expanded project size limit")]
    ExpandedTooLarge,
    #[error("archive is empty")]
    Empty,
}

#[derive(Clone, Debug)]
pub struct ImportedFile {
    pub path: LogicalPath,
    pub bytes: Bytes,
}

#[derive(Clone, Debug)]
pub struct ImportedArchive {
    pub files: Vec<ImportedFile>,
    pub detected_main: Option<LogicalPath>,
}

/// Reads only regular, safe ZIP entries. Bytes are bounded as they are
/// decompressed rather than trusting central-directory size metadata.
pub fn read_archive(bytes: &[u8]) -> Result<ImportedArchive, ArchiveError> {
    if bytes.len() > MAX_ARCHIVE_BYTES {
        return Err(ArchiveError::ArchiveTooLarge);
    }
    let mut archive = ZipArchive::new(Cursor::new(bytes)).map_err(|_| ArchiveError::Invalid)?;
    reject_duplicate_central_paths(bytes, &archive)?;
    if archive.len() > MAX_FILES.saturating_mul(2) {
        return Err(ArchiveError::TooManyFiles);
    }
    let mut files = Vec::new();
    let mut paths = BTreeSet::new();
    let mut expanded = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|_| ArchiveError::Invalid)?;
        let raw = entry.name_raw();
        if raw.contains(&0) {
            return Err(ArchiveError::UnsafePath);
        }
        let name = entry.name();
        if name.starts_with('/') || name.starts_with('\\') || name.as_bytes().get(1) == Some(&b':')
        {
            return Err(ArchiveError::UnsafePath);
        }
        if entry.encrypted() {
            return Err(ArchiveError::UnsupportedEntry);
        }
        if entry.is_symlink() {
            return Err(ArchiveError::UnsupportedEntry);
        }
        if entry.unix_mode().is_some_and(|mode| {
            let kind = mode & 0o170_000;
            kind != 0 && kind != 0o100_000 && kind != 0o040_000
        }) {
            return Err(ArchiveError::UnsupportedEntry);
        }
        if entry.is_dir() {
            if !name.is_empty() {
                validate_directory(name)?;
            }
            continue;
        }
        if !entry.is_file() {
            return Err(ArchiveError::UnsupportedEntry);
        }
        let path = LogicalPath::parse(name).map_err(|_| ArchiveError::UnsafePath)?;
        if path.as_str().split('/').count() > MAX_DEPTH {
            return Err(ArchiveError::UnsafePath);
        }
        if !paths.insert(path.clone()) {
            return Err(ArchiveError::DuplicatePath);
        }
        if files.len() == MAX_FILES {
            return Err(ArchiveError::TooManyFiles);
        }
        expanded = reserve_declared_size(expanded, entry.size())?;
        let limit = usize::try_from(MAX_FILE_BYTES + 1).map_err(|_| ArchiveError::FileTooLarge)?;
        let mut content = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0).min(limit));
        entry
            .by_ref()
            .take(MAX_FILE_BYTES + 1)
            .read_to_end(&mut content)
            .map_err(|_| ArchiveError::Invalid)?;
        if content.len()
            > usize::try_from(MAX_FILE_BYTES).map_err(|_| ArchiveError::FileTooLarge)?
        {
            return Err(ArchiveError::FileTooLarge);
        }
        let actual = u64::try_from(content.len()).map_err(|_| ArchiveError::FileTooLarge)?;
        if actual > entry.size() {
            return Err(ArchiveError::ExpandedTooLarge);
        }
        files.push(ImportedFile {
            path,
            bytes: Bytes::from(content),
        });
    }
    if files.is_empty() {
        return Err(ArchiveError::Empty);
    }
    let detected_main = detect_main(&files);
    Ok(ImportedArchive {
        files,
        detected_main,
    })
}

fn reserve_declared_size(expanded: u64, size: u64) -> Result<u64, ArchiveError> {
    if size > MAX_FILE_BYTES {
        return Err(ArchiveError::FileTooLarge);
    }
    let total = expanded
        .checked_add(size)
        .ok_or(ArchiveError::ExpandedTooLarge)?;
    if total > MAX_EXPANDED_BYTES {
        Err(ArchiveError::ExpandedTooLarge)
    } else {
        Ok(total)
    }
}

/// `ZipArchive` indexes names for lookup, so duplicate central-directory names
/// cannot be inferred from its public file iterator. Inspect the central
/// directory structure as well, while still letting the ZIP library handle all
/// decompression and entry decoding.
fn reject_duplicate_central_paths(
    bytes: &[u8],
    archive: &ZipArchive<Cursor<&[u8]>>,
) -> Result<(), ArchiveError> {
    let start = archive
        .offset()
        .checked_add(archive.central_directory_start())
        .ok_or(ArchiveError::Invalid)?;
    let mut offset = usize::try_from(start).map_err(|_| ArchiveError::Invalid)?;
    let mut paths = BTreeSet::new();
    while bytes.get(offset..offset + 4) == Some(b"PK\x01\x02") {
        let header = bytes
            .get(offset..offset + 46)
            .ok_or(ArchiveError::Invalid)?;
        let name_len = usize::from(u16::from_le_bytes([header[28], header[29]]));
        let extra_len = usize::from(u16::from_le_bytes([header[30], header[31]]));
        let comment_len = usize::from(u16::from_le_bytes([header[32], header[33]]));
        let name_start = offset.checked_add(46).ok_or(ArchiveError::Invalid)?;
        let name_end = name_start
            .checked_add(name_len)
            .ok_or(ArchiveError::Invalid)?;
        let name = std::str::from_utf8(
            bytes
                .get(name_start..name_end)
                .ok_or(ArchiveError::Invalid)?,
        )
        .map_err(|_| ArchiveError::UnsafePath)?;
        if !name.ends_with('/') {
            let path = LogicalPath::parse(name).map_err(|_| ArchiveError::UnsafePath)?;
            if !paths.insert(path) {
                return Err(ArchiveError::DuplicatePath);
            }
        }
        offset = name_end
            .checked_add(extra_len)
            .and_then(|value| value.checked_add(comment_len))
            .ok_or(ArchiveError::Invalid)?;
    }
    Ok(())
}

fn validate_directory(name: &str) -> Result<(), ArchiveError> {
    let value = name.strip_suffix('/').ok_or(ArchiveError::UnsafePath)?;
    let path = LogicalPath::parse(value).map_err(|_| ArchiveError::UnsafePath)?;
    if path.as_str().split('/').count() > MAX_DEPTH {
        Err(ArchiveError::UnsafePath)
    } else {
        Ok(())
    }
}

fn detect_main(files: &[ImportedFile]) -> Option<LogicalPath> {
    if let Some(file) = files.iter().find(|file| file.path.as_str() == "main.tex") {
        return Some(file.path.clone());
    }
    let roots = files
        .iter()
        .filter(|file| file.path.extension() == Some("tex") && !file.path.as_str().contains('/'))
        .collect::<Vec<_>>();
    (roots.len() == 1).then(|| roots[0].path.clone())
}

#[cfg(test)]
#[allow(clippy::expect_used, reason = "archive fixtures are deterministic")]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};
    use zip::{ZipWriter, write::SimpleFileOptions};

    fn zip(entries: &[(&str, &[u8])]) -> Bytes {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut output);
            for (path, bytes) in entries {
                writer
                    .start_file(*path, SimpleFileOptions::default())
                    .expect("fixture path");
                writer.write_all(bytes).expect("fixture content");
            }
            writer.finish().expect("finish fixture");
        }
        Bytes::from(output.into_inner())
    }

    #[test]
    fn accepts_nested_latex_and_binary_files() {
        let fixture = zip(&[
            ("main.tex", b"\\documentclass{article}"),
            ("sections/intro.tex", b"Text"),
            ("references.bib", b"@article{x}"),
            ("local.sty", b"\\ProvidesPackage{local}"),
            ("figure.png", &[0, 1, 2, 255]),
        ]);
        let archive = read_archive(&fixture).expect("safe archive");
        assert_eq!(archive.files.len(), 5);
        assert_eq!(archive.detected_main.expect("main").as_str(), "main.tex");
        assert_eq!(archive.files[4].bytes.as_ref(), &[0, 1, 2, 255]);
    }

    #[test]
    fn rejects_traversal_absolute_and_duplicate_paths() {
        for entries in [
            vec![("../escape.tex", b"x" as &[u8])],
            vec![("/absolute.tex", b"x" as &[u8])],
            vec![("C:/absolute.tex", b"x" as &[u8])],
        ] {
            let fixture = zip(&entries);
            assert!(read_archive(&fixture).is_err());
        }
        let mut duplicate = zip(&[("main.tex", b"x"), ("copy.tex", b"y")]).to_vec();
        replace_everywhere(&mut duplicate, b"copy.tex", b"main.tex");
        assert!(matches!(
            read_archive(&duplicate),
            Err(ArchiveError::DuplicatePath)
        ));
    }

    #[test]
    fn rejects_symlink_entries() {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut output);
            writer
                .start_file("link", SimpleFileOptions::default())
                .expect("fixture link");
            writer.write_all(b"target").expect("target");
            writer.finish().expect("finish fixture");
        }
        let mut bytes = output.into_inner();
        let central = bytes
            .windows(4)
            .position(|value| value == b"PK\x01\x02")
            .expect("central directory");
        bytes[central + 5] = 3; // Unix creator platform
        bytes[central + 38..central + 42].copy_from_slice(&(0o120_777_u32 << 16).to_le_bytes());
        assert!(matches!(
            read_archive(&bytes),
            Err(ArchiveError::UnsupportedEntry)
        ));
    }

    #[test]
    fn rejects_excessive_file_count() {
        let mut output = Cursor::new(Vec::new());
        {
            let mut writer = ZipWriter::new(&mut output);
            for index in 0..=MAX_FILES {
                writer
                    .start_file(format!("f{index}.tex"), SimpleFileOptions::default())
                    .expect("fixture file");
            }
            writer.finish().expect("finish fixture");
        }
        let bytes = output.into_inner();
        assert!(matches!(
            read_archive(&bytes),
            Err(ArchiveError::TooManyFiles)
        ));
    }

    #[test]
    fn enforces_individual_and_expanded_size_limits() {
        assert!(matches!(
            reserve_declared_size(0, MAX_FILE_BYTES + 1),
            Err(ArchiveError::FileTooLarge)
        ));
        assert!(matches!(
            reserve_declared_size(MAX_EXPANDED_BYTES - 1, 2),
            Err(ArchiveError::ExpandedTooLarge)
        ));
    }

    fn replace_everywhere(bytes: &mut [u8], from: &[u8], to: &[u8]) {
        assert_eq!(from.len(), to.len());
        for offset in bytes
            .windows(from.len())
            .enumerate()
            .filter_map(|(offset, window)| (window == from).then_some(offset))
            .collect::<Vec<_>>()
        {
            bytes[offset..offset + from.len()].copy_from_slice(to);
        }
    }
}
