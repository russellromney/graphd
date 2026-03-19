use base64::Engine;
use chrono::Datelike;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::error::GraphdError;

/// Internal ID for nodes and relationships in the graph database.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InternalId {
    pub table: u64,
    pub offset: u64,
}

/// A value returned from a Cypher query, encoded as JSON per Neo4j HTTP API spec.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum GraphValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Timestamp(chrono::DateTime<chrono::FixedOffset>),
    Date(String),
    Base64(String),
    Duration(String),
    List(Vec<GraphValue>),
    Map(HashMap<String, GraphValue>),
    Tagged(TaggedValue),
}

/// Graph-specific types that use a `$type` discriminator.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "$type", rename_all = "lowercase")]
pub enum TaggedValue {
    Node {
        id: InternalId,
        label: String,
        properties: HashMap<String, GraphValue>,
    },
    Rel {
        id: InternalId,
        label: String,
        src: InternalId,
        dst: InternalId,
        properties: HashMap<String, GraphValue>,
    },
    Path {
        nodes: Vec<GraphValue>,
        rels: Vec<GraphValue>,
    },
    Union {
        tag: String,
        value: Box<GraphValue>,
    },
}

/// Convert a LadybugDB Value into a GraphValue for JSON serialization.
pub fn from_lbug_value(value: &lbug::Value) -> GraphValue {
    match value {
        lbug::Value::Null(_) => GraphValue::Null,
        lbug::Value::Bool(b) => GraphValue::Bool(*b),
        lbug::Value::Int8(n) => GraphValue::Int(*n as i64),
        lbug::Value::Int16(n) => GraphValue::Int(*n as i64),
        lbug::Value::Int32(n) => GraphValue::Int(*n as i64),
        lbug::Value::Int64(n) => GraphValue::Int(*n),
        lbug::Value::Int128(n) => GraphValue::String(n.to_string()),
        lbug::Value::UInt8(n) => GraphValue::Int(*n as i64),
        lbug::Value::UInt16(n) => GraphValue::Int(*n as i64),
        lbug::Value::UInt32(n) => GraphValue::Int(*n as i64),
        lbug::Value::UInt64(n) => {
            if *n <= i64::MAX as u64 {
                GraphValue::Int(*n as i64)
            } else {
                GraphValue::String(n.to_string())
            }
        }
        lbug::Value::Float(f) => GraphValue::Float(*f as f64),
        lbug::Value::Double(f) => GraphValue::Float(*f),
        lbug::Value::Decimal(d) => GraphValue::String(d.to_string()),
        lbug::Value::String(s) => GraphValue::String(s.clone()),
        lbug::Value::Blob(b) => {
            GraphValue::Base64(base64::engine::general_purpose::STANDARD.encode(b))
        }
        lbug::Value::UUID(u) => GraphValue::String(u.to_string()),
        lbug::Value::Date(d) => GraphValue::Date(d.to_string()),
        lbug::Value::Timestamp(t)
        | lbug::Value::TimestampTz(t)
        | lbug::Value::TimestampNs(t)
        | lbug::Value::TimestampMs(t)
        | lbug::Value::TimestampSec(t) => {
            let offset_secs = t.offset().whole_seconds();
            let fixed_offset =
                chrono::FixedOffset::east_opt(offset_secs).expect("valid UTC offset");
            #[allow(deprecated)]
            let naive =
                chrono::NaiveDateTime::from_timestamp_opt(t.unix_timestamp(), t.nanosecond())
                    .expect("valid timestamp");
            GraphValue::Timestamp(chrono::DateTime::from_naive_utc_and_offset(
                naive,
                fixed_offset,
            ))
        }
        lbug::Value::Interval(d) => GraphValue::Duration(time_duration_to_iso8601(d)),
        lbug::Value::List(_, items) | lbug::Value::Array(_, items) => {
            GraphValue::List(items.iter().map(from_lbug_value).collect())
        }
        lbug::Value::Map(_, entries) => {
            let mut map = HashMap::new();
            for (k, v) in entries {
                let key = match k {
                    lbug::Value::String(s) => s.clone(),
                    other => format!("{other:?}"),
                };
                map.insert(key, from_lbug_value(v));
            }
            GraphValue::Map(map)
        }
        lbug::Value::Struct(fields) => {
            let mut map = HashMap::new();
            for (key, val) in fields {
                map.insert(key.clone(), from_lbug_value(val));
            }
            GraphValue::Map(map)
        }
        lbug::Value::Node(node) => {
            let id = InternalId {
                table: node.get_node_id().table_id,
                offset: node.get_node_id().offset,
            };
            let mut properties = HashMap::new();
            for (key, val) in node.get_properties() {
                properties.insert(key.clone(), from_lbug_value(val));
            }
            GraphValue::Tagged(TaggedValue::Node {
                id,
                label: node.get_label_name().clone(),
                properties,
            })
        }
        lbug::Value::Rel(rel) => {
            let src = InternalId {
                table: rel.get_src_node().table_id,
                offset: rel.get_src_node().offset,
            };
            let dst = InternalId {
                table: rel.get_dst_node().table_id,
                offset: rel.get_dst_node().offset,
            };
            let mut properties = HashMap::new();
            for (key, val) in rel.get_properties() {
                properties.insert(key.clone(), from_lbug_value(val));
            }
            GraphValue::Tagged(TaggedValue::Rel {
                id: InternalId { table: 0, offset: 0 }, // RelVal doesn't expose its own ID
                label: rel.get_label_name().clone(),
                src,
                dst,
                properties,
            })
        }
        lbug::Value::RecursiveRel { nodes, rels } => {
            GraphValue::Tagged(TaggedValue::Path {
                nodes: nodes.iter().map(|n| from_lbug_value(&lbug::Value::Node(n.clone()))).collect(),
                rels: rels.iter().map(|r| from_lbug_value(&lbug::Value::Rel(r.clone()))).collect(),
            })
        }
        lbug::Value::InternalID(id) => GraphValue::Map(HashMap::from([
            ("table".to_string(), GraphValue::Int(id.table_id as i64)),
            ("offset".to_string(), GraphValue::Int(id.offset as i64)),
        ])),
        lbug::Value::Union { value, .. } => from_lbug_value(value),
    }
}

