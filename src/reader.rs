//! ERFF file reader.
//!
//! Supports sequential iteration, random access by feature index, and
//! spatial queries via the packed Hilbert R-tree.

use crate::error::{ErffError, Result};
use crate::index;
use crate::types::*;
use std::io::{Read, Seek, SeekFrom};

/// Reads an ERFF file.
pub struct ErffReader<R: Read + Seek> {
    reader: R,
    schema: Schema,
    feature_count: u64,
    envelope: Envelope,
    flags: u8,
    // Footer-derived offsets
    offset_table_pos: u64,
    index_pos: u64,
    data_start_pos: u64,
    // Loaded lazily
    feature_offsets: Option<Vec<u64>>,
    // Spatial index (loaded lazily)
    spatial_index: Option<SpatialIndex>,
}

struct SpatialIndex {
    feature_indices: Vec<u64>,
    nodes: Vec<Envelope>,
    node_size: u16,
    num_items: u64,
}

// ── Binary reading helpers ─────────────────────────────────────────────────

fn read_u8<R: Read>(r: &mut R) -> Result<u8> {
    let mut buf = [0u8; 1];
    r.read_exact(&mut buf)?;
    Ok(buf[0])
}

fn read_u16<R: Read>(r: &mut R) -> Result<u16> {
    let mut buf = [0u8; 2];
    r.read_exact(&mut buf)?;
    Ok(u16::from_le_bytes(buf))
}

fn read_u32<R: Read>(r: &mut R) -> Result<u32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(u32::from_le_bytes(buf))
}

fn read_i32<R: Read>(r: &mut R) -> Result<i32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(i32::from_le_bytes(buf))
}

fn read_u64<R: Read>(r: &mut R) -> Result<u64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(u64::from_le_bytes(buf))
}

fn read_i64<R: Read>(r: &mut R) -> Result<i64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(i64::from_le_bytes(buf))
}

fn read_f64<R: Read>(r: &mut R) -> Result<f64> {
    let mut buf = [0u8; 8];
    r.read_exact(&mut buf)?;
    Ok(f64::from_le_bytes(buf))
}

fn read_f32<R: Read>(r: &mut R) -> Result<f32> {
    let mut buf = [0u8; 4];
    r.read_exact(&mut buf)?;
    Ok(f32::from_le_bytes(buf))
}

fn read_i8<R: Read>(r: &mut R) -> Result<i8> {
    let mut buf = [0u8; 1];
    r.read_exact(&mut buf)?;
    Ok(i8::from_le_bytes(buf))
}

fn read_i16<R: Read>(r: &mut R) -> Result<i16> {
    let mut buf = [0u8; 2];
    r.read_exact(&mut buf)?;
    Ok(i16::from_le_bytes(buf))
}

fn read_string_u16<R: Read>(r: &mut R) -> Result<String> {
    let len = read_u16(r)? as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(String::from_utf8(buf)?)
}

fn read_string_u32<R: Read>(r: &mut R) -> Result<String> {
    let len = read_u32(r)? as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(String::from_utf8(buf)?)
}

