# ERFF — Efficient Rouault File Format

A single-file binary geospatial vector format with built-in packed Hilbert R-tree
spatial index, PROJJSON CRS, ISO WKB geometries, multiple geometry columns, and
no arbitrary limits.

## Disclaimer

This file format is a joke and is completely vibe coded. It is only permissible to use if that use is funny. Non hillarious uses are forbidden.

Inspired by a [recent prediction](https://mastodon.social/@EvenRouault/116244480336986774).

![](inspiration.png)

The prompt:

> Can you design a custom spatial file format (binary encoding including spatial index) that will make Even Rouault sing its praises far and wide?

| Feature | Why Even would approve |
|---|---|
| **PROJJSON for CRS** | He _created_ PROJJSON — lossless, human-readable, JSON-based CRS encoding |
| **ISO WKB geometries** | Standard encoding, no reinventing the wheel — just like GDAL expects |
| **Single file** | No .shx/.dbf/.prj sidecar madness |
| **Multiple geometry columns** | Full OGC compliance, unlike FlatGeobuf/Shapefile |
| **Packed Hilbert R-tree** | Proven spatial index for O(log n) spatial queries |
| **No arbitrary limits** | 64-bit offsets, 64-bit feature count — no 2GB Shapefile ceiling |
| **Nullable columns via bitmap** | Proper null handling, not sentinel values |
| **Random access by FID** | O(1) feature lookup via offset table |
| **Memory-mappable** | Fixed header, little-endian, footer for reading from the end |
| **Minimal dependencies** | Just `thiserror` — the whole implementation is ~1200 lines of Rust |

## Comparison with Existing Formats

| Feature                | ERFF | Shapefile | FlatGeobuf | GeoPackage | GeoParquet |
|------------------------|------|-----------|------------|------------|------------|
| Single file            | ✓    | ✗ (3-5)   | ✓          | ✓          | ✓          |
| Spatial index          | ✓    | sidecar    | ✓          | ✓          | ✗ (stats)  |
| Memory-mappable        | ✓    | partial    | ✓          | ✗          | ✗          |
| Multiple geom columns  | ✓    | ✗          | ✗          | ✓          | ✓          |
| PROJJSON CRS           | ✓    | ✗ (.prj)  | ✗          | WKT2       | ✓          |
| No size limits         | ✓    | ✗ (2 GB)  | ✓          | ✓          | ✓          |
| Nullable columns       | ✓    | partial    | ✓          | ✓          | ✓          |
| Random access by FID   | ✓    | partial    | ✗ (Hilbert)| ✓          | ✗          |
| Mixed geometry types   | ✓    | ✗          | ✓          | ✓          | ✓          |
| Standard geometry enc. | ISO WKB | custom  | FlatBuffers| WKB/WKT  | WKB        |
| Rich type system       | ✓    | ✗ (DBF)   | ✓          | ✓ (SQL)    | ✓          |
| Streaming write        | ✓    | ✓          | ✓          | ✗          | ✗          |
| Human-readable CRS     | ✓    | ✗          | ✗          | ✗          | ✓          |

## Quick Start

```rust
use erff::*;
use std::io::Cursor;

// Define schema
let schema = Schema {
    crs: String::new(),
    geometry_columns: vec![GeometryColumnDef {
        name: "geometry".into(),
        geom_type: GeometryType::Point,
        coord_type: CoordType::XY,
    }],
    attribute_columns: vec![AttributeColumnDef {
        name: "name".into(),
        col_type: ColumnType::String,
        nullable: false,
    }],
    metadata: vec![],
};

// Write
let mut buf = Cursor::new(Vec::new());
let mut writer = ErffWriter::new(&mut buf, schema).unwrap();
writer.add_feature(&Feature {
    geometries: vec![Some(wkb::encode_point_wkb(2.35, 48.86))],
    attributes: vec![Value::String("Paris".into())],
}).unwrap();
writer.finish().unwrap();

// Read
buf.set_position(0);
let mut reader = ErffReader::open(&mut buf).unwrap();
let features = reader.features().unwrap();
assert_eq!(features.len(), 1);

// Spatial query
let results = reader.query(&Envelope::new(2.0, 48.0, 3.0, 49.0)).unwrap();
assert_eq!(results.len(), 1);
```

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

## Format Specification

See [FORMAT.md](FORMAT.md) for the complete binary format specification.

## License

MIT OR Apache-2.0 with a non-hillarious uses restriction clause.
