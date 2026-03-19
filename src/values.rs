use base64::Engine;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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

// ─── ParamValue: typed query parameters (no JSON dependency) ───

/// Strongly-typed parameter value for journal entries and query binding.
/// Maps directly to protobuf GraphValue and lbug::Value — no JSON intermediary.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    List(Vec<ParamValue>),
}

/// Convert a JSON value to a ParamValue (boundary conversion at journal entry).
pub fn json_to_param_value(v: &serde_json::Value) -> ParamValue {
    match v {
        serde_json::Value::Null => ParamValue::Null,
        serde_json::Value::Bool(b) => ParamValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                ParamValue::Int(i)
            } else if let Some(u) = n.as_u64() {
                // u64 > i64::MAX: store as string to preserve exact value.
                ParamValue::String(u.to_string())
            } else {
                ParamValue::Float(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => ParamValue::String(s.clone()),
        serde_json::Value::Array(arr) => {
            ParamValue::List(arr.iter().map(json_to_param_value).collect())
        }
        serde_json::Value::Object(_) => {
            // Objects not supported as params — store as JSON string as fallback.
            ParamValue::String(v.to_string())
        }
    }
}

/// Convert JSON params object to typed param pairs (for journal boundary).
pub fn json_to_param_values(params: &Option<serde_json::Value>) -> Vec<(String, ParamValue)> {
    match params {
        Some(serde_json::Value::Object(map)) => map
            .iter()
            .map(|(k, v)| (k.clone(), json_to_param_value(v)))
            .collect(),
        _ => vec![],
    }
}

/// Convert a ParamValue to a lbug::Value for query execution.
pub fn param_value_to_lbug(v: &ParamValue) -> Result<lbug::Value, String> {
    match v {
        ParamValue::Null => Ok(lbug::Value::Null(lbug::LogicalType::Any)),
        ParamValue::Bool(b) => Ok(lbug::Value::Bool(*b)),
        ParamValue::Int(i) => Ok(lbug::Value::Int64(*i)),
        ParamValue::Float(f) => Ok(lbug::Value::Double(*f)),
        ParamValue::String(s) => Ok(lbug::Value::String(s.clone())),
        ParamValue::List(items) => {
            let converted: Result<Vec<lbug::Value>, String> =
                items.iter().map(param_value_to_lbug).collect();
            let converted = converted?;
            let elem_type = converted
                .first()
                .map(lbug_value_logical_type)
                .unwrap_or(lbug::LogicalType::Any);
            Ok(lbug::Value::List(elem_type, converted))
        }
    }
}

/// Convert typed param pairs to lbug values for query execution.
pub fn param_values_to_lbug(
    params: &[(String, ParamValue)],
) -> Result<Vec<(String, lbug::Value)>, String> {
    params
        .iter()
        .map(|(k, v)| Ok((k.clone(), param_value_to_lbug(v)?)))
        .collect()
}

/// Infer the logical type from a lbug Value (for list element type).
fn lbug_value_logical_type(v: &lbug::Value) -> lbug::LogicalType {
    match v {
        lbug::Value::Bool(_) => lbug::LogicalType::Bool,
        lbug::Value::Int64(_) => lbug::LogicalType::Int64,
        lbug::Value::UInt64(_) => lbug::LogicalType::UInt64,
        lbug::Value::Double(_) => lbug::LogicalType::Double,
        lbug::Value::Float(_) => lbug::LogicalType::Float,
        lbug::Value::String(_) => lbug::LogicalType::String,
        _ => lbug::LogicalType::Any,
    }
}

/// Convert a JSON value to a LadybugDB value for parameter binding.
pub fn to_lbug_value(json: &serde_json::Value) -> Result<lbug::Value, String> {
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
                Err("Unsupported number type".into())
            }
        }
        serde_json::Value::String(s) => Ok(lbug::Value::String(s.clone())),
        serde_json::Value::Array(arr) => {
            let items: Result<Vec<lbug::Value>, String> =
                arr.iter().map(to_lbug_value).collect();
            let items = items?;
            // Infer element type from first item, default to Any for empty lists.
            let elem_type = items
                .first()
                .map(lbug_value_logical_type)
                .unwrap_or(lbug::LogicalType::Any);
            Ok(lbug::Value::List(elem_type, items))
        }
        serde_json::Value::Object(_) => {
            Err("Objects not supported as query parameters".into())
        }
    }
}

