//! Core types for the ERFF format.

/// Axis-aligned bounding box.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Envelope {
    pub min_x: f64,
    pub min_y: f64,
    pub max_x: f64,
    pub max_y: f64,
}

impl Envelope {
    pub const EMPTY: Envelope = Envelope {
        min_x: f64::INFINITY,
        min_y: f64::INFINITY,
        max_x: f64::NEG_INFINITY,
        max_y: f64::NEG_INFINITY,
    };

    pub fn new(min_x: f64, min_y: f64, max_x: f64, max_y: f64) -> Self {
        Self { min_x, min_y, max_x, max_y }
    }

    pub fn is_empty(&self) -> bool {
        self.min_x > self.max_x || self.min_y > self.max_y
    }

    pub fn expand(&mut self, other: &Envelope) {
        if other.is_empty() {
            return;
        }
        self.min_x = self.min_x.min(other.min_x);
        self.min_y = self.min_y.min(other.min_y);
        self.max_x = self.max_x.max(other.max_x);
        self.max_y = self.max_y.max(other.max_y);
    }

    pub fn expand_xy(&mut self, x: f64, y: f64) {
        self.min_x = self.min_x.min(x);
        self.min_y = self.min_y.min(y);
        self.max_x = self.max_x.max(x);
        self.max_y = self.max_y.max(y);
    }

    pub fn intersects(&self, other: &Envelope) -> bool {
        self.min_x <= other.max_x
            && self.max_x >= other.min_x
            && self.min_y <= other.max_y
            && self.max_y >= other.min_y
    }

    pub fn center_x(&self) -> f64 {
        (self.min_x + self.max_x) * 0.5
    }

    pub fn center_y(&self) -> f64 {
        (self.min_y + self.max_y) * 0.5
    }
}

/// Geometry type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum GeometryType {
    Unknown = 0,
    Point = 1,
    LineString = 2,
    Polygon = 3,
    MultiPoint = 4,
    MultiLineString = 5,
    MultiPolygon = 6,
    GeometryCollection = 7,
}

impl GeometryType {
    pub fn from_u8(v: u8) -> crate::error::Result<Self> {
        match v {
            0 => Ok(Self::Unknown),
            1 => Ok(Self::Point),
            2 => Ok(Self::LineString),
            3 => Ok(Self::Polygon),
            4 => Ok(Self::MultiPoint),
            5 => Ok(Self::MultiLineString),
            6 => Ok(Self::MultiPolygon),
            7 => Ok(Self::GeometryCollection),
            _ => Err(crate::error::ErffError::InvalidGeometryType(v)),
        }
    }
}

/// Coordinate dimensionality.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum CoordType {
    XY = 0,
    XYZ = 1,
    XYM = 2,
    XYZM = 3,
}

impl CoordType {
    pub fn from_u8(v: u8) -> crate::error::Result<Self> {
        match v {
            0 => Ok(Self::XY),
            1 => Ok(Self::XYZ),
            2 => Ok(Self::XYM),
            3 => Ok(Self::XYZM),
            _ => Err(crate::error::ErffError::InvalidCoordType(v)),
        }
    }
}

/// Attribute column data type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ColumnType {
    Bool = 0,
    Int8 = 1,
    UInt8 = 2,
    Int16 = 3,
    UInt16 = 4,
    Int32 = 5,
    UInt32 = 6,
    Int64 = 7,
    UInt64 = 8,
    Float32 = 9,
    Float64 = 10,
    String = 11,
    Binary = 12,
    Date = 13,
    DateTime = 14,
    Json = 15,
}

impl ColumnType {
    pub fn from_u8(v: u8) -> crate::error::Result<Self> {
        match v {
            0 => Ok(Self::Bool),
            1 => Ok(Self::Int8),
            2 => Ok(Self::UInt8),
            3 => Ok(Self::Int16),
            4 => Ok(Self::UInt16),
            5 => Ok(Self::Int32),
            6 => Ok(Self::UInt32),
            7 => Ok(Self::Int64),
            8 => Ok(Self::UInt64),
            9 => Ok(Self::Float32),
            10 => Ok(Self::Float64),
            11 => Ok(Self::String),
            12 => Ok(Self::Binary),
            13 => Ok(Self::Date),
            14 => Ok(Self::DateTime),
            15 => Ok(Self::Json),
            _ => Err(crate::error::ErffError::InvalidColumnType(v)),
        }
    }

    /// Returns the fixed byte size, or None for variable-length types.
    pub fn fixed_size(&self) -> Option<usize> {
        match self {
            Self::Bool | Self::Int8 | Self::UInt8 => Some(1),
            Self::Int16 | Self::UInt16 => Some(2),
            Self::Int32 | Self::UInt32 | Self::Float32 | Self::Date => Some(4),
            Self::Int64 | Self::UInt64 | Self::Float64 | Self::DateTime => Some(8),
            Self::String | Self::Binary | Self::Json => None,
        }
    }
}

/// Definition of a geometry column.
#[derive(Debug, Clone)]
pub struct GeometryColumnDef {
    pub name: String,
    pub geom_type: GeometryType,
    pub coord_type: CoordType,
}

/// Definition of an attribute column.
#[derive(Debug, Clone)]
pub struct AttributeColumnDef {
    pub name: String,
    pub col_type: ColumnType,
    pub nullable: bool,
}

/// Dataset schema: CRS, columns, and metadata.
#[derive(Debug, Clone)]
pub struct Schema {
    /// CRS as a PROJJSON string (empty string if unspecified).
    pub crs: String,
    pub geometry_columns: Vec<GeometryColumnDef>,
    pub attribute_columns: Vec<AttributeColumnDef>,
    /// Arbitrary key-value metadata.
    pub metadata: Vec<(String, String)>,
}

impl Schema {
    pub fn total_columns(&self) -> usize {
        self.geometry_columns.len() + self.attribute_columns.len()
    }

    pub fn null_bitmap_bytes(&self) -> usize {
        self.total_columns().div_ceil(8)
    }
}

/// A typed attribute value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int8(i8),
    UInt8(u8),
    Int16(i16),
    UInt16(u16),
    Int32(i32),
    UInt32(u32),
    Int64(i64),
    UInt64(u64),
    Float32(f32),
    Float64(f64),
    String(String),
    Binary(Vec<u8>),
    /// Days since Unix epoch.
    Date(i32),
    /// Milliseconds since Unix epoch.
    DateTime(i64),
    Json(String),
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
}

/// A single feature with geometry and attribute data.
#[derive(Debug, Clone)]
pub struct Feature {
    /// One entry per geometry column in the schema. `None` = null geometry.
    pub geometries: Vec<Option<Vec<u8>>>,
    /// One entry per attribute column in the schema.
    pub attributes: Vec<Value>,
}

// -- File format constants --

pub const MAGIC: [u8; 4] = [0x45, 0x52, 0x46, 0x46]; // "ERFF"
pub const VERSION_MAJOR: u8 = 1;
pub const VERSION_MINOR: u8 = 0;
pub const HEADER_SIZE: u64 = 64;
pub const FOOTER_SIZE: u64 = 32;
pub const FLAG_HAS_SPATIAL_INDEX: u8 = 0x01;
pub const FLAG_HAS_OFFSET_TABLE: u8 = 0x02;
pub const DEFAULT_NODE_SIZE: u16 = 16;
