//! Packed Hilbert R-tree spatial index.
//!
//! Features are sorted by the Hilbert curve value of their bounding box center.
//! A static R-tree is built bottom-up from the sorted bboxes.

use crate::error::Result;
use crate::types::Envelope;
use std::io::{Read, Seek, Write};

// ── Hilbert curve ──────────────────────────────────────────────────────────

const HILBERT_ORDER: u32 = 16;
const HILBERT_N: u32 = 1 << HILBERT_ORDER; // 65536

/// Map (x, y) in [0, N) to a Hilbert curve index.
fn xy_to_hilbert(mut x: i64, mut y: i64) -> u64 {
    let mut d: u64 = 0;
    let mut s = (HILBERT_N / 2) as i64;
    while s > 0 {
        let rx = if (x & s) > 0 { 1i64 } else { 0 };
        let ry = if (y & s) > 0 { 1i64 } else { 0 };
        d += (s * s * ((3 * rx) ^ ry)) as u64;
        // Rotate
        if ry == 0 {
            if rx == 1 {
                x = s - 1 - x;
                y = s - 1 - y;
            }
            std::mem::swap(&mut x, &mut y);
        }
        s >>= 1;
    }
    d
}

/// Compute the Hilbert index for a point within the given extent.
fn hilbert_index(env: &Envelope, x: f64, y: f64) -> u64 {
    let width = env.max_x - env.min_x;
    let height = env.max_y - env.min_y;
    if width <= 0.0 || height <= 0.0 {
        return 0;
    }
    let max = (HILBERT_N - 1) as f64;
    let hx = (((x - env.min_x) / width) * max).clamp(0.0, max) as i64;
    let hy = (((y - env.min_y) / height) * max).clamp(0.0, max) as i64;
    xy_to_hilbert(hx, hy)
}

// ── R-tree structure ───────────────────────────────────────────────────────

/// Compute the number of nodes at each level of the packed R-tree.
fn level_sizes(num_items: u64, node_size: u16) -> Vec<u64> {
    let mut sizes = vec![num_items];
    let mut n = num_items;
    while n > 1 {
        n = n.div_ceil(node_size as u64);
        sizes.push(n);
    }
    sizes
}

/// Compute cumulative offsets into the flat node array for each level.
fn level_offsets(sizes: &[u64]) -> Vec<u64> {
    let mut offsets = Vec::with_capacity(sizes.len());
    let mut acc = 0u64;
    for &s in sizes {
        offsets.push(acc);
        acc += s;
    }
    offsets
}

// ── Building ───────────────────────────────────────────────────────────────

/// Build a packed Hilbert R-tree from feature bounding boxes.
///
/// Returns `(feature_indices, node_bboxes, node_size)` where:
/// - `feature_indices[i]` is the original feature index for sorted position `i`
/// - `node_bboxes` contains all levels: items first, then internal nodes bottom-up
pub fn build_index(
    bboxes: &[Envelope],
    extent: &Envelope,
    node_size: u16,
) -> (Vec<u64>, Vec<Envelope>) {
    let num_items = bboxes.len() as u64;
    if num_items == 0 {
        return (vec![], vec![]);
    }

    // Compute Hilbert values and sort
    let mut items: Vec<(u64, u64, Envelope)> = bboxes
        .iter()
        .enumerate()
        .map(|(i, bbox)| {
            let h = hilbert_index(extent, bbox.center_x(), bbox.center_y());
            (h, i as u64, *bbox)
        })
        .collect();
    items.sort_by_key(|&(h, idx, _)| (h, idx));

    let feature_indices: Vec<u64> = items.iter().map(|&(_, idx, _)| idx).collect();

    // Build node bboxes
    let sizes = level_sizes(num_items, node_size);
    let offsets = level_offsets(&sizes);
    let total_nodes: u64 = sizes.iter().sum();

    let mut nodes = vec![Envelope::EMPTY; total_nodes as usize];

    // Level 0: copy item bboxes in sorted order
    for (i, &(_, _, bbox)) in items.iter().enumerate() {
        nodes[i] = bbox;
    }

    // Internal levels: aggregate children
    for level in 1..sizes.len() {
        let child_offset = offsets[level - 1] as usize;
        let child_count = sizes[level - 1] as usize;
        let parent_offset = offsets[level] as usize;
        let ns = node_size as usize;

        for i in 0..sizes[level] as usize {
            let child_start = i * ns;
            let child_end = (child_start + ns).min(child_count);
            let mut env = Envelope::EMPTY;
            for c in child_start..child_end {
                env.expand(&nodes[child_offset + c]);
            }
            nodes[parent_offset + i] = env;
        }
    }

    (feature_indices, nodes)
}

// ── Searching ──────────────────────────────────────────────────────────────

/// Search the packed R-tree for features whose bboxes intersect the query envelope.
///
/// Returns original feature indices.
pub fn search_index(
    query: &Envelope,
    feature_indices: &[u64],
    nodes: &[Envelope],
    num_items: u64,
    node_size: u16,
) -> Vec<u64> {
    if num_items == 0 {
        return vec![];
    }

    let sizes = level_sizes(num_items, node_size);
    let offsets = level_offsets(&sizes);
    let root_level = sizes.len() - 1;

    let mut results = Vec::new();

    if root_level == 0 {
        // Only items, no internal nodes — linear scan
        for i in 0..num_items as usize {
            if nodes[i].intersects(query) {
                results.push(feature_indices[i]);
            }
        }
    } else {
        search_node(
            query,
            feature_indices,
            nodes,
            &sizes,
            &offsets,
            node_size,
            root_level,
            0,
            &mut results,
        );
    }
    results
}

