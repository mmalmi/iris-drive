#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn cid_from_nhash(value: &str) -> Result<Cid, GatewayError> {
    let NHashData { hash, decrypt_key } =
        nhash_decode(value).map_err(|e| GatewayError::InvalidRequest(e.to_string()))?;
    Ok(Cid {
        hash,
        key: decrypt_key,
    })
}

pub(crate) fn cid_with_request_key(
    mut cid: Cid,
    uri: &Uri,
    headers: &HeaderMap,
) -> Result<(Cid, Option<String>), GatewayError> {
    if cid.key.is_some() {
        return Ok((cid, None));
    }
    let key = query_param(uri.query(), "k").or_else(|| cookie_value(headers, KEY_COOKIE));
    let Some(key) = key else {
        return Ok((cid, None));
    };
    let parsed = from_hex(&key).map_err(|_| GatewayError::InvalidRequest("invalid key".into()))?;
    cid.key = Some(parsed);
    Ok((cid, Some(to_hex(&parsed))))
}

pub(crate) fn parse_gateway_path(
    path: &str,
) -> Result<(Vec<String>, Option<PathRoute>), GatewayError> {
    let mut segments = decode_path_segments(path)?;
    if segments.first().is_some_and(|segment| segment == "drive") {
        if segments.len() < 2 {
            return Err(GatewayError::InvalidRequest("missing drive id".into()));
        }
        let drive_id = segments.remove(1);
        segments.remove(0);
        return Ok((segments, Some(PathRoute::Drive(drive_id))));
    }
    if segments.first().is_some_and(|segment| segment == "nhash") {
        if segments.len() < 2 {
            return Err(GatewayError::InvalidRequest("missing nhash".into()));
        }
        let nhash = segments.remove(1);
        segments.remove(0);
        return Ok((segments, Some(PathRoute::Nhash(nhash))));
    }
    Ok((segments, None))
}

pub(crate) fn decode_path_segments(path: &str) -> Result<Vec<String>, GatewayError> {
    let mut out = Vec::new();
    for raw in path.split('/').filter(|segment| !segment.is_empty()) {
        let segment = percent_decode(raw)?;
        if segment == "." || segment == ".." || segment.contains('\0') {
            return Err(GatewayError::InvalidRequest("invalid path segment".into()));
        }
        out.push(segment);
    }
    Ok(out)
}

pub(crate) fn percent_decode(value: &str) -> Result<String, GatewayError> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return Err(GatewayError::InvalidRequest("bad percent encoding".into()));
            }
            let hi = hex_value(bytes[i + 1])
                .ok_or_else(|| GatewayError::InvalidRequest("bad percent encoding".into()))?;
            let lo = hex_value(bytes[i + 2])
                .ok_or_else(|| GatewayError::InvalidRequest("bad percent encoding".into()))?;
            out.push((hi << 4) | lo);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).map_err(|_| GatewayError::InvalidRequest("path is not utf-8".into()))
}

pub(crate) fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn query_param(query: Option<&str>, name: &str) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        if percent_decode(key).ok().as_deref() == Some(name) {
            return percent_decode(value).ok();
        }
    }
    None
}

pub(crate) fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    let raw = headers.get(COOKIE)?.to_str().ok()?;
    raw.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name).then(|| value.to_string())
    })
}

pub(crate) fn key_cookie_value(key: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{KEY_COOKIE}={key}; Path=/; HttpOnly; SameSite=Strict"
    ))
    .expect("valid cookie")
}

pub(crate) fn parse_byte_range(value: &str, size: u64) -> Result<(u64, u64), String> {
    let range = value
        .strip_prefix("bytes=")
        .ok_or_else(|| "only bytes ranges are supported".to_string())?;
    let (start_raw, end_raw) = range
        .split_once('-')
        .ok_or_else(|| "missing range delimiter".to_string())?;
    if start_raw.is_empty() {
        let suffix = end_raw
            .parse::<u64>()
            .map_err(|_| "invalid suffix range".to_string())?;
        if suffix == 0 {
            return Err("empty suffix range".into());
        }
        let start = size.saturating_sub(suffix);
        return Ok((start, size));
    }

    let start = start_raw
        .parse::<u64>()
        .map_err(|_| "invalid start".to_string())?;
    let end_inclusive = if end_raw.is_empty() {
        size.saturating_sub(1)
    } else {
        end_raw
            .parse::<u64>()
            .map_err(|_| "invalid end".to_string())?
    };
    if start >= size || end_inclusive < start {
        return Err("range outside file".into());
    }
    Ok((start, end_inclusive.saturating_add(1).min(size)))
}

