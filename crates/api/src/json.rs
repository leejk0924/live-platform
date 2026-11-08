//! JSON encodings shared by DTOs.

/// Serializes a [`SnowflakeId`](domain::id::SnowflakeId) as a decimal string,
/// because JavaScript numbers lose precision above 2^53.
/// Use with `#[serde(with = "crate::json::snowflake_id")]`.
pub mod snowflake_id {
    use domain::id::SnowflakeId;
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    pub fn serialize<S: Serializer>(id: &SnowflakeId, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(id)
    }

    /// Accepts only a string of decimal digits; a JSON number is rejected.
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<SnowflakeId, D::Error> {
        let raw = String::deserialize(deserializer)?;
        raw.parse().map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use domain::id::SnowflakeId;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Dto {
        #[serde(with = "crate::json::snowflake_id")]
        id: SnowflakeId,
    }

    fn dto(raw: i64) -> Dto {
        Dto {
            id: SnowflakeId::try_from(raw).unwrap(),
        }
    }

    #[test]
    fn round_trips_as_a_string() {
        let json = serde_json::to_string(&dto(42)).unwrap();
        assert_eq!(json, r#"{"id":"42"}"#);
        assert_eq!(serde_json::from_str::<Dto>(&json).unwrap(), dto(42));
    }

    #[test]
    fn values_above_2_pow_53_stay_exact() {
        let raw = (1_i64 << 53) + 1;
        let json = serde_json::to_string(&dto(raw)).unwrap();
        assert_eq!(json, r#"{"id":"9007199254740993"}"#);
        assert_eq!(serde_json::from_str::<Dto>(&json).unwrap().id.as_i64(), raw);
    }

    #[test]
    fn rejects_numbers_and_non_decimal_strings() {
        for bad in [
            r#"{"id":42}"#,
            r#"{"id":"-1"}"#,
            r#"{"id":"4x"}"#,
            r#"{"id":""}"#,
        ] {
            assert!(
                serde_json::from_str::<Dto>(bad).is_err(),
                "{bad} should be rejected"
            );
        }
    }
}
