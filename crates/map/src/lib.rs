//! Map data for layered maps: a small, bounded GeoJSON importer (phase 1 of the maps proposal, issue #150).
//!
//! **Attribution (required).** Map data from OpenStreetMap is licensed under the ODbL
//! (<https://www.openstreetmap.org/copyright>). Anything derived from it must carry the credit
//! [`OSM_CREDIT`] ("(c) OpenStreetMap contributors"): in the exported file's metadata and, optionally, as an
//! engraved text layer. The importer only reads; later phases that export must add the credit.
//!
//! The subset is GeoJSON per RFC 7946 (<https://www.rfc-editor.org/rfc/rfc7946>): `FeatureCollection`, `Feature`
//! and bare geometries of type Point, MultiPoint, LineString, MultiLineString, Polygon, MultiPolygon and
//! GeometryCollection. Positions are `[longitude, latitude]` in degrees. The input is untrusted: size, feature
//! count, vertices per feature, geometry nesting and tags are capped, coordinates must be finite and in range,
//! and nothing is indexed with data.

use std::collections::BTreeMap;

use serde_json::Value;

/// The credit every map export must carry (ODbL).
pub const OSM_CREDIT: &str = "(c) OpenStreetMap contributors";

pub const MAX_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_FEATURES: usize = 50_000;
pub const MAX_VERTICES_PER_FEATURE: usize = 100_000;
/// Nesting of `GeometryCollection`s inside one another.
pub const MAX_GEOMETRY_DEPTH: usize = 4;
pub const MAX_TAGS_PER_FEATURE: usize = 64;
pub const MAX_TAG_LEN: usize = 256;

/// A position in degrees; always finite, longitude in -180..=180, latitude in -90..=90.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LonLat {
    pub lon: f64,
    pub lat: f64,
}

