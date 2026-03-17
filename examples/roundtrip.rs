/// Roundtrip example: write and read an ERFF file with European capitals.

use erff::*;
use std::fs::File;

fn main() -> erff::Result<()> {
    let path = "capitals.erff";

    // ── Schema ─────────────────────────────────────────────────────────
    let schema = Schema {
        crs: r#"{
  "$schema": "https://proj.org/schemas/v0.7/projjson.schema.json",
  "type": "GeographicCRS",
  "name": "WGS 84",
  "datum": {
    "type": "GeodeticReferenceFrame",
    "name": "World Geodetic System 1984",
    "ellipsoid": {
      "name": "WGS 84",
      "semi_major_axis": 6378137,
      "inverse_flattening": 298.257223563
    }
  },
  "coordinate_system": {
    "subtype": "ellipsoidal",
    "axis": [
      {"name": "Geodetic latitude", "abbreviation": "Lat", "direction": "north", "unit": "degree"},
      {"name": "Geodetic longitude", "abbreviation": "Lon", "direction": "east", "unit": "degree"}
    ]
  },
  "id": {"authority": "EPSG", "code": 4326}
}"#
        .to_string(),
        geometry_columns: vec![GeometryColumnDef {
            name: "geometry".into(),
            geom_type: GeometryType::Point,
            coord_type: CoordType::XY,
        }],
        attribute_columns: vec![
            AttributeColumnDef {
                name: "name".into(),
                col_type: ColumnType::String,
                nullable: false,
            },
            AttributeColumnDef {
                name: "country".into(),
                col_type: ColumnType::String,
                nullable: false,
            },
            AttributeColumnDef {
                name: "population".into(),
                col_type: ColumnType::Int64,
                nullable: false,
            },
            AttributeColumnDef {
                name: "elevation_m".into(),
                col_type: ColumnType::Float64,
                nullable: true,
            },
        ],
        metadata: vec![
            ("creator".into(), "erff-rs roundtrip example".into()),
            ("source".into(), "Wikipedia".into()),
        ],
    };

    // ── Write ──────────────────────────────────────────────────────────
    println!("Writing {path}...");
    let file = File::create(path)?;
    let mut writer = ErffWriter::new(file, schema)?;

    // (name, country, lon, lat, population, elevation_m)
    let cities: &[(&str, &str, f64, f64, i64, Option<f64>)] = &[
        ("London", "United Kingdom", -0.1276, 51.5074, 8_982_000, Some(11.0)),
        ("Paris", "France", 2.3522, 48.8566, 2_161_000, Some(35.0)),
        ("Berlin", "Germany", 13.4050, 52.5200, 3_748_000, Some(34.0)),
        ("Madrid", "Spain", -3.7038, 40.4168, 3_223_000, Some(667.0)),
        ("Rome", "Italy", 12.4964, 41.9028, 2_873_000, Some(21.0)),
        ("Amsterdam", "Netherlands", 4.9041, 52.3676, 872_000, Some(-2.0)),
        ("Brussels", "Belgium", 4.3517, 50.8503, 1_209_000, Some(13.0)),
        ("Vienna", "Austria", 16.3738, 48.2082, 1_911_000, Some(171.0)),
        ("Warsaw", "Poland", 21.0122, 52.2297, 1_790_000, Some(100.0)),
        ("Lisbon", "Portugal", -9.1393, 38.7223, 505_000, Some(2.0)),
        ("Athens", "Greece", 23.7275, 37.9838, 664_000, Some(70.0)),
        ("Stockholm", "Sweden", 18.0686, 59.3293, 975_000, None),
        ("Oslo", "Norway", 10.7522, 59.9139, 693_000, None),
        ("Helsinki", "Finland", 24.9384, 60.1699, 656_000, Some(0.0)),
        ("Copenhagen", "Denmark", 12.5683, 55.6761, 602_000, Some(1.0)),
    ];

    for &(name, country, lon, lat, pop, elev) in cities {
        let geom = wkb::encode_point_wkb(lon, lat);
        writer.add_feature(&Feature {
            geometries: vec![Some(geom)],
            attributes: vec![
                Value::String(name.into()),
                Value::String(country.into()),
                Value::Int64(pop),
                match elev {
                    Some(e) => Value::Float64(e),
                    None => Value::Null,
                },
            ],
        })?;
    }

    writer.finish()?;
    println!("Wrote {path} with {} features", cities.len());

    // ── Read ───────────────────────────────────────────────────────────
    println!("\nReading {path}...");
    let file = File::open(path)?;
    let mut reader = ErffReader::open(file)?;

    println!("Feature count: {}", reader.feature_count());
    println!(
        "Envelope: ({:.4}, {:.4}) — ({:.4}, {:.4})",
        reader.envelope().min_x,
        reader.envelope().min_y,
        reader.envelope().max_x,
        reader.envelope().max_y
    );
    println!("CRS: {} chars of PROJJSON", reader.schema().crs.len());
    println!(
        "Geometry columns: {:?}",
        reader.schema().geometry_columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );
    println!(
        "Attribute columns: {:?}",
        reader.schema().attribute_columns.iter().map(|c| &c.name).collect::<Vec<_>>()
    );
    println!("Metadata: {:?}", reader.schema().metadata);

    // Sequential read
    println!("\n── All features ──");
    let features = reader.features()?;
    for (i, f) in features.iter().enumerate() {
        let name = match &f.attributes[0] {
            Value::String(s) => s.as_str(),
            _ => "?",
        };
        let country = match &f.attributes[1] {
            Value::String(s) => s.as_str(),
            _ => "?",
        };
        let pop = match &f.attributes[2] {
            Value::Int64(v) => *v,
            _ => 0,
        };
        let elev = match &f.attributes[3] {
            Value::Float64(v) => format!("{v}m"),
            Value::Null => "N/A".to_string(),
            _ => "?".to_string(),
        };
        println!("  [{i:2}] {name}, {country} — pop: {pop}, elev: {elev}");
    }

    // Random access
    println!("\n── Random access (feature #7) ──");
    let f = reader.read_feature(7)?;
    println!("  {:?}", f.attributes);

    // Spatial query: Western Europe roughly (-10, 38) to (5, 55)
    println!("\n── Spatial query: Western Europe ──");
    let query = Envelope::new(-10.0, 38.0, 5.0, 55.0);
    let results = reader.query(&query)?;
    for f in &results {
        let name = match &f.attributes[0] {
            Value::String(s) => s.as_str(),
            _ => "?",
        };
        println!("  {name}");
    }

    // Spatial query: Scandinavia
    println!("\n── Spatial query: Scandinavia ──");
    let query = Envelope::new(5.0, 55.0, 30.0, 65.0);
    let indices = reader.query_indices(&query)?;
    println!("  Matching feature indices: {indices:?}");
    for idx in indices {
        let f = reader.read_feature(idx)?;
        let name = match &f.attributes[0] {
            Value::String(s) => s.as_str(),
            _ => "?",
        };
        println!("  [{idx}] {name}");
    }

    // Clean up
    std::fs::remove_file(path)?;
    println!("\nDone! Cleaned up {path}.");

    Ok(())
}
