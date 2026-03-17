//! Minimal WKB parser for bounding box extraction.
//!
//! Supports ISO WKB (type + 1000/2000/3000 for Z/M/ZM) and
//! EWKB (PostGIS flag bits 0x80000000 / 0x40000000 / 0x20000000).

use crate::error::{ErffError, Result};
use crate::types::Envelope;
use std::io::{Cursor, Read};

fn read_u8(r: &mut Cursor<&[u8]>) -> Result<u8> {
    let mut buf = [0u8; 1];
    r.read_exact(&mut buf)?;
    Ok(buf[0])
}

fn read_u32(r: &mut Cursor<&[u8]>, le: bool) -> Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(if le { u32::from_le_bytes(buf) } else { u32::from_be_bytes(buf) })
}

fn read_f64(r: &mut Cursor<&[u8]>, le: bool) -> Result<f64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(if le { f64::from_le_bytes(buf) } else { f64::from_be_bytes(buf) })
}

/// Decode WKB geometry type into (base_type, coord_dimensions).
/// coord_dimensions: 2=XY, 3=XYZ or XYM, 4=XYZM.
fn decode_geom_type(raw: u32) -> (u32, usize) {
    // EWKB flags
    let has_z_ewkb = raw & 0x80000000 != 0;
    let has_m_ewkb = raw & 0x40000000 != 0;
    let has_srid_ewkb = raw & 0x20000000 != 0;

    if has_z_ewkb || has_m_ewkb || has_srid_ewkb {
        let base = raw & 0x000FFFFF;
        let dims = 2 + has_z_ewkb as usize + has_m_ewkb as usize;
        return (base, dims);
    }

    // ISO WKB
    let base = raw % 1000;
    let modifier = raw / 1000;
    let dims = match modifier {
        0 => 2,
        1 => 3, // Z
        2 => 3, // M
        3 => 4, // ZM
        _ => 2,
    };
    (base, dims)
}

/// Skip EWKB SRID if present.
fn skip_ewkb_srid(r: &mut Cursor<&[u8]>, raw_type: u32, le: bool) -> Result<()> {
    if raw_type & 0x20000000 != 0 {
        let _srid = read_u32(r, le)?;
    }
    Ok(())
}

/// Read a single coordinate and expand the envelope (only X and Y).
fn read_coord(r: &mut Cursor<&[u8]>, le: bool, dims: usize, env: &mut Envelope) -> Result<()> {
    let x = read_f64(r, le)?;
    let y = read_f64(r, le)?;
    env.expand_xy(x, y);
    // Skip extra dimensions (Z, M)
    for _ in 2..dims {
        let _ = read_f64(r, le)?;
    }
    Ok(())
}

/// Recursively extract the bounding box from WKB data starting at the current cursor position.
fn extract_bbox(r: &mut Cursor<&[u8]>) -> Result<Envelope> {
    let byte_order = read_u8(r)?;
    let le = match byte_order {
        0 => false,
        1 => true,
        _ => return Err(ErffError::InvalidWkb("bad byte order".into())),
    };

    let raw_type = read_u32(r, le)?;
    skip_ewkb_srid(r, raw_type, le)?;
    let (base_type, dims) = decode_geom_type(raw_type);

    let mut env = Envelope::EMPTY;

    match base_type {
        1 => {
            // Point
            read_coord(r, le, dims, &mut env)?;
        }
        2 => {
            // LineString
            let n = read_u32(r, le)? as usize;
            for _ in 0..n {
                read_coord(r, le, dims, &mut env)?;
            }
        }
        3 => {
            // Polygon
            let num_rings = read_u32(r, le)? as usize;
            for _ in 0..num_rings {
                let n = read_u32(r, le)? as usize;
                for _ in 0..n {
                    read_coord(r, le, dims, &mut env)?;
                }
            }
        }
        4..=7 => {
            // Multi* and GeometryCollection: recursive
            let num_geoms = read_u32(r, le)? as usize;
            for _ in 0..num_geoms {
                let sub = extract_bbox(r)?;
                env.expand(&sub);
            }
        }
        _ => return Err(ErffError::InvalidWkb(format!("unknown geometry type {base_type}"))),
    }

    Ok(env)
}

/// Compute the 2D bounding box of an ISO WKB or EWKB geometry.
pub fn wkb_envelope(wkb: &[u8]) -> Result<Envelope> {
    if wkb.is_empty() {
        return Ok(Envelope::EMPTY);
    }
    let mut cursor = Cursor::new(wkb);
    let env = extract_bbox(&mut cursor)?;
    Ok(env)
}

/// Encode a 2D point as ISO WKB (little-endian).
pub fn encode_point_wkb(x: f64, y: f64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(21);
    buf.push(1); // little-endian
    buf.extend_from_slice(&1u32.to_le_bytes()); // Point
    buf.extend_from_slice(&x.to_le_bytes());
    buf.extend_from_slice(&y.to_le_bytes());
    buf
}

/// Encode a 2D linestring as ISO WKB (little-endian).
pub fn encode_linestring_wkb(coords: &[(f64, f64)]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(9 + coords.len() * 16);
    buf.push(1);
    buf.extend_from_slice(&2u32.to_le_bytes()); // LineString
    buf.extend_from_slice(&(coords.len() as u32).to_le_bytes());
    for &(x, y) in coords {
        buf.extend_from_slice(&x.to_le_bytes());
        buf.extend_from_slice(&y.to_le_bytes());
    }
    buf
}

/// Encode a 2D polygon as ISO WKB (little-endian). Rings are (exterior, holes...).
pub fn encode_polygon_wkb(rings: &[&[(f64, f64)]]) -> Vec<u8> {
    let total_coords: usize = rings.iter().map(|r| r.len()).sum();
    let mut buf = Vec::with_capacity(9 + rings.len() * 4 + total_coords * 16);
    buf.push(1);
    buf.extend_from_slice(&3u32.to_le_bytes()); // Polygon
    buf.extend_from_slice(&(rings.len() as u32).to_le_bytes());
    for ring in rings {
        buf.extend_from_slice(&(ring.len() as u32).to_le_bytes());
        for &(x, y) in *ring {
            buf.extend_from_slice(&x.to_le_bytes());
            buf.extend_from_slice(&y.to_le_bytes());
        }
    }
    buf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_point_bbox() {
        let wkb = encode_point_wkb(1.0, 2.0);
        let env = wkb_envelope(&wkb).unwrap();
        assert_eq!(env, Envelope::new(1.0, 2.0, 1.0, 2.0));
    }

    #[test]
    fn test_linestring_bbox() {
        let wkb = encode_linestring_wkb(&[(0.0, 0.0), (10.0, 5.0), (3.0, 8.0)]);
        let env = wkb_envelope(&wkb).unwrap();
        assert_eq!(env, Envelope::new(0.0, 0.0, 10.0, 8.0));
    }

    #[test]
    fn test_polygon_bbox() {
        let ring = [(0.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 0.0)];
        let wkb = encode_polygon_wkb(&[&ring]);
        let env = wkb_envelope(&wkb).unwrap();
        assert_eq!(env, Envelope::new(0.0, 0.0, 10.0, 10.0));
    }

    #[test]
    fn test_empty() {
        let env = wkb_envelope(&[]).unwrap();
        assert!(env.is_empty());
    }
}
