#[allow(clippy::wildcard_imports)]
use super::*;

pub(crate) fn directory_response(
    entries: &[TreeEntry],
    display_path: &str,
    head: bool,
    cache_policy: CachePolicy,
    set_key_cookie: Option<&str>,
) -> Response {
    let mut html = String::new();
    html.push_str("<!doctype html><meta charset=\"utf-8\"><title>Iris Drive</title>");
    html.push_str("<style>body{font:15px system-ui,sans-serif;max-width:860px;margin:32px auto;padding:0 16px;color:#111}a{color:#0645ad;text-decoration:none}a:hover{text-decoration:underline}ul{line-height:1.9;padding-left:1.2rem}.muted{color:#666}</style>");
    html.push_str("<h1>");
    if display_path.is_empty() {
        html.push('/');
    } else {
        html.push_str(&html_escape(display_path));
    }
    html.push_str("</h1><ul>");
    if !display_path.is_empty() {
        html.push_str("<li><a href=\"../\">../</a></li>");
    }
    for entry in entries {
        let suffix = if entry.link_type == LinkType::Dir {
            "/"
        } else {
            ""
        };
        let href = format!("{}{}", percent_encode_path_segment(&entry.name), suffix);
        html.push_str("<li><a href=\"");
        html.push_str(&href);
        html.push_str("\">");
        html.push_str(&html_escape(&entry.name));
        html.push_str(suffix);
        html.push_str("</a>");
        if entry.link_type != LinkType::Dir {
            html.push_str(" <span class=\"muted\">");
            html.push_str(&entry.size.to_string());
            html.push_str(" bytes</span>");
        }
        html.push_str("</li>");
    }
    html.push_str("</ul>");

    let bytes = html.into_bytes();
    let mut builder = response_builder(StatusCode::OK, head)
        .header(CONTENT_TYPE, "text/html; charset=utf-8")
        .header(CONTENT_LENGTH, bytes.len().to_string())
        .header(CACHE_CONTROL, cache_control(cache_policy))
        .header(X_CONTENT_TYPE_OPTIONS, "nosniff");
    if let Some(key) = set_key_cookie {
        builder = builder.header(SET_COOKIE, key_cookie_value(key));
    }
    builder
        .body(if head {
            Body::empty()
        } else {
            Body::from(bytes)
        })
        .expect("response")
}

pub(crate) fn text_response(status: StatusCode, message: &str) -> Response {
    response_builder(status, false)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Body::from(message.to_string()))
        .expect("response")
}

pub(crate) fn response_builder(status: StatusCode, _head: bool) -> http::response::Builder {
    Response::builder().status(status)
}

pub(crate) fn entry_cid(entry: &TreeEntry) -> Cid {
    Cid {
        hash: entry.hash,
        key: entry.key,
    }
}