fn read_bytes_u32<R: Read>(r: &mut R) -> Result<Vec<u8>> {
    let len = read_u32(r)? as usize;
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

// ── Schema deserialization ─────────────────────────────────────────────────

fn deserialize_schema<R: Read>(r: &mut R) -> Result<Schema> {
    // CRS
    let crs = read_string_u32(r)?;

    // Geometry columns
    let geom_count = read_u16(r)? as usize;
    let mut geometry_columns = Vec::with_capacity(geom_count);
    for _ in 0..geom_count {
        let name = read_string_u16(r)?;
        let geom_type = GeometryType::from_u8(read_u8(r)?)?;
        let coord_type = CoordType::from_u8(read_u8(r)?)?;
        geometry_columns.push(GeometryColumnDef { name, geom_type, coord_type });
    }

    // Attribute columns
    let attr_count = read_u16(r)? as usize;
    let mut attribute_columns = Vec::with_capacity(attr_count);
    for _ in 0..attr_count {
        let name = read_string_u16(r)?;
        let col_type = ColumnType::from_u8(read_u8(r)?)?;
        let flags = read_u8(r)?;
        let nullable = flags & 0x01 != 0;
        attribute_columns.push(AttributeColumnDef { name, col_type, nullable });
    }

    // Metadata
    let meta_count = read_u32(r)? as usize;
    let mut metadata = Vec::with_capacity(meta_count);
    for _ in 0..meta_count {
        let key = read_string_u16(r)?;
        let value = read_string_u32(r)?;
        metadata.push((key, value));
    }

    Ok(Schema { crs, geometry_columns, attribute_columns, metadata })
}

// ── Feature deserialization ────────────────────────────────────────────────

fn deserialize_feature<R: Read>(r: &mut R, schema: &Schema) -> Result<Feature> {
    let _size = read_u32(r)?;

    // Null bitmap
    let bitmap_bytes = schema.null_bitmap_bytes();
    let mut bitmap = vec![0u8; bitmap_bytes];
    r.read_exact(&mut bitmap)?;

    let geom_count = schema.geometry_columns.len();

    // Geometry columns
    let mut geometries = Vec::with_capacity(geom_count);
    for i in 0..geom_count {
        let present = bitmap[i / 8] & (1 << (i % 8)) != 0;
        if present {
            let wkb_data = read_bytes_u32(r)?;
            geometries.push(Some(wkb_data));
        } else {
            geometries.push(None);
        }
    }

    // Attribute columns
    let mut attributes = Vec::with_capacity(schema.attribute_columns.len());
    for (i, col) in schema.attribute_columns.iter().enumerate() {
        let col_idx = geom_count + i;
        let present = bitmap[col_idx / 8] & (1 << (col_idx % 8)) != 0;
        if !present {
            attributes.push(Value::Null);
            continue;
        }
        let val = match col.col_type {
            ColumnType::Bool => Value::Bool(read_u8(r)? != 0),
            ColumnType::Int8 => Value::Int8(read_i8(r)?),
            ColumnType::UInt8 => Value::UInt8(read_u8(r)?),
            ColumnType::Int16 => Value::Int16(read_i16(r)?),
            ColumnType::UInt16 => Value::UInt16(read_u16(r)?),
            ColumnType::Int32 => Value::Int32(read_i32(r)?),
            ColumnType::UInt32 => Value::UInt32(read_u32(r)?),
            ColumnType::Int64 => Value::Int64(read_i64(r)?),
            ColumnType::UInt64 => Value::UInt64(read_u64(r)?),
            ColumnType::Float32 => Value::Float32(read_f32(r)?),
            ColumnType::Float64 => Value::Float64(read_f64(r)?),
            ColumnType::String => Value::String(read_string_u32(r)?),
            ColumnType::Binary => Value::Binary(read_bytes_u32(r)?),
            ColumnType::Date => Value::Date(read_i32(r)?),
            ColumnType::DateTime => Value::DateTime(read_i64(r)?),
            ColumnType::Json => Value::Json(read_string_u32(r)?),
        };
        attributes.push(val);
    }

    Ok(Feature { geometries, attributes })
}

// ── Reader implementation ──────────────────────────────────────────────────

impl<R: Read + Seek> ErffReader<R> {
    /// Open an ERFF file for reading.
    pub fn open(mut reader: R) -> Result<Self> {
        // Read file header (64 bytes)
        let mut magic = [0u8; 4];
        reader.read_exact(&mut magic)?;
        if magic != MAGIC {
            return Err(ErffError::InvalidMagic);
        }

        let major = read_u8(&mut reader)?;
        let minor = read_u8(&mut reader)?;
        if major != VERSION_MAJOR {
            return Err(ErffError::UnsupportedVersion { major, minor });
        }

        let flags = read_u8(&mut reader)?;
        let _reserved = read_u8(&mut reader)?;
        let feature_count = read_u64(&mut reader)?;
        let min_x = read_f64(&mut reader)?;
        let min_y = read_f64(&mut reader)?;
        let max_x = read_f64(&mut reader)?;
        let max_y = read_f64(&mut reader)?;
        let schema_size = read_u32(&mut reader)?;

        // Skip reserved bytes (12)
        reader.seek(SeekFrom::Current(12))?;

        // Read schema
        let schema = deserialize_schema(&mut reader)?;

        let data_start_pos = HEADER_SIZE + schema_size as u64;

        // Read footer (last 32 bytes)
        reader.seek(SeekFrom::End(-(FOOTER_SIZE as i64)))?;
        let offset_table_pos = read_u64(&mut reader)?;
        let index_pos = read_u64(&mut reader)?;
        let _footer_data_start = read_u64(&mut reader)?;
        let mut footer_magic = [0u8; 4];
        reader.read_exact(&mut footer_magic)?;
        if footer_magic != MAGIC {
            return Err(ErffError::InvalidMagic);
        }

        let envelope = Envelope { min_x, min_y, max_x, max_y };

        Ok(Self {
            reader,
            schema,
            feature_count,
            envelope,
            flags,
            offset_table_pos,
            index_pos,
            data_start_pos,
            feature_offsets: None,
            spatial_index: None,
        })
    }

    pub fn schema(&self) -> &Schema {
        &self.schema
    }

    pub fn feature_count(&self) -> u64 {
        self.feature_count
    }

    pub fn envelope(&self) -> &Envelope {
        &self.envelope
    }

    /// Load the feature offset table (for random access).
    fn ensure_offsets(&mut self) -> Result<()> {
        if self.feature_offsets.is_some() {
            return Ok(());
        }
        self.reader.seek(SeekFrom::Start(self.offset_table_pos))?;
        let mut offsets = Vec::with_capacity(self.feature_count as usize);
        for _ in 0..self.feature_count {
            offsets.push(read_u64(&mut self.reader)?);
        }
        self.feature_offsets = Some(offsets);
        Ok(())
    }

    /// Load the spatial index.
    fn ensure_index(&mut self) -> Result<()> {
        if self.spatial_index.is_some() {
            return Ok(());
        }
        if self.flags & FLAG_HAS_SPATIAL_INDEX == 0 {
            return Err(ErffError::NoSpatialIndex);
        }
        self.reader.seek(SeekFrom::Start(self.index_pos))?;
        let (feature_indices, nodes, node_size, num_items) =
            index::read_index(&mut self.reader)?;
        self.spatial_index = Some(SpatialIndex {
            feature_indices,
            nodes,
            node_size,
            num_items,
        });
        Ok(())
    }

    /// Read a single feature by index (0-based).
    pub fn read_feature(&mut self, index: u64) -> Result<Feature> {
        if index >= self.feature_count {
            return Err(ErffError::FeatureOutOfRange(index, self.feature_count));
        }
        self.ensure_offsets()?;
        let offset = self.feature_offsets.as_ref().unwrap()[index as usize];
        self.reader.seek(SeekFrom::Start(offset))?;
        deserialize_feature(&mut self.reader, &self.schema)
    }

    /// Iterate over all features sequentially.
    pub fn features(&mut self) -> Result<Vec<Feature>> {
        self.reader.seek(SeekFrom::Start(self.data_start_pos))?;
        let mut features = Vec::with_capacity(self.feature_count as usize);
        for _ in 0..self.feature_count {
            features.push(deserialize_feature(&mut self.reader, &self.schema)?);
        }
        Ok(features)
    }

    /// Spatial query: return all features whose bounding box intersects the query envelope.
    pub fn query(&mut self, query: &Envelope) -> Result<Vec<Feature>> {
        self.ensure_index()?;
        self.ensure_offsets()?;

        let idx = self.spatial_index.as_ref().unwrap();
        let matching_indices = index::search_index(
            query,
            &idx.feature_indices,
            &idx.nodes,
            idx.num_items,
            idx.node_size,
        );

        let offsets = self.feature_offsets.as_ref().unwrap();
        let mut features = Vec::with_capacity(matching_indices.len());
        for feat_idx in matching_indices {
            let offset = offsets[feat_idx as usize];
            self.reader.seek(SeekFrom::Start(offset))?;
            features.push(deserialize_feature(&mut self.reader, &self.schema)?);
        }
        Ok(features)
    }

    /// Spatial query: return feature indices whose bounding box intersects the query envelope.
    pub fn query_indices(&mut self, query: &Envelope) -> Result<Vec<u64>> {
        self.ensure_index()?;
        let idx = self.spatial_index.as_ref().unwrap();
        Ok(index::search_index(
            query,
            &idx.feature_indices,
            &idx.nodes,
            idx.num_items,
            idx.node_size,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wkb;
    use crate::writer::ErffWriter;
    use std::io::Cursor;

    fn test_schema() -> Schema {
        Schema {
            crs: r#"{"$schema":"https://proj.org/schemas/v0.7/projjson.schema.json","type":"GeographicCRS","name":"WGS 84","datum":{"type":"GeodeticReferenceFrame","name":"World Geodetic System 1984","ellipsoid":{"name":"WGS 84","semi_major_axis":6378137,"inverse_flattening":298.257223563}},"coordinate_system":{"subtype":"ellipsoidal","axis":[{"name":"Geodetic latitude","abbreviation":"Lat","direction":"north","unit":"degree"},{"name":"Geodetic longitude","abbreviation":"Lon","direction":"east","unit":"degree"}]},"id":{"authority":"EPSG","code":4326}}"#.to_string(),
            geometry_columns: vec![GeometryColumnDef {
                name: "geometry".into(),
                geom_type: GeometryType::Point,
                coord_type: CoordType::XY,
            }],
            attribute_columns: vec![
                AttributeColumnDef { name: "name".into(), col_type: ColumnType::String, nullable: true },
                AttributeColumnDef { name: "population".into(), col_type: ColumnType::Int64, nullable: false },
                AttributeColumnDef { name: "elevation".into(), col_type: ColumnType::Float64, nullable: true },
            ],
            metadata: vec![
                ("creator".into(), "erff-rs".into()),
                ("description".into(), "Test dataset".into()),
            ],
        }
    }

    #[test]
    fn test_write_read_roundtrip() {
        let schema = test_schema();
        let mut buf = Cursor::new(Vec::new());

        // Write
        {
            let mut writer = ErffWriter::new(&mut buf, schema.clone()).unwrap();

            let cities = [
                ("London", -0.1276, 51.5074, 8_982_000i64, 11.0f64),
                ("Paris", 2.3522, 48.8566, 2_161_000, 35.0),
                ("Berlin", 13.4050, 52.5200, 3_748_000, 34.0),
                ("Madrid", -3.7038, 40.4168, 3_223_000, 667.0),
                ("Rome", 12.4964, 41.9028, 2_873_000, 21.0),
            ];

            for (name, lon, lat, pop, elev) in &cities {
                let geom = wkb::encode_point_wkb(*lon, *lat);
                writer.add_feature(&Feature {
                    geometries: vec![Some(geom)],
                    attributes: vec![
                        Value::String(name.to_string()),
                        Value::Int64(*pop),
                        Value::Float64(*elev),
                    ],
                }).unwrap();
            }

            writer.finish().unwrap();
        }

        // Read
        buf.seek(SeekFrom::Start(0)).unwrap();
        let mut reader = ErffReader::open(&mut buf).unwrap();

        assert_eq!(reader.feature_count(), 5);
        assert_eq!(reader.schema().geometry_columns.len(), 1);
        assert_eq!(reader.schema().attribute_columns.len(), 3);
        assert!(reader.schema().crs.contains("WGS 84"));
        assert_eq!(reader.schema().metadata.len(), 2);

        // Sequential read
        let features = reader.features().unwrap();
        assert_eq!(features.len(), 5);
        assert_eq!(features[0].attributes[0], Value::String("London".into()));
        assert_eq!(features[0].attributes[1], Value::Int64(8_982_000));

        // Random access
        let f2 = reader.read_feature(2).unwrap();
        assert_eq!(f2.attributes[0], Value::String("Berlin".into()));

        // Spatial query: area around London/Paris
        let results = reader.query(&Envelope::new(-1.0, 48.0, 3.0, 52.0)).unwrap();
        let names: Vec<&str> = results
            .iter()
            .map(|f| match &f.attributes[0] {
                Value::String(s) => s.as_str(),
                _ => "",
            })
            .collect();
        assert!(names.contains(&"London"));
        assert!(names.contains(&"Paris"));
        assert!(!names.contains(&"Madrid"));
    }

    #[test]
    fn test_null_values() {
        let schema = test_schema();
        let mut buf = Cursor::new(Vec::new());

        {
            let mut writer = ErffWriter::new(&mut buf, schema).unwrap();
            let geom = wkb::encode_point_wkb(0.0, 0.0);
            writer.add_feature(&Feature {
                geometries: vec![Some(geom)],
                attributes: vec![
                    Value::Null, // nullable name
                    Value::Int64(0),
                    Value::Null, // nullable elevation
                ],
            }).unwrap();
            writer.finish().unwrap();
        }

        buf.seek(SeekFrom::Start(0)).unwrap();
        let mut reader = ErffReader::open(&mut buf).unwrap();
        let features = reader.features().unwrap();
        assert_eq!(features[0].attributes[0], Value::Null);
        assert_eq!(features[0].attributes[1], Value::Int64(0));
        assert_eq!(features[0].attributes[2], Value::Null);
    }
}
