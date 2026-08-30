//! Narrow, non-executing reader for persisted `SyncTeX` artifacts.

use crate::CompilerError;
use flate2::read::GzDecoder;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io::Read};

const SP_PER_POINT: f64 = 65_781.76;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SyncTexLocation {
    pub page: u32,
    pub source_path: String,
    pub line: u32,
    pub column: u32,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub exact: bool,
}

#[derive(Clone, Debug)]
pub struct SyncTexIndex {
    locations: Vec<SyncTexLocation>,
}

impl SyncTexIndex {
    pub fn from_gzip(bytes: &[u8]) -> Result<Self, CompilerError> {
        let mut gzip = GzDecoder::new(bytes);
        let mut text = String::new();
        gzip.read_to_string(&mut text)
            .map_err(|source| CompilerError::Io {
                operation: "decode SyncTeX artifact",
                source,
            })?;
        Self::from_text(&text)
    }

    pub fn from_text(text: &str) -> Result<Self, CompilerError> {
        let mut inputs = HashMap::<i64, String>::new();
        let mut page = None;
        let mut unit = 1_f64;
        let mut magnification = 1_000_f64;
        let mut x_offset = 0_f64;
        let mut y_offset = 0_f64;
        let mut raw = Vec::new();
        for line in text.lines() {
            if let Some(value) = line.strip_prefix("Input:") {
                if let Some((tag, path)) = value.split_once(':') {
                    if let Ok(tag) = tag.parse::<i64>() {
                        inputs.insert(tag, path.to_owned());
                    }
                }
            } else if let Some(value) = line.strip_prefix("Unit:") {
                unit = value.parse().unwrap_or(1_f64);
            } else if let Some(value) = line.strip_prefix("Magnification:") {
                magnification = value.parse().unwrap_or(1_000_f64);
            } else if let Some(value) = line.strip_prefix("X Offset:") {
                x_offset = value.parse().unwrap_or(0_f64);
            } else if let Some(value) = line.strip_prefix("Y Offset:") {
                y_offset = value.parse().unwrap_or(0_f64);
            } else if let Some(value) = line.strip_prefix('{') {
                page = value.parse::<u32>().ok();
            } else if line == "}" {
                page = None;
            } else if let Some(page) = page {
                if let Some(record) = parse_record(line, page) {
                    raw.push(record);
                }
            }
        }
        let factor = unit * 1_000_f64 / magnification / SP_PER_POINT;
        let locations = raw
            .into_iter()
            .filter_map(|record| {
                let source_path = inputs.get(&record.tag)?.clone();
                Some(SyncTexLocation {
                    page: record.page,
                    source_path,
                    line: record.line,
                    column: 0,
                    x: (record.x + x_offset) * factor,
                    y: (record.y + y_offset) * factor,
                    width: record.width.abs() * factor,
                    height: (record.height.abs() + record.depth.abs()) * factor,
                    exact: false,
                })
            })
            .collect();
        Ok(Self { locations })
    }

    #[must_use]
    pub fn forward(&self, source_path: &str, line: u32, column: u32) -> Option<SyncTexLocation> {
        let mut result = self
            .locations
            .iter()
            .filter(|location| path_matches(&location.source_path, source_path))
            .min_by_key(|location| location.line.abs_diff(line))?
            .clone();
        result.column = column;
        result.exact = result.line == line && column == 0;
        Some(result)
    }

    #[must_use]
    pub fn inverse(&self, page: u32, x: f64, y: f64) -> Option<SyncTexLocation> {
        let mut result = self
            .locations
            .iter()
            .filter(|location| location.page == page)
            .min_by(|left, right| {
                distance(left, x, y)
                    .total_cmp(&distance(right, x, y))
                    .then_with(|| left.line.cmp(&right.line))
            })?
            .clone();
        // A containing box identifies a useful line, but the format does not
        // provide a reliable source column for exact inverse attribution.
        result.exact = false;
        Some(result)
    }
}

#[derive(Copy, Clone, Debug)]
struct RawRecord {
    page: u32,
    tag: i64,
    line: u32,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    depth: f64,
}

fn parse_record(line: &str, page: u32) -> Option<RawRecord> {
    let body = line.strip_prefix(['[', '(', 'v', 'h'])?;
    let (source, geometry) = body.split_once(':')?;
    let mut source = source.split(',');
    let tag = source.next()?.parse().ok()?;
    let line = source.next()?.parse().ok()?;
    let mut parts = geometry.split(':');
    let mut point = parts.next()?.split(',');
    let x = point.next()?.parse().ok()?;
    let y = point.next()?.parse().ok()?;
    let mut size = parts.next().unwrap_or_default().split(',');
    Some(RawRecord {
        page,
        tag,
        line,
        x,
        y,
        width: size
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0_f64),
        height: size
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0_f64),
        depth: size
            .next()
            .and_then(|value| value.parse().ok())
            .unwrap_or(0_f64),
    })
}

fn path_matches(artifact_path: &str, requested: &str) -> bool {
    artifact_path == requested
        || artifact_path
            .strip_suffix(requested)
            .is_some_and(|prefix| prefix.ends_with('/'))
}

fn distance(location: &SyncTexLocation, x: f64, y: f64) -> f64 {
    let center_x = location.x + location.width / 2_f64;
    let center_y = location.y - location.height / 2_f64;
    (center_x - x).mul_add(center_x - x, (center_y - y) * (center_y - y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_forward_inverse_and_does_not_invent_invalid_locations() {
        let index = SyncTexIndex::from_text(
            "SyncTeX Version:1\nInput:1:/work/main.tex\nUnit:1\nMagnification:1000\nX Offset:0\nY Offset:0\nContent:\n{1\n[1,7:6578176,13156352:6578176,657817,0\n}\n",
        )
        .expect("parse representative SyncTeX text");
        let forward = index.forward("main.tex", 7, 0).expect("forward location");
        assert_eq!(forward.page, 1);
        assert!(forward.exact);
        assert_eq!(forward.column, 0);
        let inverse = index
            .inverse(1, forward.x + 1_f64, forward.y)
            .expect("inverse location");
        assert_eq!(inverse.line, 7);
        assert!(!inverse.exact);
        assert!(index.forward("missing.tex", 1, 0).is_none());
        assert!(index.inverse(99, 0_f64, 0_f64).is_none());
    }
}