/// Convert a JSON params object to a Vec of (name, lbug::Value) pairs.
pub fn json_params_to_lbug(
    params: &serde_json::Value,
) -> Result<Vec<(String, lbug::Value)>, String> {
    let obj = params
        .as_object()
        .ok_or_else(|| "params must be a JSON object".to_string())?;
    obj.iter()
        .map(|(k, v)| Ok((k.clone(), to_lbug_value(v)?)))
        .collect()
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
    fn test_to_lbug_object_error() {
        assert!(to_lbug_value(&serde_json::json!({"a": 1})).is_err());
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

    // ─── ParamValue tests ───

    #[test]
    fn test_json_to_param_value_scalars() {
        assert_eq!(json_to_param_value(&serde_json::Value::Null), ParamValue::Null);
        assert_eq!(json_to_param_value(&serde_json::json!(true)), ParamValue::Bool(true));
        assert_eq!(json_to_param_value(&serde_json::json!(42)), ParamValue::Int(42));
        assert_eq!(json_to_param_value(&serde_json::json!(-1)), ParamValue::Int(-1));
        assert_eq!(json_to_param_value(&serde_json::json!(3.14)), ParamValue::Float(3.14));
        assert_eq!(
            json_to_param_value(&serde_json::json!("hello")),
            ParamValue::String("hello".into())
        );
    }

    #[test]
    fn test_json_to_param_value_u64_overflow() {
        // u64 > i64::MAX stored as string to preserve precision.
        let big = u64::MAX;
        let json = serde_json::json!(big);
        assert_eq!(
            json_to_param_value(&json),
            ParamValue::String(big.to_string())
        );
    }

    #[test]
    fn test_json_to_param_value_list() {
        let json = serde_json::json!([1, 2, 3]);
        assert_eq!(
            json_to_param_value(&json),
            ParamValue::List(vec![ParamValue::Int(1), ParamValue::Int(2), ParamValue::Int(3)])
        );
    }

    #[test]
    fn test_json_to_param_value_object_fallback() {
        // Objects are serialized to JSON string as fallback.
        let json = serde_json::json!({"a": 1});
        match json_to_param_value(&json) {
            ParamValue::String(s) => assert!(s.contains("\"a\""), "expected JSON string, got {s}"),
            other => panic!("expected String fallback, got {other:?}"),
        }
    }

    #[test]
    fn test_json_to_param_values_from_object() {
        let params = Some(serde_json::json!({"name": "Alice", "age": 30}));
        let result = json_to_param_values(&params);
        assert_eq!(result.len(), 2);
        let find = |k: &str| result.iter().find(|(key, _)| key == k).unwrap().1.clone();
        assert_eq!(find("name"), ParamValue::String("Alice".into()));
        assert_eq!(find("age"), ParamValue::Int(30));
    }

    #[test]
    fn test_json_to_param_values_none() {
        assert!(json_to_param_values(&None).is_empty());
    }

    #[test]
    fn test_json_to_param_values_non_object() {
        assert!(json_to_param_values(&Some(serde_json::json!("string"))).is_empty());
        assert!(json_to_param_values(&Some(serde_json::json!(42))).is_empty());
    }

    #[test]
    fn test_param_value_to_lbug_scalars() {
        assert!(matches!(param_value_to_lbug(&ParamValue::Null).unwrap(), lbug::Value::Null(_)));
        assert!(matches!(param_value_to_lbug(&ParamValue::Bool(true)).unwrap(), lbug::Value::Bool(true)));
        match param_value_to_lbug(&ParamValue::Int(42)).unwrap() {
            lbug::Value::Int64(n) => assert_eq!(n, 42),
            other => panic!("expected Int64, got {other:?}"),
        }
        match param_value_to_lbug(&ParamValue::Float(3.14)).unwrap() {
            lbug::Value::Double(f) => assert!((f - 3.14).abs() < f64::EPSILON),
            other => panic!("expected Double, got {other:?}"),
        }
        match param_value_to_lbug(&ParamValue::String("hi".into())).unwrap() {
            lbug::Value::String(s) => assert_eq!(s, "hi"),
            other => panic!("expected String, got {other:?}"),
        }
    }

    #[test]
    fn test_param_value_to_lbug_list() {
        let pv = ParamValue::List(vec![ParamValue::Int(1), ParamValue::Int(2)]);
        match param_value_to_lbug(&pv).unwrap() {
            lbug::Value::List(ty, items) => {
                assert_eq!(ty, lbug::LogicalType::Int64);
                assert_eq!(items.len(), 2);
            }
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn test_param_value_to_lbug_empty_list() {
        let pv = ParamValue::List(vec![]);
        match param_value_to_lbug(&pv).unwrap() {
            lbug::Value::List(ty, items) => {
                assert_eq!(ty, lbug::LogicalType::Any);
                assert!(items.is_empty());
            }
            other => panic!("expected List, got {other:?}"),
        }
    }

    #[test]
    fn test_param_values_to_lbug_multiple() {
        let params = vec![
            ("name".into(), ParamValue::String("Alice".into())),
            ("age".into(), ParamValue::Int(30)),
            ("active".into(), ParamValue::Bool(true)),
        ];
        let result = param_values_to_lbug(&params).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].0, "name");
        assert!(matches!(result[0].1, lbug::Value::String(_)));
        assert_eq!(result[1].0, "age");
        assert!(matches!(result[1].1, lbug::Value::Int64(30)));
        assert_eq!(result[2].0, "active");
        assert!(matches!(result[2].1, lbug::Value::Bool(true)));
    }

    #[test]
    fn test_param_values_to_lbug_empty() {
        let result = param_values_to_lbug(&[]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_json_to_param_to_lbug_roundtrip() {
        // Full pipeline: JSON → ParamValue → lbug::Value
        let json = serde_json::json!({"x": 42, "y": "hello", "z": true, "w": [1, 2]});
        let pv = json_to_param_values(&Some(json));
        let lbug_vals = param_values_to_lbug(&pv).unwrap();
        assert_eq!(lbug_vals.len(), 4);
        let find = |k: &str| lbug_vals.iter().find(|(key, _)| key == k).unwrap();
        assert!(matches!(find("x").1, lbug::Value::Int64(42)));
        match &find("y").1 {
            lbug::Value::String(s) => assert_eq!(s, "hello"),
            other => panic!("expected String, got {other:?}"),
        }
        assert!(matches!(find("z").1, lbug::Value::Bool(true)));
        assert!(matches!(find("w").1, lbug::Value::List(_, _)));
    }
}