/// Infer the logical type from a lbug Value (for list element type).
pub fn lbug_value_logical_type(v: &lbug::Value) -> lbug::LogicalType {
    match v {
        lbug::Value::Bool(_) => lbug::LogicalType::Bool,
        lbug::Value::Int64(_) => lbug::LogicalType::Int64,
        lbug::Value::UInt64(_) => lbug::LogicalType::UInt64,
        lbug::Value::Double(_) => lbug::LogicalType::Double,
        lbug::Value::Float(_) => lbug::LogicalType::Float,
        lbug::Value::String(_) => lbug::LogicalType::String,
        lbug::Value::Date(_) => lbug::LogicalType::Date,
        lbug::Value::Blob(_) => lbug::LogicalType::Blob,
        lbug::Value::Interval(_) => lbug::LogicalType::Interval,
        _ => lbug::LogicalType::Any,
    }
}

/// Convert a JSON value to a LadybugDB value for parameter binding.
pub fn to_lbug_value(json: &serde_json::Value) -> Result<lbug::Value, GraphdError> {
    match json {
        serde_json::Value::Null => Ok(lbug::Value::Null(lbug::LogicalType::Any)),
        serde_json::Value::Bool(b) => Ok(lbug::Value::Bool(*b)),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Ok(lbug::Value::Int64(i))
            } else if let Some(u) = n.as_u64() {
                Ok(lbug::Value::UInt64(u))
            } else if let Some(f) = n.as_f64() {
                Ok(lbug::Value::Double(f))
            } else {
                Err(GraphdError::TypeError("Unsupported number type".into()))
            }
        }
        serde_json::Value::String(s) => Ok(lbug::Value::String(s.clone())),
        serde_json::Value::Array(arr) => {
            let items: Result<Vec<lbug::Value>, GraphdError> =
                arr.iter().map(to_lbug_value).collect();
            let items = items?;
            let elem_type = items
                .first()
                .map(lbug_value_logical_type)
                .unwrap_or(lbug::LogicalType::Any);
            Ok(lbug::Value::List(elem_type, items))
        }
        serde_json::Value::Object(obj) => match (obj.get("$type"), obj.get("_value")) {
            (Some(serde_json::Value::String(ty)), Some(val)) => typed_json_to_lbug(ty, val),
            _ => Err(GraphdError::TypeError(
                "Object params require {\"$type\": ..., \"_value\": ...} typed JSON format".into(),
            )),
        },
    }
}

/// Convert a JSON params object to a Vec of (name, lbug::Value) pairs.
pub fn json_params_to_lbug(
    params: &serde_json::Value,
) -> Result<Vec<(String, lbug::Value)>, GraphdError> {
    let obj = params
        .as_object()
        .ok_or_else(|| GraphdError::TypeError("params must be a JSON object".into()))?;
    obj.iter()
        .map(|(k, v)| Ok((k.clone(), to_lbug_value(v)?)))
        .collect()
}

