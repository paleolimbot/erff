# ERFF — Efficient Rouault File Format

**Version 1.0**

A single-file binary geospatial vector format designed for fast spatial queries,
memory-mapped I/O, and modern geospatial workflows. Named in honor of Even Rouault.

## Design Principles

1. **Single file** — no sidecars, no split files
2. **Memory-mappable** — fixed offsets, little-endian, aligned structures
3. **Built-in spatial index** — packed Hilbert R-tree for O(log n) spatial queries
4. **PROJJSON for CRS** — the modern, human-readable, lossless CRS encoding
5. **ISO WKB geometries** — standard, widely supported, no reinvention
6. **Multiple geometry columns** — full OGC compliance
7. **No arbitrary limits** — 64-bit offsets, 64-bit feature counts
8. **Rich type system** — nullable columns, dates, JSON, binary blobs
9. **Random access** — O(1) feature lookup by index via offset table
10. **Streamable writes** — header placeholders updated at finalization

## File Layout

```
┌──────────────────────────────────────┐
│ File Header              (64 bytes)  │
├──────────────────────────────────────┤
│ Schema Section           (variable)  │
├──────────────────────────────────────┤
│ Feature Data Section     (variable)  │
│  └─ Feature 0                        │
│  └─ Feature 1                        │
│  └─ ...                              │
├──────────────────────────────────────┤
│ Feature Offset Table     (variable)  │
│  └─ u64 offset per feature           │
├──────────────────────────────────────┤
│ Spatial Index Section    (variable)  │
│  └─ Packed Hilbert R-tree            │
├──────────────────────────────────────┤
│ File Footer              (32 bytes)  │
└──────────────────────────────────────┘
```

## Byte Order

All multi-byte values are **little-endian**.

## File Header (64 bytes, offset 0)

| Offset | Size | Type   | Description                                    |
|--------|------|--------|------------------------------------------------|
| 0      | 4    | bytes  | Magic: `ERFF` (0x45 0x52 0x46 0x46)           |
| 4      | 1    | u8     | Version major (1)                              |
| 5      | 1    | u8     | Version minor (0)                              |
| 6      | 1    | u8     | Flags (bit 0: has spatial index, bit 1: has offset table) |
| 7      | 1    | u8     | Reserved (0)                                   |
| 8      | 8    | u64    | Feature count                                  |
| 16     | 8    | f64    | Envelope min_x                                 |
| 24     | 8    | f64    | Envelope min_y                                 |
| 32     | 8    | f64    | Envelope max_x                                 |
| 40     | 8    | f64    | Envelope max_y                                 |
| 48     | 4    | u32    | Schema section size in bytes                   |
| 52     | 12   | bytes  | Reserved (zeros)                               |

## Schema Section (immediately after header)

### CRS
| Size | Type   | Description                    |
|------|--------|--------------------------------|
| 4    | u32    | CRS string length in bytes     |
| var  | UTF-8  | PROJJSON string                |

### Geometry Columns
| Size | Type   | Description                    |
|------|--------|--------------------------------|
| 2    | u16    | Number of geometry columns     |

Per geometry column:
| Size | Type   | Description                    |
|------|--------|--------------------------------|
| 2    | u16    | Column name length             |
| var  | UTF-8  | Column name                    |
| 1    | u8     | Geometry type (see below)      |
| 1    | u8     | Coordinate type (see below)    |

### Attribute Columns
| Size | Type   | Description                    |
|------|--------|--------------------------------|
| 2    | u16    | Number of attribute columns    |

Per attribute column:
| Size | Type   | Description                    |
|------|--------|--------------------------------|
| 2    | u16    | Column name length             |
| var  | UTF-8  | Column name                    |
| 1    | u8     | Column type (see below)        |
| 1    | u8     | Flags (bit 0: nullable)        |

### Metadata
| Size | Type   | Description                    |
|------|--------|--------------------------------|
| 4    | u32    | Number of metadata entries     |

Per entry:
| Size | Type   | Description                    |
|------|--------|--------------------------------|
| 2    | u16    | Key length                     |
| var  | UTF-8  | Key                            |
| 4    | u32    | Value length                   |
| var  | UTF-8  | Value                          |

## Feature Data Section

Each feature is stored as:

