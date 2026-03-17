//! ERFF file writer.
//!
//! Writes features sequentially, then builds and appends the spatial index
//! and footer on finalization.

use crate::error::{ErffError, Result};
use crate::index;
use crate::types::*;
use crate::wkb;
use std::io::{Seek, SeekFrom, Write};

/// Writes an ERFF file.
pub struct ErffWriter<W: Write + Seek> {
    writer: W,
    schema: Schema,
    feature_count: u64,
    envelope: Envelope,
    feature_offsets: Vec<u64>,
    feature_bboxes: Vec<Envelope>,
    data_start_offset: u64,
    finalized: bool,
    node_size: u16,
}

// ── Schema serialization ───────────────────────────────────────────────────

fn serialize_schema(schema: &Schema) -> Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(256);

    // CRS (u32 length + UTF-8)
    let crs_bytes = schema.crs.as_bytes();
    buf.extend_from_slice(&(crs_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(crs_bytes);

    // Geometry columns
    buf.extend_from_slice(&(schema.geometry_columns.len() as u16).to_le_bytes());
    for col in &schema.geometry_columns {
        let name_bytes = col.name.as_bytes();
        buf.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(name_bytes);
        buf.push(col.geom_type as u8);
        buf.push(col.coord_type as u8);
    }

    // Attribute columns
    buf.extend_from_slice(&(schema.attribute_columns.len() as u16).to_le_bytes());
    for col in &schema.attribute_columns {
        let name_bytes = col.name.as_bytes();
        buf.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(name_bytes);
        buf.push(col.col_type as u8);
        buf.push(if col.nullable { 1u8 } else { 0u8 });
    }

    // Metadata
    buf.extend_from_slice(&(schema.metadata.len() as u32).to_le_bytes());
    for (key, value) in &schema.metadata {
        let k = key.as_bytes();
        buf.extend_from_slice(&(k.len() as u16).to_le_bytes());
        buf.extend_from_slice(k);
        let v = value.as_bytes();
        buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
        buf.extend_from_slice(v);
    }

    Ok(buf)
}

// ── Feature serialization ──────────────────────────────────────────────────

fn serialize_feature(feature: &Feature, schema: &Schema) -> Result<Vec<u8>> {
    let bitmap_bytes = schema.null_bitmap_bytes();
    let mut buf = Vec::with_capacity(128);

    // Placeholder for size (filled at end)
    buf.extend_from_slice(&0u32.to_le_bytes());

    // Null bitmap
    let mut bitmap = vec![0u8; bitmap_bytes];
    for (i, geom) in feature.geometries.iter().enumerate() {
        if geom.is_some() {
            bitmap[i / 8] |= 1 << (i % 8);
        }
    }
    let geom_count = schema.geometry_columns.len();
    for (i, val) in feature.attributes.iter().enumerate() {
        if !val.is_null() {
            let col_idx = geom_count + i;
            bitmap[col_idx / 8] |= 1 << (col_idx % 8);
        }
    }
    buf.extend_from_slice(&bitmap);

    // Geometry columns
    for wkb_data in feature.geometries.iter().flatten() {
        buf.extend_from_slice(&(wkb_data.len() as u32).to_le_bytes());
        buf.extend_from_slice(wkb_data);
    }

    // Attribute columns
    for val in feature.attributes.iter() {
        match val {
            Value::Null => {}
            Value::Bool(v) => buf.push(if *v { 1 } else { 0 }),
            Value::Int8(v) => buf.extend_from_slice(&v.to_le_bytes()),
            Value::UInt8(v) => buf.push(*v),
            Value::Int16(v) => buf.extend_from_slice(&v.to_le_bytes()),
            Value::UInt16(v) => buf.extend_from_slice(&v.to_le_bytes()),
            Value::Int32(v) => buf.extend_from_slice(&v.to_le_bytes()),
            Value::UInt32(v) => buf.extend_from_slice(&v.to_le_bytes()),
            Value::Int64(v) => buf.extend_from_slice(&v.to_le_bytes()),
            Value::UInt64(v) => buf.extend_from_slice(&v.to_le_bytes()),
            Value::Float32(v) => buf.extend_from_slice(&v.to_le_bytes()),
            Value::Float64(v) => buf.extend_from_slice(&v.to_le_bytes()),
            Value::String(v) => {
                let bytes = v.as_bytes();
                buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(bytes);
            }
            Value::Binary(v) => {
                buf.extend_from_slice(&(v.len() as u32).to_le_bytes());
                buf.extend_from_slice(v);
            }
            Value::Date(v) => buf.extend_from_slice(&v.to_le_bytes()),
            Value::DateTime(v) => buf.extend_from_slice(&v.to_le_bytes()),
            Value::Json(v) => {
                let bytes = v.as_bytes();
                buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
                buf.extend_from_slice(bytes);
            }
        }
    }

    // Fill in the size (excludes the 4-byte size field itself)
    let size = (buf.len() - 4) as u32;
    buf[0..4].copy_from_slice(&size.to_le_bytes());

    Ok(buf)
}

// ── Writer implementation ──────────────────────────────────────────────────

impl<W: Write + Seek> ErffWriter<W> {
    /// Create a new ERFF writer. Writes the header and schema immediately.
    pub fn new(mut writer: W, schema: Schema) -> Result<Self> {
        let schema_bytes = serialize_schema(&schema)?;

        // Write file header (64 bytes) with placeholders
        writer.write_all(&MAGIC)?;
        writer.write_all(&[VERSION_MAJOR, VERSION_MINOR])?;
        writer.write_all(&[FLAG_HAS_SPATIAL_INDEX | FLAG_HAS_OFFSET_TABLE])?; // flags
        writer.write_all(&[0u8])?; // reserved
        writer.write_all(&0u64.to_le_bytes())?; // feature count placeholder
        writer.write_all(&f64::INFINITY.to_le_bytes())?; // min_x placeholder
        writer.write_all(&f64::INFINITY.to_le_bytes())?; // min_y placeholder
        writer.write_all(&f64::NEG_INFINITY.to_le_bytes())?; // max_x placeholder
        writer.write_all(&f64::NEG_INFINITY.to_le_bytes())?; // max_y placeholder
        writer.write_all(&(schema_bytes.len() as u32).to_le_bytes())?; // schema size
        writer.write_all(&[0u8; 12])?; // reserved

        // Write schema
        writer.write_all(&schema_bytes)?;

        let data_start_offset = HEADER_SIZE + schema_bytes.len() as u64;

        Ok(Self {
            writer,
            schema,
            feature_count: 0,
            envelope: Envelope::EMPTY,
            feature_offsets: Vec::new(),
            feature_bboxes: Vec::new(),
            data_start_offset,
            finalized: false,
            node_size: DEFAULT_NODE_SIZE,
        })
    }

    /// Set the R-tree node size (default: 16). Must be called before adding features.
    pub fn set_node_size(&mut self, node_size: u16) {
        self.node_size = node_size;
    }

    /// Add a feature to the file.
    pub fn add_feature(&mut self, feature: &Feature) -> Result<()> {
        if self.finalized {
            return Err(ErffError::AlreadyFinalized);
        }

        // Validate column counts
        if feature.geometries.len() != self.schema.geometry_columns.len() {
            return Err(ErffError::SchemaMismatch {
                expected: self.schema.geometry_columns.len(),
                got: feature.geometries.len(),
            });
        }
        if feature.attributes.len() != self.schema.attribute_columns.len() {
            return Err(ErffError::SchemaMismatch {
                expected: self.schema.attribute_columns.len(),
                got: feature.attributes.len(),
            });
        }

        // Compute feature bbox from geometries
        let mut feature_env = Envelope::EMPTY;
        for wkb_data in feature.geometries.iter().flatten() {
            let env = wkb::wkb_envelope(wkb_data)?;
            feature_env.expand(&env);
        }

        // Record offset before writing
        let offset = self.writer.stream_position()?;
        self.feature_offsets.push(offset);
        self.feature_bboxes.push(feature_env);
        self.envelope.expand(&feature_env);

        // Serialize and write
        let buf = serialize_feature(feature, &self.schema)?;
        self.writer.write_all(&buf)?;

        self.feature_count += 1;
        Ok(())
    }

    /// Finalize the file: write offset table, spatial index, footer, and
    /// update the header with final feature count and envelope.
    pub fn finish(mut self) -> Result<W> {
        if self.finalized {
            return Err(ErffError::AlreadyFinalized);
        }

        // ── Feature Offset Table ──
        let offset_table_pos = self.writer.stream_position()?;
        for &off in &self.feature_offsets {
            self.writer.write_all(&off.to_le_bytes())?;
        }

        // ── Spatial Index ──
        let index_pos = self.writer.stream_position()?;
        let (feat_indices, nodes) =
            index::build_index(&self.feature_bboxes, &self.envelope, self.node_size);
        index::write_index(
            &mut self.writer,
            &feat_indices,
            &nodes,
            self.node_size,
            self.feature_count,
        )?;

        // ── Footer (32 bytes) ──
        self.writer.write_all(&offset_table_pos.to_le_bytes())?;
        self.writer.write_all(&index_pos.to_le_bytes())?;
        self.writer.write_all(&self.data_start_offset.to_le_bytes())?;
        self.writer.write_all(&MAGIC)?;
        self.writer.write_all(&[0u8; 4])?;

        // ── Update header ──
        self.writer.seek(SeekFrom::Start(8))?;
        self.writer.write_all(&self.feature_count.to_le_bytes())?;
        self.writer.write_all(&self.envelope.min_x.to_le_bytes())?;
        self.writer.write_all(&self.envelope.min_y.to_le_bytes())?;
        self.writer.write_all(&self.envelope.max_x.to_le_bytes())?;
        self.writer.write_all(&self.envelope.max_y.to_le_bytes())?;

        self.finalized = true;
        Ok(self.writer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_serialization_roundtrip() {
        let schema = Schema {
            crs: r#"{"type":"GeographicCRS"}"#.to_string(),
            geometry_columns: vec![GeometryColumnDef {
                name: "geom".into(),
                geom_type: GeometryType::Point,
                coord_type: CoordType::XY,
            }],
            attribute_columns: vec![
                AttributeColumnDef {
                    name: "name".into(),
                    col_type: ColumnType::String,
                    nullable: true,
                },
                AttributeColumnDef {
                    name: "pop".into(),
                    col_type: ColumnType::Int64,
                    nullable: false,
                },
            ],
            metadata: vec![("creator".into(), "test".into())],
        };

        let bytes = serialize_schema(&schema).unwrap();
        // Verify it's non-empty and starts with the CRS length
        assert!(bytes.len() > 10);
        let crs_len = u32::from_le_bytes(bytes[0..4].try_into().unwrap());
        assert_eq!(crs_len as usize, schema.crs.len());
    }
}
