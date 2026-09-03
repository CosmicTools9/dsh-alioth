//! Serde helpers for ZUID (64-bit integer IDs)
//!
//! Serializes `i64` values as JSON strings to avoid JavaScript number precision loss.
//! Deserializes from either JSON strings or numbers for backward compatibility.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Serialize an `i64` as a JSON string.
pub fn serialize<S>(value: &i64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

/// Deserialize an `i64` from either a JSON string or number.
pub fn deserialize<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(s) => s.parse().map_err(serde::de::Error::custom),
        serde_json::Value::Number(n) => n
            .as_i64()
            .ok_or_else(|| serde::de::Error::custom("invalid zuid number")),
        _ => Err(serde::de::Error::custom("zuid must be a string or number")),
    }
}

/// Serialize an `Option<i64>` as a JSON string or null.
pub fn serialize_opt<S>(value: &Option<i64>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(v) => serialize(v, serializer),
        None => serializer.serialize_none(),
    }
}

/// Deserialize an `Option<i64>` from either a JSON string, number, or null.
pub fn deserialize_opt<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        Some(serde_json::Value::String(s)) => s.parse().map(Some).map_err(serde::de::Error::custom),
        Some(serde_json::Value::Number(n)) => n
            .as_i64()
            .map(Some)
            .ok_or_else(|| serde::de::Error::custom("invalid zuid number")),
        Some(_) => Err(serde::de::Error::custom("zuid must be a string or number")),
        None => Ok(None),
    }
}