#[allow(clippy::too_many_arguments)]
fn search_node(
    query: &Envelope,
    feature_indices: &[u64],
    nodes: &[Envelope],
    sizes: &[u64],
    offsets: &[u64],
    node_size: u16,
    level: usize,
    node_idx: usize,
    results: &mut Vec<u64>,
) {
    let ns = node_size as usize;
    let child_level = level - 1;
    let child_offset = offsets[child_level] as usize;
    let child_count = sizes[child_level] as usize;
    let child_start = node_idx * ns;
    let child_end = (child_start + ns).min(child_count);

    for c in child_start..child_end {
        if !nodes[child_offset + c].intersects(query) {
            continue;
        }
        if child_level == 0 {
            // Reached the item level — add directly
            results.push(feature_indices[c]);
        } else {
            search_node(
                query,
                feature_indices,
                nodes,
                sizes,
                offsets,
                node_size,
                child_level,
                c,
                results,
            );
        }
    }
}

// ── Serialization ──────────────────────────────────────────────────────────

/// Write the spatial index to a writer.
pub fn write_index<W: Write>(
    w: &mut W,
    feature_indices: &[u64],
    nodes: &[Envelope],
    node_size: u16,
    num_items: u64,
) -> Result<()> {
    w.write_all(&node_size.to_le_bytes())?;
    w.write_all(&num_items.to_le_bytes())?;

    // Feature index mapping
    for &idx in feature_indices {
        w.write_all(&idx.to_le_bytes())?;
    }

    // Node bboxes (all levels, items first then internal bottom-up)
    for node in nodes {
        w.write_all(&node.min_x.to_le_bytes())?;
        w.write_all(&node.min_y.to_le_bytes())?;
        w.write_all(&node.max_x.to_le_bytes())?;
        w.write_all(&node.max_y.to_le_bytes())?;
    }

    Ok(())
}

/// Read the spatial index from a reader.
pub fn read_index<R: Read + Seek>(r: &mut R) -> Result<(Vec<u64>, Vec<Envelope>, u16, u64)> {
    let mut buf2 = [0u8; 2];
    let mut buf8 = [0u8; 8];

    r.read_exact(&mut buf2)?;
    let node_size = u16::from_le_bytes(buf2);

    r.read_exact(&mut buf8)?;
    let num_items = u64::from_le_bytes(buf8);

    // Feature index mapping
    let mut feature_indices = Vec::with_capacity(num_items as usize);
    for _ in 0..num_items {
        r.read_exact(&mut buf8)?;
        feature_indices.push(u64::from_le_bytes(buf8));
    }

    // Compute total node count
    let sizes = level_sizes(num_items, node_size);
    let total_nodes: u64 = sizes.iter().sum();

    let mut nodes = Vec::with_capacity(total_nodes as usize);
    let mut buf_f64 = [0u8; 8];
    for _ in 0..total_nodes {
        r.read_exact(&mut buf_f64)?;
        let min_x = f64::from_le_bytes(buf_f64);
        r.read_exact(&mut buf_f64)?;
        let min_y = f64::from_le_bytes(buf_f64);
        r.read_exact(&mut buf_f64)?;
        let max_x = f64::from_le_bytes(buf_f64);
        r.read_exact(&mut buf_f64)?;
        let max_y = f64::from_le_bytes(buf_f64);
        nodes.push(Envelope { min_x, min_y, max_x, max_y });
    }

    Ok((feature_indices, nodes, node_size, num_items))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_hilbert_roundtrip_order() {
        // Points along a diagonal should have monotonically increasing Hilbert values
        // when mapped in a well-behaved extent.
        let extent = Envelope::new(0.0, 0.0, 100.0, 100.0);
        let h1 = hilbert_index(&extent, 1.0, 1.0);
        let h2 = hilbert_index(&extent, 50.0, 50.0);
        // Distinct points should produce distinct Hilbert values
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_build_and_search() {
        let bboxes: Vec<Envelope> = (0..100)
            .map(|i| {
                let x = (i % 10) as f64;
                let y = (i / 10) as f64;
                Envelope::new(x, y, x + 0.5, y + 0.5)
            })
            .collect();
        let extent = Envelope::new(0.0, 0.0, 10.0, 10.0);
        let (indices, nodes) = build_index(&bboxes, &extent, 16);

        // Query a small area that should contain a few features
        let query = Envelope::new(2.0, 3.0, 4.0, 5.0);
        let results = search_index(&query, &indices, &nodes, 100, 16);

        // Features at (2,3), (3,3), (2,4), (3,4) and their +0.5 extents
        assert!(!results.is_empty());
        for &idx in &results {
            let bbox = &bboxes[idx as usize];
            assert!(bbox.intersects(&query));
        }

        // Verify that all truly intersecting features are found
        for (i, bbox) in bboxes.iter().enumerate() {
            if bbox.intersects(&query) {
                assert!(results.contains(&(i as u64)), "Missing feature {i}");
            }
        }
    }

    #[test]
    fn test_index_serialization() {
        let bboxes: Vec<Envelope> = (0..50)
            .map(|i| {
                let x = i as f64;
                Envelope::new(x, 0.0, x + 1.0, 1.0)
            })
            .collect();
        let extent = Envelope::new(0.0, 0.0, 51.0, 1.0);
        let (indices, nodes) = build_index(&bboxes, &extent, 8);

        let mut buf = Vec::new();
        write_index(&mut buf, &indices, &nodes, 8, 50).unwrap();

        let mut cursor = Cursor::new(&buf[..]);
        let (indices2, nodes2, ns2, ni2) = read_index(&mut cursor).unwrap();

        assert_eq!(indices, indices2);
        assert_eq!(nodes.len(), nodes2.len());
        assert_eq!(ns2, 8);
        assert_eq!(ni2, 50);
    }
}
