//! Optimistic locking over HTTP: the version a client edits comes back in
//! `If-Match`, and every response carrying a versioned resource names its
//! version in `ETag`.

use axum::http::{HeaderMap, HeaderValue, header::IF_MATCH};
use domain::broadcast::Version;

use crate::error::ApiError;

/// Accepts only a strong tag such as `"3"`: RFC 9110 compares `If-Match`
/// strongly, so a weak tag can never match.
pub(crate) fn expected_version(headers: &HeaderMap) -> Result<Version, ApiError> {
    let raw = headers
        .get(IF_MATCH)
        .ok_or_else(ApiError::if_match_required)?;
    let tag = raw
        .to_str()
        .map_err(|_| ApiError::invalid_if_match())?
        .trim();
    tag.strip_prefix('"')
        .and_then(|tag| tag.strip_suffix('"'))
        .filter(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|digits| digits.parse().ok())
        .and_then(Version::new)
        .ok_or_else(ApiError::invalid_if_match)
}

pub(crate) fn etag(version: Version) -> HeaderValue {
    HeaderValue::try_from(format!("\"{}\"", version.get())).expect("digits are a valid header")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn if_match(value: Option<&str>) -> Result<i32, &'static str> {
        let mut headers = HeaderMap::new();
        if let Some(value) = value {
            headers.insert(IF_MATCH, HeaderValue::from_str(value).unwrap());
        }
        expected_version(&headers)
            .map(Version::get)
            .map_err(|err| err.code())
    }

    #[test]
    fn if_match_takes_a_strong_version_tag() {
        assert_eq!(if_match(Some("\"3\"")), Ok(3));
        assert_eq!(if_match(Some("\"12\"")), Ok(12));
        assert_eq!(if_match(Some(" \"1\" ")), Ok(1));
    }

    #[test]
    fn if_match_is_required_and_must_be_a_positive_version() {
        assert_eq!(if_match(None), Err("IF_MATCH_REQUIRED"));
        for bad in [
            "3",
            "\"\"",
            "\"0\"",
            "\"-1\"",
            "\"+1\"",
            "\"1.0\"",
            "*",
            "\"1\", \"2\"",
            "W/\"1\"",
            "w/\"1\"",
            "\"99999999999\"",
        ] {
            assert_eq!(if_match(Some(bad)), Err("INVALID_IF_MATCH"), "{bad}");
        }
    }

    #[test]
    fn etag_round_trips_through_if_match() {
        let tag = etag(Version::new(42).unwrap());
        assert_eq!(tag, "\"42\"");
        assert_eq!(if_match(Some(tag.to_str().unwrap())), Ok(42));
    }
}