/// A simple geometry; the `Multi*` types are flattened into several of these.
#[derive(Debug, Clone, PartialEq)]
pub enum Geometry {
    Point(LonLat),
    /// At least two positions.
    Line(Vec<LonLat>),
    /// Rings (outer first, then holes), each with at least three positions.
    Polygon(Vec<Vec<LonLat>>),
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Feature {
    /// Scalar `properties` entries as text (for example `highway` = `residential`).
    pub tags: BTreeMap<String, String>,
    /// Empty when the feature's geometry is null.
    pub geometries: Vec<Geometry>,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct MapData {
    pub features: Vec<Feature>,
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum MapError {
    #[error("the file is {0} bytes; the limit is {limit}", limit = MAX_BYTES)]
    TooLarge(usize),
    #[error("not valid JSON: {0}")]
    Json(String),
    #[error("not GeoJSON")]
    NotGeoJson,
    #[error("more than {limit} features", limit = MAX_FEATURES)]
    TooManyFeatures,
    #[error("a feature has more than {limit} vertices", limit = MAX_VERTICES_PER_FEATURE)]
    TooManyVertices,
    #[error("geometry collections nest deeper than {limit}", limit = MAX_GEOMETRY_DEPTH)]
    TooDeep,
    #[error("a coordinate is missing, not finite or out of range, or a line or ring is too short")]
    BadGeometry,
    #[error("unsupported geometry type {0:?}")]
    Unsupported(String),
}

/// Reads GeoJSON bytes. Errors, never panics.
pub fn import_geojson(data: &[u8]) -> Result<MapData, MapError> {
    if data.len() > MAX_BYTES {
        return Err(MapError::TooLarge(data.len()));
    }
    // serde_json bounds its own recursion, so deeply nested arrays are an error, not a stack overflow.
    let root: Value = serde_json::from_slice(data).map_err(|e| MapError::Json(e.to_string()))?;
    let kind = root.get("type").and_then(Value::as_str).ok_or(MapError::NotGeoJson)?;
    let mut features = Vec::new();
    match kind {
        "FeatureCollection" => {
            let items = root.get("features").and_then(Value::as_array).ok_or(MapError::NotGeoJson)?;
            if items.len() > MAX_FEATURES {
                return Err(MapError::TooManyFeatures);
            }
            for item in items {
                features.push(read_feature(item)?);
            }
        }
        "Feature" => features.push(read_feature(&root)?),
        _ => {
            let mut f = Feature::default();
            read_geometry(&root, 0, &mut 0, &mut f.geometries)?;
            features.push(f);
        }
    }
    Ok(MapData { features })
}

fn read_feature(v: &Value) -> Result<Feature, MapError> {
    if v.get("type").and_then(Value::as_str) != Some("Feature") {
        return Err(MapError::NotGeoJson);
    }
    let mut f = Feature::default();
    if let Some(props) = v.get("properties").and_then(Value::as_object) {
        for (k, val) in props {
            if f.tags.len() >= MAX_TAGS_PER_FEATURE {
                break;
            }
            let text = match val {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            if k.len() <= MAX_TAG_LEN && text.len() <= MAX_TAG_LEN {
                f.tags.insert(k.clone(), text);
            }
        }
    }
    match v.get("geometry") {
        None | Some(Value::Null) => {}
        Some(g) => read_geometry(g, 0, &mut 0, &mut f.geometries)?,
    }
    Ok(f)
}

/// `count` is the number of vertices read so far for the feature.
fn read_geometry(v: &Value, depth: usize, count: &mut usize, out: &mut Vec<Geometry>) -> Result<(), MapError> {
    if depth > MAX_GEOMETRY_DEPTH {
        return Err(MapError::TooDeep);
    }
    let kind = v.get("type").and_then(Value::as_str).ok_or(MapError::NotGeoJson)?;
    if kind == "GeometryCollection" {
        for g in array(v.get("geometries"))? {
            read_geometry(g, depth + 1, count, out)?;
        }
        return Ok(());
    }
    let coords = v.get("coordinates");
    match kind {
        "Point" => out.push(Geometry::Point(position(coords, count)?)),
        "MultiPoint" => {
            for p in array(coords)? {
                out.push(Geometry::Point(position(Some(p), count)?));
            }
        }
        "LineString" => out.push(Geometry::Line(line(coords, 2, count)?)),
        "MultiLineString" => {
            for l in array(coords)? {
                out.push(Geometry::Line(line(Some(l), 2, count)?));
            }
        }
        "Polygon" => out.push(Geometry::Polygon(rings(coords, count)?)),
        "MultiPolygon" => {
            for p in array(coords)? {
                out.push(Geometry::Polygon(rings(Some(p), count)?));
            }
        }
        other => return Err(MapError::Unsupported(other.chars().take(32).collect())),
    }
    Ok(())
}

fn array(v: Option<&Value>) -> Result<&Vec<Value>, MapError> {
    v.and_then(Value::as_array).ok_or(MapError::NotGeoJson)
}

fn rings(v: Option<&Value>, count: &mut usize) -> Result<Vec<Vec<LonLat>>, MapError> {
    array(v)?.iter().map(|r| line(Some(r), 3, count)).collect()
}

fn line(v: Option<&Value>, min: usize, count: &mut usize) -> Result<Vec<LonLat>, MapError> {
    let points: Vec<LonLat> = array(v)?.iter().map(|p| position(Some(p), count)).collect::<Result<_, _>>()?;
    if points.len() < min {
        return Err(MapError::BadGeometry);
    }
    Ok(points)
}

fn position(v: Option<&Value>, count: &mut usize) -> Result<LonLat, MapError> {
    *count += 1;
    if *count > MAX_VERTICES_PER_FEATURE {
        return Err(MapError::TooManyVertices);
    }
    let p = array(v)?;
    let lon = p.first().and_then(Value::as_f64).ok_or(MapError::BadGeometry)?;
    let lat = p.get(1).and_then(Value::as_f64).ok_or(MapError::BadGeometry)?;
    if !(lon.is_finite() && lat.is_finite() && (-180.0..=180.0).contains(&lon) && (-90.0..=90.0).contains(&lat)) {
        return Err(MapError::BadGeometry);
    }
    Ok(LonLat { lon, lat })
}

#[cfg(test)]
mod tests {
    use super::*;

    mod valid {
        use super::*;

        #[test]
        fn reads_a_feature_collection_with_tags_and_flattens_multi_geometries() {
            let data = import_geojson(SAMPLE.as_bytes()).unwrap();
            assert_eq!(data.features.len(), 3);
            assert_eq!(data.features[0].tags.get("highway").map(String::as_str), Some("residential"));
            assert_eq!(data.features[0].tags.get("lanes").map(String::as_str), Some("2"));
            assert_eq!(data.features[0].geometries, vec![Geometry::Line(vec![LonLat { lon: 8.5, lat: 47.3 }, LonLat { lon: 8.6, lat: 47.4 }])]);
            assert_eq!(data.features[1].geometries.len(), 2, "a MultiPolygon is two polygons");
            assert!(data.features[2].geometries.is_empty(), "a null geometry is an empty feature");
        }

        #[test]
        fn reads_a_bare_geometry_and_a_single_feature() {
            let bare = import_geojson(br#"{"type":"Point","coordinates":[1,2]}"#).unwrap();
            assert_eq!(bare.features[0].geometries, vec![Geometry::Point(LonLat { lon: 1.0, lat: 2.0 })]);
            let text = r#"{"type":"Feature","geometry":{"type":"GeometryCollection","geometries":[{"type":"Point","coordinates":[0,0]}]}}"#;
            assert_eq!(import_geojson(text.as_bytes()).unwrap().features[0].geometries.len(), 1);
        }

        #[test]
        fn the_credit_text_is_fixed() {
            assert_eq!(OSM_CREDIT, "(c) OpenStreetMap contributors");
        }
    }

    mod hostile {
        use super::*;

        #[test]
        fn rejects_oversized_files_before_parsing() {
            assert_eq!(import_geojson(&vec![b' '; MAX_BYTES + 1]), Err(MapError::TooLarge(MAX_BYTES + 1)));
        }

        #[test]
        fn rejects_garbage_and_non_geojson() {
            let no_features = r#"{"type":"FeatureCollection"}"#;
            let empty_feature = r#"{"type":"FeatureCollection","features":[{}]}"#;
            for bad in ["", "{", "[]", "null", r#"{"type":5}"#, no_features, empty_feature] {
                assert!(import_geojson(bad.as_bytes()).is_err(), "{bad}");
            }
            assert!(import_geojson(&[0xff, 0xfe, 0x00]).is_err());
        }

        #[test]
        fn rejects_bad_coordinates() {
            for bad in ["[1]", r#"[1,"x"]"#, "[181,0]", "[0,90.5]", "[0,-91]", "[1e999,0]", "[]", "5", "null"] {
                let text = format!(r#"{{"type":"Point","coordinates":{bad}}}"#);
                assert!(import_geojson(text.as_bytes()).is_err(), "{bad}");
            }
        }

        #[test]
        fn rejects_short_lines_and_rings() {
            assert_eq!(import_geojson(br#"{"type":"LineString","coordinates":[[0,0]]}"#), Err(MapError::BadGeometry));
            assert_eq!(import_geojson(br#"{"type":"Polygon","coordinates":[[[0,0],[1,1]]]}"#), Err(MapError::BadGeometry));
        }

        #[test]
        fn rejects_unsupported_geometry_types() {
            assert!(matches!(import_geojson(br#"{"type":"Circle","coordinates":[0,0]}"#), Err(MapError::Unsupported(_))));
        }

        #[test]
        fn caps_vertices_per_feature() {
            let points = vec!["[1,1]"; MAX_VERTICES_PER_FEATURE + 1].join(",");
            let text = format!(r#"{{"type":"LineString","coordinates":[{points}]}}"#);
            assert_eq!(import_geojson(text.as_bytes()), Err(MapError::TooManyVertices));
            let ok = vec!["[1,1]"; MAX_VERTICES_PER_FEATURE].join(",");
            assert!(import_geojson(format!(r#"{{"type":"LineString","coordinates":[{ok}]}}"#).as_bytes()).is_ok());
        }

        #[test]
        fn caps_features() {
            let items = vec![r#"{"type":"Feature","geometry":null}"#; MAX_FEATURES + 1].join(",");
            let text = format!(r#"{{"type":"FeatureCollection","features":[{items}]}}"#);
            assert_eq!(import_geojson(text.as_bytes()), Err(MapError::TooManyFeatures));
        }

        #[test]
        fn caps_geometry_collection_nesting_and_json_depth() {
            let nest = |n: usize| {
                let mut s = String::from(r#"{"type":"Point","coordinates":[0,0]}"#);
                for _ in 0..n {
                    s = format!(r#"{{"type":"GeometryCollection","geometries":[{s}]}}"#);
                }
                s
            };
            assert!(import_geojson(nest(MAX_GEOMETRY_DEPTH).as_bytes()).is_ok());
            assert_eq!(import_geojson(nest(MAX_GEOMETRY_DEPTH + 1).as_bytes()), Err(MapError::TooDeep));
            assert!(import_geojson(nest(300).as_bytes()).is_err(), "must not overflow the stack");
            let brackets = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
            assert!(import_geojson(brackets.as_bytes()).is_err());
        }

        #[test]
        fn caps_tags() {
            let props: Vec<String> = (0..200).map(|i| format!(r#""k{i}":"v""#)).collect();
            let long = "x".repeat(MAX_TAG_LEN + 1);
            let text = format!(r#"{{"type":"Feature","geometry":null,"properties":{{{},"long":"{long}","nested":{{"a":1}}}}}}"#, props.join(","));
            let data = import_geojson(text.as_bytes()).unwrap();
            assert!(data.features[0].tags.len() <= MAX_TAGS_PER_FEATURE);
            assert!(!data.features[0].tags.contains_key("long"));
            assert!(!data.features[0].tags.contains_key("nested"));
        }
    }

    pub const SAMPLE: &str = r#"{"type":"FeatureCollection","features":[
        {"type":"Feature","properties":{"highway":"residential","lanes":2,"name":null},"geometry":{"type":"LineString","coordinates":[[8.5,47.3],[8.6,47.4]]}},
        {"type":"Feature","properties":{"building":"yes"},"geometry":{"type":"MultiPolygon","coordinates":[
            [[[0,0],[1,0],[1,1],[0,0]]],
            [[[2,2],[3,2],[3,3],[2,2]],[[2.2,2.1],[2.8,2.1],[2.8,2.5],[2.2,2.1]]]]}},
        {"type":"Feature","properties":{},"geometry":null}]}"#;
}