pub(crate) fn cache_control(policy: CachePolicy) -> &'static str {
    match policy {
        CachePolicy::Immutable => "public, max-age=31536000, immutable",
        CachePolicy::Mutable => "no-cache",
    }
}

pub(crate) fn etag_for(cid: &Cid) -> String {
    format!("\"{}\"", to_hex(&cid.hash))
}

pub(crate) fn mime_type_for_path(
    path: &str,
    meta: Option<&std::collections::HashMap<String, serde_json::Value>>,
) -> String {
    if let Some(mime) = meta
        .and_then(|meta| meta.get("mimeType"))
        .and_then(serde_json::Value::as_str)
        .filter(|mime| !mime.trim().is_empty())
    {
        return mime.to_string();
    }
    mime_guess::from_path(path)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

pub(crate) fn append_path(path: &str, child: &str) -> String {
    if path.is_empty() {
        child.to_string()
    } else {
        format!("{path}/{child}")
    }
}

pub(crate) fn normalize_host(host: &str) -> String {
    let trimmed = host.trim().trim_end_matches('.').to_ascii_lowercase();
    if let Some(inner) = trimmed
        .strip_prefix('[')
        .and_then(|v| v.split_once(']').map(|(h, _)| h))
    {
        return inner.to_string();
    }
    trimmed
        .rsplit_once(':')
        .and_then(|(head, tail)| tail.parse::<u16>().ok().map(|_| head.to_string()))
        .unwrap_or(trimmed)
}

pub(crate) fn is_loopback_host(host: &str) -> bool {
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

pub(crate) fn is_safe_drive_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_'))
}

pub(crate) fn nhash_from_split_host(host: &str, suffix: &str) -> Option<String> {
    let labels = host.strip_suffix(suffix)?;
    if labels.is_empty() {
        return None;
    }
    let mut nhash = String::new();
    for label in labels.split('.') {
        if label.is_empty()
            || label.len() > 63
            || !label
                .bytes()
                .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9'))
        {
            return None;
        }
        nhash.push_str(label);
    }
    nhash.starts_with("nhash1").then_some(nhash)
}

pub(crate) fn split_dns_labels(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + value.len() / 63);
    for (index, chunk) in value.as_bytes().chunks(63).enumerate() {
        if index > 0 {
            out.push('.');
        }
        out.push_str(std::str::from_utf8(chunk).expect("ascii"));
    }
    out
}

pub(crate) fn percent_encode_path_segment(segment: &str) -> String {
    let mut encoded = String::new();
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(char::from(byte));
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(&mut encoded, "%{byte:02X}");
            }
        }
    }
    encoded
}

pub(crate) fn html_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

const BASE32_ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

#[must_use]
pub fn encode_immutable_host_label(hash: &Hash) -> String {
    let mut bits = 0u32;
    let mut value = 0u32;
    let mut output = String::new();
    for byte in hash {
        value = (value << 8) | u32::from(*byte);
        bits += 8;
        while bits >= 5 {
            let index = ((value >> (bits - 5)) & 31) as usize;
            output.push(char::from(BASE32_ALPHABET[index]));
            bits -= 5;
        }
    }
    if bits > 0 {
        let index = ((value << (5 - bits)) & 31) as usize;
        output.push(char::from(BASE32_ALPHABET[index]));
    }
    output
}

pub(crate) fn decode_base32_hash(label: &str) -> Option<Hash> {
    let mut bits = 0u32;
    let mut current = 0u32;
    let mut bytes = Vec::with_capacity(32);
    for ch in label.trim().bytes() {
        let index = BASE32_ALPHABET.iter().position(|b| *b == ch)?;
        current = (current << 5) | u32::try_from(index).ok()?;
        bits += 5;
        if bits >= 8 {
            bytes.push(((current >> (bits - 8)) & 0xff) as u8);
            bits -= 8;
        }
    }
    if bytes.len() != 32 {
        return None;
    }
    let mut hash = [0u8; 32];
    hash.copy_from_slice(&bytes);
    Some(hash)
}