| Size | Type   | Description                              |
|------|--------|------------------------------------------|
| 4    | u32    | Total feature size in bytes (excl. this) |
| var  | bytes  | Null bitmap: ⌈total_columns / 8⌉ bytes  |
| var  | bytes  | Geometry columns (in schema order)       |
| var  | bytes  | Attribute columns (in schema order)      |

**Null bitmap**: Bit N corresponds to column N (geometry columns first, then
attribute columns). Bit set = value is present. Bit clear = value is null.
Bit 0 is the least significant bit of byte 0.

**Geometry column encoding** (if not null):
| Size | Type   | Description                    |
|------|--------|--------------------------------|
| 4    | u32    | WKB byte length                |
| var  | bytes  | ISO WKB encoded geometry       |

**Attribute column encoding** (if not null):
- Fixed-size types: raw bytes (e.g., i32 = 4 bytes LE)
- Variable-size types (String, Binary, Json):
  | Size | Type   | Description              |
  |------|--------|--------------------------|
  | 4    | u32    | Byte length              |
  | var  | bytes  | UTF-8 string or raw bytes|

## Feature Offset Table

An array of `feature_count` × u64 values, each being the byte offset from the
start of the file to the beginning of that feature's data (the u32 size field).

## Spatial Index Section

A packed Hilbert R-tree for efficient spatial queries. Features are sorted by
the Hilbert curve value of their bounding box center, and a static R-tree is
built bottom-up.

### Index Header
| Size | Type   | Description                    |
|------|--------|--------------------------------|
| 2    | u16    | Node size (default: 16)        |
| 8    | u64    | Number of items                |

### Feature Index Mapping
`num_items` × u64: maps Hilbert-sorted position to original feature index.

### Node Bounding Boxes
All levels stored contiguously. Level 0 is the item level (leaf bboxes),
then level 1, level 2, etc. up to the root.

Per node bbox:
| Size | Type   | Description                    |
|------|--------|--------------------------------|
| 8    | f64    | min_x                          |
| 8    | f64    | min_y                          |
| 8    | f64    | max_x                          |
| 8    | f64    | max_y                          |

Level sizes:
- Level 0: `num_items` nodes (one per feature)
- Level 1: ⌈level_0_count / node_size⌉ nodes
- Level k: ⌈level_(k-1)_count / node_size⌉ nodes
- Terminates when level has 1 node (root)

## File Footer (32 bytes)

| Offset | Size | Type   | Description                              |
|--------|------|--------|------------------------------------------|
| 0      | 8    | u64    | Offset to Feature Offset Table           |
| 8      | 8    | u64    | Offset to Spatial Index (0 if none)      |
| 16     | 8    | u64    | Offset to start of Feature Data Section  |
| 24     | 4    | bytes  | Magic: `ERFF`                            |
| 28     | 4    | bytes  | Reserved (zeros)                         |

The footer enables reading metadata from the end of the file, useful for
memory-mapped and append workflows.

## Type Enumerations

### Geometry Types

| Value | Type                |
|-------|---------------------|
| 0     | Unknown             |
| 1     | Point               |
| 2     | LineString          |
| 3     | Polygon             |
| 4     | MultiPoint          |
| 5     | MultiLineString     |
| 6     | MultiPolygon        |
| 7     | GeometryCollection  |

### Coordinate Types

| Value | Type |
|-------|------|
| 0     | XY   |
| 1     | XYZ  |
| 2     | XYM  |
| 3     | XYZM |

### Column Types

| Value | Type    | Size     |
|-------|---------|----------|
| 0     | Bool    | 1 byte   |
| 1     | Int8    | 1 byte   |
| 2     | UInt8   | 1 byte   |
| 3     | Int16   | 2 bytes  |
| 4     | UInt16  | 2 bytes  |
| 5     | Int32   | 4 bytes  |
| 6     | UInt32  | 4 bytes  |
| 7     | Int64   | 8 bytes  |
| 8     | UInt64  | 8 bytes  |
| 9     | Float32 | 4 bytes  |
| 10    | Float64 | 8 bytes  |
| 11    | String  | variable |
| 12    | Binary  | variable |
| 13    | Date    | 4 bytes (i32, days since Unix epoch) |
| 14    | DateTime| 8 bytes (i64, milliseconds since Unix epoch) |
| 15    | Json    | variable |