/// Convert a Neo4j Query API v2 typed JSON value to a lbug Value.
///
/// Typed JSON uses `{"$type": "TypeName", "_value": ...}` wrappers to encode
/// types that plain JSON cannot represent (datetimes, dates, etc.).
/// See: <https://neo4j.com/docs/query-api/current/typed-json/>
fn typed_json_to_lbug(type_name: &str, value: &serde_json::Value) -> Result<lbug::Value, GraphdError> {
    match type_name {
        "Null" => Ok(lbug::Value::Null(lbug::LogicalType::Any)),
        "Boolean" => value
            .as_bool()
            .map(lbug::Value::Bool)
            .ok_or_else(|| GraphdError::TypeError("Boolean _value must be true/false".into())),
        "Integer" => {
            let s = value
                .as_str()
                .or_else(|| None)
                .ok_or_else(|| GraphdError::TypeError("Integer _value must be a string".into()))?;
            s.parse::<i64>()
                .map(lbug::Value::Int64)
                .map_err(|e| GraphdError::TypeError(format!("Invalid Integer: {e}")))
        }
        "Float" => {
            let s = value
                .as_str()
                .ok_or_else(|| GraphdError::TypeError("Float _value must be a string".into()))?;
            s.parse::<f64>()
                .map(lbug::Value::Double)
                .map_err(|e| GraphdError::TypeError(format!("Invalid Float: {e}")))
        }
        "String" => value
            .as_str()
            .map(|s| lbug::Value::String(s.to_string()))
            .ok_or_else(|| GraphdError::TypeError("String _value must be a string".into())),
        "OffsetDateTime" | "ZonedDateTime" => {
            let s = value
                .as_str()
                .ok_or_else(|| GraphdError::TypeError("DateTime _value must be an ISO-8601 string".into()))?;
            // Strip optional IANA zone suffix: "2024-01-15T10:30:00Z[Europe/London]" → "2024-01-15T10:30:00Z"
            let iso = s.split('[').next().unwrap_or(s);
            let dt = chrono::DateTime::parse_from_rfc3339(iso)
                .map_err(|e| GraphdError::TypeError(format!("Invalid datetime: {e}")))?;
            chrono_to_lbug_timestamp(&dt)
        }
        "LocalDateTime" => {
            let s = value
                .as_str()
                .ok_or_else(|| GraphdError::TypeError("LocalDateTime _value must be an ISO-8601 string".into()))?;
            // Parse as naive, assume UTC
            let naive = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
                .or_else(|_| chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S"))
                .map_err(|e| GraphdError::TypeError(format!("Invalid LocalDateTime: {e}")))?;
            let dt = chrono::DateTime::<chrono::FixedOffset>::from_naive_utc_and_offset(
                naive,
                chrono::FixedOffset::east_opt(0).expect("UTC offset"),
            );
            chrono_to_lbug_timestamp(&dt)
        }
        "Date" => {
            let s = value
                .as_str()
                .ok_or_else(|| GraphdError::TypeError("Date _value must be a string".into()))?;
            let d = chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
                .map_err(|e| GraphdError::TypeError(format!("Invalid Date: {e}")))?;
            let date = time::Date::from_calendar_date(
                d.year(),
                time::Month::try_from(d.month() as u8)
                    .map_err(|e| GraphdError::TypeError(format!("Invalid month: {e}")))?,
                d.day() as u8,
            )
            .map_err(|e| GraphdError::TypeError(format!("Invalid Date: {e}")))?;
            Ok(lbug::Value::Date(date))
        }
        "Duration" => {
            let s = value
                .as_str()
                .ok_or_else(|| GraphdError::TypeError("Duration _value must be a string".into()))?;
            let dur = parse_iso8601_duration(s)?;
            Ok(lbug::Value::Interval(dur))
        }
        "Base64" => {
            let s = value
                .as_str()
                .ok_or_else(|| GraphdError::TypeError("Base64 _value must be a string".into()))?;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(s)
                .map_err(|e| GraphdError::TypeError(format!("Invalid Base64: {e}")))?;
            Ok(lbug::Value::Blob(bytes))
        }
        "List" => {
            let arr = value
                .as_array()
                .ok_or_else(|| GraphdError::TypeError("List _value must be an array".into()))?;
            let items: Result<Vec<lbug::Value>, GraphdError> =
                arr.iter().map(to_lbug_value).collect();
            let items = items?;
            let elem_type = items
                .first()
                .map(lbug_value_logical_type)
                .unwrap_or(lbug::LogicalType::Any);
            Ok(lbug::Value::List(elem_type, items))
        }
        "Map" => {
            let obj = value
                .as_object()
                .ok_or_else(|| GraphdError::TypeError("Map _value must be an object".into()))?;
            let entries: Result<Vec<(lbug::Value, lbug::Value)>, GraphdError> = obj
                .iter()
                .map(|(k, v)| {
                    let key = lbug::Value::String(k.clone());
                    let val = to_lbug_value(v)?;
                    Ok((key, val))
                })
                .collect();
            Ok(lbug::Value::Map(
                (lbug::LogicalType::String, lbug::LogicalType::Any),
                entries?,
            ))
        }
        _ => Err(GraphdError::TypeError(format!(
            "Unsupported typed JSON $type: {type_name}"
        ))),
    }
}

/// Convert a chrono DateTime to lbug::Value::Timestamp via time::OffsetDateTime.
fn chrono_to_lbug_timestamp(
    dt: &chrono::DateTime<chrono::FixedOffset>,
) -> Result<lbug::Value, GraphdError> {
    use chrono::Timelike;
    let ts = time::OffsetDateTime::from_unix_timestamp(dt.timestamp())
        .map_err(|e| GraphdError::TypeError(format!("{e}")))?
        .replace_nanosecond(dt.nanosecond())
        .map_err(|e| GraphdError::TypeError(format!("{e}")))?;
    let offset = time::UtcOffset::from_whole_seconds(dt.offset().local_minus_utc())
        .map_err(|e| GraphdError::TypeError(format!("{e}")))?;
    Ok(lbug::Value::Timestamp(ts.to_offset(offset)))
}

/// Convert a time::Duration to an ISO 8601 duration string (e.g. "P14DT16H12M").
fn time_duration_to_iso8601(d: &time::Duration) -> String {
    let total_secs = d.whole_seconds().unsigned_abs();
    let negative = d.whole_seconds() < 0;

    let days = total_secs / 86400;
    let remainder = total_secs % 86400;
    let hours = remainder / 3600;
    let remainder = remainder % 3600;
    let minutes = remainder / 60;
    let seconds = remainder % 60;
    let nanos = d.subsec_nanoseconds().unsigned_abs() as u64;

    let mut s = if negative {
        "-P".to_string()
    } else {
        "P".to_string()
    };
    if days > 0 {
        s.push_str(&format!("{days}D"));
    }
    if hours > 0 || minutes > 0 || seconds > 0 || nanos > 0 {
        s.push('T');
        if hours > 0 {
            s.push_str(&format!("{hours}H"));
        }
        if minutes > 0 {
            s.push_str(&format!("{minutes}M"));
        }
        if seconds > 0 || nanos > 0 {
            if nanos > 0 {
                let frac = nanos as f64 / 1_000_000_000.0;
                let sec_f = seconds as f64 + frac;
                s.push_str(&format!("{sec_f}S"));
            } else {
                s.push_str(&format!("{seconds}S"));
            }
        }
    }
    if s == "P" || s == "-P" {
        s.push_str("T0S");
    }
    s
}

/// Parse an ISO 8601 duration string (e.g. "P14DT16H12M") into a time::Duration.
/// Supports P[nD][T[nH][nM][n.nS]]. Does not support year/month components.
fn parse_iso8601_duration(s: &str) -> Result<time::Duration, GraphdError> {
    let input = s.trim();
    let (negative, input) = if let Some(rest) = input.strip_prefix('-') {
        (true, rest)
    } else {
        (false, input)
    };
    let input = input.strip_prefix('P').ok_or_else(|| {
        GraphdError::TypeError(format!("Duration must start with 'P': {s}"))
    })?;

    let (date_part, time_part) = if let Some(t_idx) = input.find('T') {
        (&input[..t_idx], Some(&input[t_idx + 1..]))
    } else {
        (input, None)
    };

    let mut total_secs: i64 = 0;
    let mut nanos: i64 = 0;

    // Parse date part (only D supported)
    if !date_part.is_empty() {
        if let Some(d) = date_part.strip_suffix('D') {
            let days: i64 = d
                .parse()
                .map_err(|e| GraphdError::TypeError(format!("Invalid days in duration: {e}")))?;
            total_secs += days * 86400;
        } else {
            return Err(GraphdError::TypeError(format!(
                "Unsupported duration date component (only days supported): {s}"
            )));
        }
    }

    // Parse time part
    if let Some(time_str) = time_part {
        let mut rest = time_str;
        if let Some(h_idx) = rest.find('H') {
            let hours: i64 = rest[..h_idx]
                .parse()
                .map_err(|e| GraphdError::TypeError(format!("Invalid hours: {e}")))?;
            total_secs += hours * 3600;
            rest = &rest[h_idx + 1..];
        }
        if let Some(m_idx) = rest.find('M') {
            let minutes: i64 = rest[..m_idx]
                .parse()
                .map_err(|e| GraphdError::TypeError(format!("Invalid minutes: {e}")))?;
            total_secs += minutes * 60;
            rest = &rest[m_idx + 1..];
        }
        if let Some(s_idx) = rest.find('S') {
            let sec_str = &rest[..s_idx];
            if let Some(dot_idx) = sec_str.find('.') {
                let whole: i64 = sec_str[..dot_idx]
                    .parse()
                    .map_err(|e| GraphdError::TypeError(format!("Invalid seconds: {e}")))?;
                total_secs += whole;
                let frac_str = &sec_str[dot_idx + 1..];
                let padded = format!("{:0<9}", frac_str);
                nanos = padded[..9]
                    .parse()
                    .map_err(|e| GraphdError::TypeError(format!("Invalid fractional seconds: {e}")))?;
            } else {
                let secs: i64 = sec_str
                    .parse()
                    .map_err(|e| GraphdError::TypeError(format!("Invalid seconds: {e}")))?;
                total_secs += secs;
            }
        }
    }

    let mut dur = time::Duration::new(total_secs, nanos as i32);
    if negative {
        dur = -dur;
    }
    Ok(dur)
}

/// Serialize a GraphValue to Neo4j Query API v2 typed JSON format.
///
/// Every value gets a `{"$type": "TypeName", "_value": ...}` wrapper.
/// See: <https://neo4j.com/docs/query-api/current/typed-json/>
pub fn graph_value_to_typed_json(gv: &GraphValue) -> serde_json::Value {
    match gv {
        GraphValue::Null => serde_json::json!(null),
        GraphValue::Bool(b) => serde_json::json!({"$type": "Boolean", "_value": b}),
        GraphValue::Int(i) => serde_json::json!({"$type": "Integer", "_value": i.to_string()}),
        GraphValue::Float(f) => serde_json::json!({"$type": "Float", "_value": f.to_string()}),
        GraphValue::String(s) => serde_json::json!({"$type": "String", "_value": s}),
        GraphValue::Timestamp(dt) => {
            serde_json::json!({"$type": "OffsetDateTime", "_value": dt.to_rfc3339()})
        }
        GraphValue::Date(s) => serde_json::json!({"$type": "Date", "_value": s}),
        GraphValue::Base64(s) => serde_json::json!({"$type": "Base64", "_value": s}),
        GraphValue::Duration(s) => serde_json::json!({"$type": "Duration", "_value": s}),
        GraphValue::List(items) => serde_json::json!({
            "$type": "List",
            "_value": items.iter().map(graph_value_to_typed_json).collect::<Vec<_>>()
        }),
        GraphValue::Map(m) => {
            let obj: serde_json::Map<std::string::String, serde_json::Value> = m
                .iter()
                .map(|(k, v)| (k.clone(), graph_value_to_typed_json(v)))
                .collect();
            serde_json::json!({"$type": "Map", "_value": obj})
        }
        GraphValue::Tagged(TaggedValue::Node {
            id,
            label,
            properties,
        }) => {
            let props: serde_json::Map<std::string::String, serde_json::Value> = properties
                .iter()
                .map(|(k, v)| (k.clone(), graph_value_to_typed_json(v)))
                .collect();
            serde_json::json!({
                "$type": "Node",
                "_value": {
                    "_element_id": format!("{}:{}", id.table, id.offset),
                    "_labels": [label],
                    "_properties": props
                }
            })
        }
        GraphValue::Tagged(TaggedValue::Rel {
            id,
            label,
            src,
            dst,
            properties,
        }) => {
            let props: serde_json::Map<std::string::String, serde_json::Value> = properties
                .iter()
                .map(|(k, v)| (k.clone(), graph_value_to_typed_json(v)))
                .collect();
            serde_json::json!({
                "$type": "Relationship",
                "_value": {
                    "_element_id": format!("{}:{}", id.table, id.offset),
                    "_start_node_element_id": format!("{}:{}", src.table, src.offset),
                    "_end_node_element_id": format!("{}:{}", dst.table, dst.offset),
                    "_type": label,
                    "_properties": props
                }
            })
        }
        GraphValue::Tagged(TaggedValue::Path { nodes, rels }) => {
            let mut elements = Vec::new();
            for (i, node) in nodes.iter().enumerate() {
                elements.push(graph_value_to_typed_json(node));
                if let Some(rel) = rels.get(i) {
                    elements.push(graph_value_to_typed_json(rel));
                }
            }
            serde_json::json!({"$type": "Path", "_value": elements})
        }
        GraphValue::Tagged(TaggedValue::Union { value, .. }) => graph_value_to_typed_json(value),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scalar_serialization() {
        assert_eq!(serde_json::to_string(&GraphValue::Null).unwrap(), "null");
        assert_eq!(serde_json::to_string(&GraphValue::Bool(true)).unwrap(), "true");
        assert_eq!(serde_json::to_string(&GraphValue::Int(42)).unwrap(), "42");
        assert_eq!(serde_json::to_string(&GraphValue::Float(3.14)).unwrap(), "3.14");
        assert_eq!(
            serde_json::to_string(&GraphValue::String("hello".into())).unwrap(),
            "\"hello\""
        );
    }

    #[test]
    fn test_node_serialization() {
        let node = GraphValue::Tagged(TaggedValue::Node {
            id: InternalId { table: 0, offset: 5 },
            label: "Person".into(),
            properties: HashMap::from([
                ("name".into(), GraphValue::String("Alice".into())),
                ("age".into(), GraphValue::Int(30)),
            ]),
        });
        let json: serde_json::Value = serde_json::to_value(&node).unwrap();
        assert_eq!(json["$type"], "node");
        assert_eq!(json["label"], "Person");
        assert_eq!(json["id"]["table"], 0);
        assert_eq!(json["id"]["offset"], 5);
    }

    #[test]
    fn test_rel_serialization() {
        let rel = GraphValue::Tagged(TaggedValue::Rel {
            id: InternalId { table: 2, offset: 10 },
            label: "KNOWS".into(),
            src: InternalId { table: 0, offset: 5 },
            dst: InternalId { table: 0, offset: 8 },
            properties: HashMap::new(),
        });
        let json: serde_json::Value = serde_json::to_value(&rel).unwrap();
        assert_eq!(json["$type"], "rel");
        assert_eq!(json["label"], "KNOWS");
        assert_eq!(json["src"]["offset"], 5);
        assert_eq!(json["dst"]["offset"], 8);
    }

    #[test]
    fn test_list_serialization() {
        let list = GraphValue::List(vec![GraphValue::Int(1), GraphValue::Int(2)]);
        assert_eq!(serde_json::to_string(&list).unwrap(), "[1,2]");
    }

    #[test]
    fn test_to_lbug_null() {
        let v = to_lbug_value(&serde_json::Value::Null).unwrap();
        assert!(matches!(v, lbug::Value::Null(_)));
    }

    #[test]
    fn test_to_lbug_bool() {
        assert!(matches!(
            to_lbug_value(&serde_json::json!(true)).unwrap(),
            lbug::Value::Bool(true)
        ));
        assert!(matches!(
            to_lbug_value(&serde_json::json!(false)).unwrap(),
            lbug::Value::Bool(false)
        ));
    }

    #[test]
    fn test_to_lbug_int() {
        match to_lbug_value(&serde_json::json!(42)).unwrap() {
            lbug::Value::Int64(n) => assert_eq!(n, 42),
            other => panic!("expected Int64, got {other:?}"),
        }
    }

    #[test]
    fn test_to_lbug_float() {
        match to_lbug_value(&serde_json::json!(3.14)).unwrap() {
            lbug::Value::Double(f) => assert!((f - 3.14).abs() < f64::EPSILON),
            other => panic!("expected Double, got {other:?}"),
        }
    }

    #[test]
    fn test_to_lbug_string() {
        match to_lbug_value(&serde_json::json!("hello")).unwrap() {
            lbug::Value::String(s) => assert_eq!(s, "hello"),
            other => panic!("expected String, got {other:?}"),
        }
    }

    #[test]
    fn test_to_lbug_array() {
        match to_lbug_value(&serde_json::json!([1, 2, 3])).unwrap() {
            lbug::Value::List(ty, items) => {
                assert_eq!(ty, lbug::LogicalType::Int64);
                assert_eq!(items.len(), 3);
            }
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn test_to_lbug_float_array() {
        match to_lbug_value(&serde_json::json!([0.1, 0.2, 0.3])).unwrap() {
            lbug::Value::List(ty, items) => {
                assert_eq!(ty, lbug::LogicalType::Double);
                assert_eq!(items.len(), 3);
            }
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn test_to_lbug_string_array() {
        match to_lbug_value(&serde_json::json!(["a", "b"])).unwrap() {
            lbug::Value::List(ty, items) => {
                assert_eq!(ty, lbug::LogicalType::String);
                assert_eq!(items.len(), 2);
            }
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn test_to_lbug_empty_array() {
        match to_lbug_value(&serde_json::json!([])).unwrap() {
            lbug::Value::List(ty, items) => {
                assert_eq!(ty, lbug::LogicalType::Any);
                assert!(items.is_empty());
            }
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn test_to_lbug_plain_object_error() {
        assert!(to_lbug_value(&serde_json::json!({"a": 1})).is_err());
    }

    #[test]
    fn test_typed_json_integer() {
        match to_lbug_value(&serde_json::json!({"$type": "Integer", "_value": "42"})).unwrap() {
            lbug::Value::Int64(n) => assert_eq!(n, 42),
            other => panic!("expected Int64, got {other:?}"),
        }
    }

    #[test]
    fn test_typed_json_float() {
        match to_lbug_value(&serde_json::json!({"$type": "Float", "_value": "3.14"})).unwrap() {
            lbug::Value::Double(f) => assert!((f - 3.14).abs() < f64::EPSILON),
            other => panic!("expected Double, got {other:?}"),
        }
    }

    #[test]
    fn test_typed_json_string() {
        match to_lbug_value(&serde_json::json!({"$type": "String", "_value": "hello"})).unwrap() {
            lbug::Value::String(s) => assert_eq!(s, "hello"),
            other => panic!("expected String, got {other:?}"),
        }
    }

    #[test]
    fn test_typed_json_boolean() {
        match to_lbug_value(&serde_json::json!({"$type": "Boolean", "_value": true})).unwrap() {
            lbug::Value::Bool(b) => assert!(b),
            other => panic!("expected Bool, got {other:?}"),
        }
    }

    #[test]
    fn test_typed_json_null() {
        match to_lbug_value(&serde_json::json!({"$type": "Null", "_value": null})).unwrap() {
            lbug::Value::Null(_) => {}
            other => panic!("expected Null, got {other:?}"),
        }
    }

    #[test]
    fn test_typed_json_offset_datetime() {
        let val = to_lbug_value(
            &serde_json::json!({"$type": "OffsetDateTime", "_value": "2024-01-15T10:30:00Z"}),
        )
        .unwrap();
        match val {
            lbug::Value::Timestamp(ts) => {
                assert_eq!(ts.year(), 2024);
                assert_eq!(ts.month() as u8, 1);
                assert_eq!(ts.day(), 15);
                assert_eq!(ts.hour(), 10);
                assert_eq!(ts.minute(), 30);
            }
            other => panic!("expected Timestamp, got {other:?}"),
        }
    }

    #[test]
    fn test_typed_json_date() {
        let val =
            to_lbug_value(&serde_json::json!({"$type": "Date", "_value": "2024-06-15"})).unwrap();
        match val {
            lbug::Value::Date(d) => {
                assert_eq!(d.year(), 2024);
                assert_eq!(d.month() as u8, 6);
                assert_eq!(d.day(), 15);
            }
            other => panic!("expected Date, got {other:?}"),
        }
    }

    #[test]
    fn test_typed_json_unsupported_type() {
        assert!(
            to_lbug_value(&serde_json::json!({"$type": "Point", "_value": "SRID=7203;POINT(1 2)"}))
                .is_err()
        );
    }

    #[test]
    fn test_json_params_to_lbug() {
        let params = serde_json::json!({"name": "Alice", "age": 30});
        let result = json_params_to_lbug(&params).unwrap();
        assert_eq!(result.len(), 2);
        // Check both params exist (order not guaranteed from JSON object)
        let names: Vec<&str> = result.iter().map(|(k, _)| k.as_str()).collect();
        assert!(names.contains(&"name"));
        assert!(names.contains(&"age"));
    }

    #[test]
    fn test_json_params_not_object() {
        assert!(json_params_to_lbug(&serde_json::json!("not an object")).is_err());
    }

    #[test]
    fn test_graph_value_to_typed_json_scalars() {
        assert_eq!(graph_value_to_typed_json(&GraphValue::Null), serde_json::json!(null));
        assert_eq!(
            graph_value_to_typed_json(&GraphValue::Bool(true)),
            serde_json::json!({"$type": "Boolean", "_value": true})
        );
        assert_eq!(
            graph_value_to_typed_json(&GraphValue::Int(42)),
            serde_json::json!({"$type": "Integer", "_value": "42"})
        );
        assert_eq!(
            graph_value_to_typed_json(&GraphValue::String("hello".into())),
            serde_json::json!({"$type": "String", "_value": "hello"})
        );
    }

    #[test]
    fn test_graph_value_to_typed_json_timestamp() {
        let dt = chrono::DateTime::parse_from_rfc3339("2024-01-15T10:30:00Z").unwrap();
        let json = graph_value_to_typed_json(&GraphValue::Timestamp(dt));
        assert_eq!(json["$type"], "OffsetDateTime");
        assert!(json["_value"].as_str().unwrap().contains("2024-01-15"));
    }

    #[test]
    fn test_graph_value_to_typed_json_list() {
        let list = GraphValue::List(vec![GraphValue::Int(1), GraphValue::Int(2)]);
        let json = graph_value_to_typed_json(&list);
        assert_eq!(json["$type"], "List");
        assert_eq!(json["_value"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn test_graph_value_to_typed_json_node() {
        let node = GraphValue::Tagged(TaggedValue::Node {
            id: InternalId { table: 0, offset: 5 },
            label: "Person".into(),
            properties: HashMap::from([("name".into(), GraphValue::String("Alice".into()))]),
        });
        let json = graph_value_to_typed_json(&node);
        assert_eq!(json["$type"], "Node");
        assert_eq!(json["_value"]["_labels"][0], "Person");
        assert_eq!(json["_value"]["_properties"]["name"]["_value"], "Alice");
    }

    #[test]
    fn test_from_lbug_date_typed_json() {
        let date = time::Date::from_calendar_date(2024, time::Month::June, 15).unwrap();
        let gv = from_lbug_value(&lbug::Value::Date(date));
        assert!(matches!(&gv, GraphValue::Date(s) if s == "2024-06-15"));
        let json = graph_value_to_typed_json(&gv);
        assert_eq!(json["$type"], "Date");
        assert_eq!(json["_value"], "2024-06-15");
    }

    #[test]
    fn test_from_lbug_blob_typed_json() {
        let gv = from_lbug_value(&lbug::Value::Blob(vec![72, 101, 108, 108, 111]));
        assert!(matches!(&gv, GraphValue::Base64(s) if s == "SGVsbG8="));
        let json = graph_value_to_typed_json(&gv);
        assert_eq!(json["$type"], "Base64");
        assert_eq!(json["_value"], "SGVsbG8=");
    }

    #[test]
    fn test_from_lbug_interval_typed_json() {
        let d = time::Duration::new(14 * 86400 + 16 * 3600 + 12 * 60, 0);
        let gv = from_lbug_value(&lbug::Value::Interval(d));
        assert!(matches!(&gv, GraphValue::Duration(_)));
        let json = graph_value_to_typed_json(&gv);
        assert_eq!(json["$type"], "Duration");
        assert_eq!(json["_value"], "P14DT16H12M");
    }

    #[test]
    fn test_typed_json_duration() {
        let val =
            to_lbug_value(&serde_json::json!({"$type": "Duration", "_value": "P14DT16H12M"}))
                .unwrap();
        match val {
            lbug::Value::Interval(d) => {
                assert_eq!(d.whole_days(), 14);
                assert_eq!(d.whole_hours() % 24, 16);
                assert_eq!(d.whole_minutes() % 60, 12);
            }
            other => panic!("expected Interval, got {other:?}"),
        }
    }

    #[test]
    fn test_typed_json_base64() {
        let val =
            to_lbug_value(&serde_json::json!({"$type": "Base64", "_value": "SGVsbG8="})).unwrap();
        match val {
            lbug::Value::Blob(bytes) => assert_eq!(bytes, b"Hello"),
            other => panic!("expected Blob, got {other:?}"),
        }
    }

    #[test]
    fn test_typed_json_map() {
        let val = to_lbug_value(&serde_json::json!({
            "$type": "Map",
            "_value": {
                "name": {"$type": "String", "_value": "Alice"},
                "age": {"$type": "Integer", "_value": "30"}
            }
        }))
        .unwrap();
        match val {
            lbug::Value::Map(_, entries) => {
                assert_eq!(entries.len(), 2);
            }
            other => panic!("expected Map, got {other:?}"),
        }
    }

    #[test]
    fn test_duration_parse_zero() {
        let val =
            to_lbug_value(&serde_json::json!({"$type": "Duration", "_value": "PT0S"})).unwrap();
        match val {
            lbug::Value::Interval(d) => assert_eq!(d.whole_seconds(), 0),
            other => panic!("expected Interval, got {other:?}"),
        }
    }

    #[test]
    fn test_duration_parse_fractional_seconds() {
        let val =
            to_lbug_value(&serde_json::json!({"$type": "Duration", "_value": "PT1.5S"})).unwrap();
        match val {
            lbug::Value::Interval(d) => {
                assert_eq!(d.whole_seconds(), 1);
                assert_eq!(d.subsec_nanoseconds(), 500_000_000);
            }
            other => panic!("expected Interval, got {other:?}"),
        }
    }

    #[test]
    fn test_duration_roundtrip() {
        let d = time::Duration::new(3 * 86400 + 2 * 3600 + 30 * 60 + 15, 0);
        let iso = time_duration_to_iso8601(&d);
        assert_eq!(iso, "P3DT2H30M15S");
        let parsed = parse_iso8601_duration(&iso).unwrap();
        assert_eq!(parsed, d);
    }

    #[test]
    fn test_graph_value_to_typed_json_date() {
        let json = graph_value_to_typed_json(&GraphValue::Date("2024-06-15".into()));
        assert_eq!(json["$type"], "Date");
        assert_eq!(json["_value"], "2024-06-15");
    }

    #[test]
    fn test_graph_value_to_typed_json_base64() {
        let json = graph_value_to_typed_json(&GraphValue::Base64("SGVsbG8=".into()));
        assert_eq!(json["$type"], "Base64");
        assert_eq!(json["_value"], "SGVsbG8=");
    }

    #[test]
    fn test_graph_value_to_typed_json_duration() {
        let json = graph_value_to_typed_json(&GraphValue::Duration("P14DT16H12M".into()));
        assert_eq!(json["$type"], "Duration");
        assert_eq!(json["_value"], "P14DT16H12M");
    }
}
