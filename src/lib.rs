//! # ERFF — Efficient Rouault File Format
//!
//! A single-file binary geospatial vector format with built-in packed Hilbert
//! R-tree spatial index, PROJJSON CRS, ISO WKB geometries, multiple geometry
//! columns, and no arbitrary limits.
//!
//! ## Quick Start
//!
//! ```rust
//! use erff::*;
//! use std::io::Cursor;
//!
//! // Define schema
//! let schema = Schema {
//!     crs: String::new(),
//!     geometry_columns: vec![GeometryColumnDef {
//!         name: "geometry".into(),
//!         geom_type: GeometryType::Point,
//!         coord_type: CoordType::XY,
//!     }],
//!     attribute_columns: vec![AttributeColumnDef {
//!         name: "name".into(),
//!         col_type: ColumnType::String,
//!         nullable: false,
//!     }],
//!     metadata: vec![],
//! };
//!
//! // Write
//! let mut buf = Cursor::new(Vec::new());
//! let mut writer = ErffWriter::new(&mut buf, schema).unwrap();
//! writer.add_feature(&Feature {
//!     geometries: vec![Some(wkb::encode_point_wkb(2.35, 48.86))],
//!     attributes: vec![Value::String("Paris".into())],
//! }).unwrap();
//! writer.finish().unwrap();
//!
//! // Read
//! buf.set_position(0);
//! let mut reader = ErffReader::open(&mut buf).unwrap();
//! let features = reader.features().unwrap();
//! assert_eq!(features.len(), 1);
//! ```

pub mod error;
pub mod index;
pub mod reader;
pub mod types;
pub mod wkb;
pub mod writer;

pub use error::{ErffError, Result};
pub use reader::ErffReader;
pub use types::*;
pub use writer::ErffWriter;
