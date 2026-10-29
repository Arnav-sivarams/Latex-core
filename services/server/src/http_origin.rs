//! Browser origins follow the request authority; canonical links follow configuration.
use axum::http::{HeaderMap, Uri, header, uri::Authority};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub(crate) enum OriginError {
    #[error("invalid HTTP origin")]
    Invalid,
    #[error("cross-origin request denied")]
    CrossOrigin,
}

fn authority(value: &str, scheme: &str) -> Result<(String, u16), OriginError> {
    let parsed: Authority = value.parse().map_err(|_| OriginError::Invalid)?;
    if parsed.as_str().contains('@') || parsed.host().is_empty() {
        return Err(OriginError::Invalid);
    }
    // Authority::port() returns None for an out-of-range port as well as a missing port.
    let suffix = parsed
        .as_str()
        .strip_prefix(parsed.host())
        .ok_or(OriginError::Invalid)?;
    let port = if suffix.is_empty() {
        if scheme == "https" { 443 } else { 80 }
    } else {
        let digits = suffix.strip_prefix(':').ok_or(OriginError::Invalid)?;
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(OriginError::Invalid);
        }
        digits.parse::<u16>().map_err(|_| OriginError::Invalid)?
    };
    Ok((parsed.host().to_ascii_lowercase(), port))
}

pub(crate) fn public_base_url(value: &str) -> Result<String, OriginError> {
    // http::Uri discards fragments; reject them before parsing the origin.
    if value.contains('#') {
        return Err(OriginError::Invalid);
    }
    let uri: Uri = value.parse().map_err(|_| OriginError::Invalid)?;
    let scheme = uri.scheme_str().ok_or(OriginError::Invalid)?;
    if !matches!(scheme, "http" | "https") || uri.path() != "/" || uri.query().is_some() {
        return Err(OriginError::Invalid);
    }
    let raw_authority = uri.authority().ok_or(OriginError::Invalid)?;
    let (host, port) = authority(raw_authority.as_str(), scheme)?;
    let default_port = if scheme == "https" { 443 } else { 80 };
    Ok(if port == default_port {
        format!("{scheme}://{host}")
    } else {
        format!("{scheme}://{host}:{port}")
    })
}

pub(crate) fn validate_request_origin(headers: &HeaderMap) -> Result<(), OriginError> {
    let mut origins = headers.get_all(header::ORIGIN).iter();
    let Some(raw_origin) = origins.next() else {
        return Ok(());
    };
    if origins.next().is_some() {
        return Err(OriginError::Invalid);
    }
    let origin = raw_origin.to_str().map_err(|_| OriginError::Invalid)?;
    let normalized = public_base_url(origin)?;
    // Origin is serialized without a path. Canonical URL configuration may have a trailing slash.
    if origin.ends_with('/') {
        return Err(OriginError::Invalid);
    }
    let scheme = normalized.split_once("://").ok_or(OriginError::Invalid)?.0;
    let mut hosts = headers.get_all(header::HOST).iter();
    let raw_host = hosts
        .next()
        .ok_or(OriginError::Invalid)?
        .to_str()
        .map_err(|_| OriginError::Invalid)?;
    if hosts.next().is_some() {
        return Err(OriginError::Invalid);
    }
    let (host, port) = authority(raw_host, scheme)?;
    let parsed: Uri = normalized.parse().map_err(|_| OriginError::Invalid)?;
    let expected = authority(
        parsed.authority().ok_or(OriginError::Invalid)?.as_str(),
        scheme,
    )?;
    if (host, port) == expected {
        Ok(())
    } else {
        Err(OriginError::CrossOrigin)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn request_authority_supports_changing_lan_addresses_without_a_host_allowlist() {
        for host in [
            "localhost:9000".to_owned(),
            "127.0.0.1:9000".to_owned(),
            "papers.example.edu".to_owned(),
            "[fd00::42]:9000".to_owned(),
        ]
        .into_iter()
        .chain((1..5).map(|index| format!("10.42.{index}.87:9000")))
        {
            for scheme in ["http", "https"] {
                let mut headers = HeaderMap::new();
                headers.insert(header::HOST, host.parse().expect("valid test origin"));
                headers.insert(
                    header::ORIGIN,
                    format!("{scheme}://{host}")
                        .parse()
                        .expect("valid test origin"),
                );
                assert_eq!(validate_request_origin(&headers), Ok(()));
                headers.insert(
                    "x-forwarded-host",
                    "untrusted.example".parse().expect("valid test origin"),
                );
                headers.insert(
                    header::ORIGIN,
                    "https://untrusted.example"
                        .parse()
                        .expect("valid test origin"),
                );
                assert_eq!(
                    validate_request_origin(&headers),
                    Err(OriginError::CrossOrigin)
                );
                headers.insert(
                    header::ORIGIN,
                    format!("{scheme}://{host}.untrusted.example")
                        .parse()
                        .expect("valid test origin"),
                );
                assert!(validate_request_origin(&headers).is_err());
            }
        }
    }
    #[test]
    fn canonical_urls_validate_components_and_normalize_default_ports() {
        assert_eq!(
            public_base_url("https://Papers.Example.edu:443/").expect("valid test origin"),
            "https://papers.example.edu"
        );
        for invalid in [
            "http://user:pass@papers.example",
            "https://papers.example/path",
            "https://papers.example?token=x",
            "https://papers.example/#fragment",
            "null",
            "ftp://papers.example",
            "http://papers.example:99999",
            "http://papers.example:+80",
            "http://papers.example:",
        ] {
            assert!(public_base_url(invalid).is_err(), "{invalid}");
        }
    }
    #[test]
    fn same_origin_rejects_other_ports_null_origins_and_duplicate_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::HOST,
            "PAPERS.EXAMPLE:80".parse().expect("valid test origin"),
        );
        headers.insert(
            header::ORIGIN,
            "http://papers.example".parse().expect("valid test origin"),
        );
        assert_eq!(validate_request_origin(&headers), Ok(()));
        headers.insert(
            header::ORIGIN,
            "http://papers.example:9000"
                .parse()
                .expect("valid test origin"),
        );
        assert_eq!(
            validate_request_origin(&headers),
            Err(OriginError::CrossOrigin)
        );
        headers.insert(header::ORIGIN, "null".parse().expect("valid test origin"));
        assert_eq!(validate_request_origin(&headers), Err(OriginError::Invalid));
        headers.insert(
            header::ORIGIN,
            "http://papers.example".parse().expect("valid test origin"),
        );
        headers.append(
            header::ORIGIN,
            "http://papers.example".parse().expect("valid test origin"),
        );
        assert_eq!(validate_request_origin(&headers), Err(OriginError::Invalid));
    }
}