/// Submodule usable with `#[serde(with = "common::serde_zuid::opt")]` on
/// `Option<i64>` fields (serde requires a module exposing `serialize`/`deserialize`).
pub mod opt {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(value: &Option<i64>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        super::serialize_opt(value, serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<i64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        super::deserialize_opt(deserializer)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// 集合变体：Vec<i64> / Option<Vec<i64>>
// 用于 ID 列表字段（如 NGAC 的 ancestor_ids / children_ids / ak_access_rights，
// 以及各模块的 ak_*_user / ak_source 权限 ID 列表）。元素同样超出 JS 2^53，
// 必须逐元素字符串化。
// ═══════════════════════════════════════════════════════════════════════════════

/// Serialize a `Vec<i64>` as a JSON array of strings (ZUID-safe).
pub fn serialize_seq<S>(value: &[i64], serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    let strs: Vec<String> = value.iter().map(|v| v.to_string()).collect();
    strs.serialize(serializer)
}

/// Deserialize a `Vec<i64>` from a JSON array of strings or numbers.
pub fn deserialize_seq<'de, D>(deserializer: D) -> Result<Vec<i64>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw: Vec<serde_json::Value> = Vec::deserialize(deserializer)?;
    raw.into_iter()
        .map(|v| match v {
            serde_json::Value::String(s) => s.parse().map_err(serde::de::Error::custom),
            serde_json::Value::Number(n) => n
                .as_i64()
                .ok_or_else(|| serde::de::Error::custom("invalid zuid number")),
            _ => Err(serde::de::Error::custom("zuid must be a string or number")),
        })
        .collect()
}

/// Submodule for `#[serde(with = "common::serde_zuid::seq")]` on `Vec<i64>` fields.
pub mod seq {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(value: &[i64], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        super::serialize_seq(value, serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<i64>, D::Error>
    where
        D: Deserializer<'de>,
    {
        super::deserialize_seq(deserializer)
    }
}

/// Serialize an `Option<Vec<i64>>` as a JSON array of strings or null.
pub fn serialize_opt_seq<S>(value: &Option<Vec<i64>>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match value {
        Some(v) => serialize_seq(v, serializer),
        None => serializer.serialize_none(),
    }
}

/// Deserialize an `Option<Vec<i64>>` from a JSON array of strings/numbers or null.
pub fn deserialize_opt_seq<'de, D>(deserializer: D) -> Result<Option<Vec<i64>>, D::Error>
where
    D: Deserializer<'de>,
{
    let raw = Option::<Vec<serde_json::Value>>::deserialize(deserializer)?;
    match raw {
        Some(items) => {
            let parsed = items
                .into_iter()
                .map(|v| match v {
                    serde_json::Value::String(s) => s.parse().map_err(serde::de::Error::custom),
                    serde_json::Value::Number(n) => n
                        .as_i64()
                        .ok_or_else(|| serde::de::Error::custom("invalid zuid number")),
                    _ => Err(serde::de::Error::custom("zuid must be a string or number")),
                })
                .collect::<Result<Vec<i64>, D::Error>>()?;
            Ok(Some(parsed))
        }
        None => Ok(None),
    }
}

/// Submodule for `#[serde(with = "common::serde_zuid::opt_seq")]` on
/// `Option<Vec<i64>>` fields.
pub mod opt_seq {
    use serde::{Deserializer, Serializer};

    pub fn serialize<S>(value: &Option<Vec<i64>>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        super::serialize_opt_seq(value, serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Vec<i64>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        super::deserialize_opt_seq(deserializer)
    }
}

// ═══════════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use serde_json;

    /// Wrapper struct for testing the serde helpers via `#[serde(with = …)]`.
    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    #[serde(transparent)]
    struct Zuid(#[serde(with = "super")] i64);

    #[test]
    fn test_serialize_produces_string() {
        let zuid = Zuid(1234567890123456789i64);
        let json = serde_json::to_string(&zuid).unwrap();
        assert_eq!(json, r#""1234567890123456789""#);
    }

    #[test]
    fn test_deserialize_from_string() {
        let zuid: Zuid = serde_json::from_str(r#""42""#).unwrap();
        assert_eq!(zuid, Zuid(42));
    }

    #[test]
    fn test_deserialize_from_number() {
        let zuid: Zuid = serde_json::from_str(r#"42"#).unwrap();
        assert_eq!(zuid, Zuid(42));
    }

    #[test]
    fn test_deserialize_large_number() {
        let zuid: Zuid = serde_json::from_str(r#""1234567890123456789""#).unwrap();
        assert_eq!(zuid, Zuid(1234567890123456789i64));
    }

    #[test]
    fn test_round_trip() {
        let original = Zuid(-9223372036854775808i64); // i64::MIN
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Zuid = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_round_trip_positive() {
        let original = Zuid(9223372036854775807i64); // i64::MAX
        let json = serde_json::to_string(&original).unwrap();
        let deserialized: Zuid = serde_json::from_str(&json).unwrap();
        assert_eq!(original, deserialized);
    }

    #[test]
    fn test_deserialize_from_negative_number() {
        let zuid: Zuid = serde_json::from_str(r#"-1"#).unwrap();
        assert_eq!(zuid, Zuid(-1));
    }

    #[test]
    fn test_deserialize_invalid_type() {
        let result: Result<Zuid, _> = serde_json::from_str(r#"true"#);
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_invalid_string() {
        let result: Result<Zuid, _> = serde_json::from_str(r#""not-a-number""#);
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_float_rejected() {
        let result: Result<Zuid, _> = serde_json::from_str(r#"3.14"#);
        assert!(result.is_err());
    }

    #[test]
    fn test_deserialize_zero() {
        let zuid: Zuid = serde_json::from_str(r#"0"#).unwrap();
        assert_eq!(zuid, Zuid(0));
    }

    #[test]
    fn test_deserialize_zero_string() {
        let zuid: Zuid = serde_json::from_str(r#""0""#).unwrap();
        assert_eq!(zuid, Zuid(0));
    }

    // ── 集合变体 ──

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct ZuidSeq {
        #[serde(with = "super::seq")]
        ids: Vec<i64>,
    }

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct ZuidOptSeq {
        #[serde(with = "super::opt_seq")]
        ids: Option<Vec<i64>>,
    }

    #[test]
    fn test_seq_serializes_strings() {
        let v = ZuidSeq {
            ids: vec![1234567890123456789i64, -42, 0],
        };
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"ids":["1234567890123456789","-42","0"]}"#);
    }

    #[test]
    fn test_seq_deserialize_from_strings() {
        let v: ZuidSeq = serde_json::from_str(r#"{"ids":["1234567890123456789","-42"]}"#).unwrap();
        assert_eq!(
            v,
            ZuidSeq {
                ids: vec![1234567890123456789i64, -42]
            }
        );
    }

    #[test]
    fn test_seq_deserialize_from_numbers_backward_compat() {
        let v: ZuidSeq = serde_json::from_str(r#"{"ids":[1,2,3]}"#).unwrap();
        assert_eq!(v, ZuidSeq { ids: vec![1, 2, 3] });
    }

    #[test]
    fn test_seq_round_trip_large() {
        let v = ZuidSeq {
            ids: vec![i64::MIN, i64::MAX],
        };
        let json = serde_json::to_string(&v).unwrap();
        let back: ZuidSeq = serde_json::from_str(&json).unwrap();
        assert_eq!(v, back);
    }

    #[test]
    fn test_opt_seq_some_serializes_strings() {
        let v = ZuidOptSeq {
            ids: Some(vec![999]),
        };
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"ids":["999"]}"#);
    }

    #[test]
    fn test_opt_seq_none_serializes_null() {
        let v = ZuidOptSeq { ids: None };
        let json = serde_json::to_string(&v).unwrap();
        assert_eq!(json, r#"{"ids":null}"#);
    }

    #[test]
    fn test_opt_seq_round_trip() {
        for ids in [None, Some(vec![1i64, 2, 3])] {
            let v = ZuidOptSeq { ids: ids.clone() };
            let json = serde_json::to_string(&v).unwrap();
            let back: ZuidOptSeq = serde_json::from_str(&json).unwrap();
            assert_eq!(v, back);
        }
    }
}
