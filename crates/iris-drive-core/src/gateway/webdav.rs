#[allow(clippy::wildcard_imports)]
use super::*;

#[derive(Debug)]
pub(crate) enum WebDavNode {
    Directory {
        cid: Cid,
    },
    File {
        cid: Cid,
        size: u64,
        path: String,
        mime_type: String,
    },
}

pub(crate) async fn handle_webdav_request(
    state: GatewayState,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
    path_segments: Vec<String>,
) -> Result<Response, (StatusCode, String)> {
    validate_webdav_path(&path_segments)?;
    match method.as_str() {
        "OPTIONS" => Ok(webdav_options_response()),
        "PROPFIND" => webdav_propfind_response(&state, &headers, &path_segments).await,
        "GET" | "HEAD" => {
            webdav_get_response(&state, &headers, &path_segments, method == Method::HEAD).await
        }
        "PUT" => webdav_put(&state, &path_segments, body).await,
        "DELETE" => webdav_delete(&state, &path_segments).await,
        "MKCOL" => webdav_mkcol(&state, &path_segments).await,
        "MOVE" => webdav_move_or_copy(&state, &headers, &uri, &path_segments, true).await,
        "COPY" => webdav_move_or_copy(&state, &headers, &uri, &path_segments, false).await,
        "LOCK" => Ok(webdav_lock_response()),
        "UNLOCK" => Ok(status_response(StatusCode::NO_CONTENT)),
        _ => Err((StatusCode::METHOD_NOT_ALLOWED, "method not allowed".into())),
    }
}

pub(crate) fn validate_webdav_path(path_segments: &[String]) -> Result<(), (StatusCode, String)> {
    if path_segments.iter().any(|segment| segment == ".hashtree") {
        return Err((
            StatusCode::FORBIDDEN,
            "internal metadata is not writable".into(),
        ));
    }
    Ok(())
}

pub(crate) fn webdav_ignored_path(path_segments: &[String]) -> bool {
    path_segments
        .iter()
        .any(|segment| crate::indexer::should_ignore_name(segment))
}

pub(crate) fn webdav_options_response() -> Response {
    response_builder(StatusCode::OK, false)
        .header("DAV", "1, 2")
        .header(
            "Allow",
            "OPTIONS, PROPFIND, GET, HEAD, PUT, DELETE, MKCOL, MOVE, COPY, LOCK, UNLOCK",
        )
        .header("MS-Author-Via", "DAV")
        .body(Body::empty())
        .expect("response")
}

pub(crate) fn webdav_lock_response() -> Response {
    let token = "opaquelocktoken:iris-drive";
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <D:prop xmlns:D=\"DAV:\"><D:lockdiscovery><D:activelock>\
         <D:locktype><D:write/></D:locktype><D:lockscope><D:exclusive/></D:lockscope>\
         <D:depth>Infinity</D:depth><D:owner>Iris Drive</D:owner>\
         <D:timeout>Second-3600</D:timeout><D:locktoken><D:href>{}</D:href></D:locktoken>\
         </D:activelock></D:lockdiscovery></D:prop>",
        html_escape(token)
    );
    response_builder(StatusCode::OK, false)
        .header("DAV", "1, 2")
        .header("Lock-Token", format!("<{token}>"))
        .header(CONTENT_TYPE, "application/xml; charset=utf-8")
        .header(CONTENT_LENGTH, body.len().to_string())
        .body(Body::from(body))
        .expect("response")
}

pub(crate) async fn webdav_propfind_response(
    state: &GatewayState,
    headers: &HeaderMap,
    path_segments: &[String],
) -> Result<Response, (StatusCode, String)> {
    let root = current_webdav_root(state).await?;
    let node = resolve_webdav_node(&state.tree, &root, path_segments)
        .await?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "not found".into()))?;
    let depth = headers
        .get("depth")
        .and_then(|value| value.to_str().ok())
        .unwrap_or("infinity");

    let mut xml =
        String::from("<?xml version=\"1.0\" encoding=\"utf-8\"?><D:multistatus xmlns:D=\"DAV:\">");
    push_webdav_prop_response(&mut xml, path_segments, &node);

    if depth != "0"
        && let WebDavNode::Directory { cid } = node
    {
        let entries = list_public_directory(&state.tree, &cid)
            .await
            .map_err(webdav_internal_error)?;
        for entry in entries {
            let mut child_path = path_segments.to_vec();
            child_path.push(entry.name.clone());
            let child_cid = entry_cid(&entry);
            let child_node = if entry.link_type == LinkType::Dir {
                WebDavNode::Directory { cid: child_cid }
            } else {
                let path = child_path.join("/");
                WebDavNode::File {
                    cid: child_cid,
                    size: entry.size,
                    mime_type: mime_type_for_path(&path, entry.meta.as_ref()),
                    path,
                }
            };
            push_webdav_prop_response(&mut xml, &child_path, &child_node);
        }
    }

    xml.push_str("</D:multistatus>");
    let status = StatusCode::from_u16(207).expect("207 is valid");
    Ok(response_builder(status, false)
        .header(CONTENT_TYPE, "application/xml; charset=utf-8")
        .header(CONTENT_LENGTH, xml.len().to_string())
        .body(Body::from(xml))
        .expect("response"))
}

pub(crate) async fn webdav_get_response(
    state: &GatewayState,
    headers: &HeaderMap,
    path_segments: &[String],
    head: bool,
) -> Result<Response, (StatusCode, String)> {
    let root = current_webdav_root(state).await?;
    let node = resolve_webdav_node(&state.tree, &root, path_segments)
        .await?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "not found".into()))?;
    match node {
        WebDavNode::Directory { .. } => Err((StatusCode::FORBIDDEN, "directory".into())),
        WebDavNode::File {
            cid,
            size,
            path,
            mime_type,
        } => {
            let options = FileResponseOptions {
                size,
                path: &path,
                mime_type: &mime_type,
                head,
                cache_policy: CachePolicy::Mutable,
                set_key_cookie: None,
                headers,
            };
            serve_file(&state.tree, &cid, options).await
        }
    }
}

pub(crate) async fn webdav_put(
    state: &GatewayState,
    path_segments: &[String],
    body: Bytes,
) -> Result<Response, (StatusCode, String)> {
    let (name, parent) = split_webdav_parent(path_segments)?;
    if webdav_ignored_path(path_segments) {
        return Ok(status_response(StatusCode::NO_CONTENT));
    }
    let mut root = current_webdav_root(state).await?;
    root = ensure_webdav_parent_dirs(&state.tree, root, parent).await?;
    let (cid, size) = state
        .tree
        .put(&body)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let link_type = if size > DEFAULT_CHUNK_SIZE as u64 {
        LinkType::File
    } else {
        LinkType::Blob
    };
    let parent_refs = path_refs(parent);
    let parent_cid = resolve_dir(&state.tree, &root, parent_refs.as_slice()).await?;
    let existed = find_entry(&state.tree, &parent_cid, name)
        .await
        .map_err(webdav_internal_error)?
        .is_some();
    let root = state
        .tree
        .set_entry(&root, parent_refs.as_slice(), name, &cid, size, link_type)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    publish_webdav_root(state, root).await?;
    Ok(status_response(if existed {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CREATED
    }))
}

pub(crate) async fn webdav_delete(
    state: &GatewayState,
    path_segments: &[String],
) -> Result<Response, (StatusCode, String)> {
    let (name, parent) = split_webdav_parent(path_segments)?;
    if webdav_ignored_path(path_segments) {
        return Ok(status_response(StatusCode::NO_CONTENT));
    }
    let root = current_webdav_root(state).await?;
    let parent_refs = path_refs(parent);
    let parent_cid = resolve_dir(&state.tree, &root, parent_refs.as_slice()).await?;
    if find_entry(&state.tree, &parent_cid, name)
        .await
        .map_err(webdav_internal_error)?
        .is_none()
    {
        return Err((StatusCode::NOT_FOUND, "not found".into()));
    }
    let root = state
        .tree
        .remove_entry(&root, parent_refs.as_slice(), name)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    publish_webdav_root(state, root).await?;
    Ok(status_response(StatusCode::NO_CONTENT))
}

pub(crate) async fn webdav_mkcol(
    state: &GatewayState,
    path_segments: &[String],
) -> Result<Response, (StatusCode, String)> {
    let (name, parent) = split_webdav_parent(path_segments)?;
    if webdav_ignored_path(path_segments) {
        return Ok(status_response(StatusCode::CREATED));
    }
    let mut root = current_webdav_root(state).await?;
    root = ensure_webdav_parent_dirs(&state.tree, root, parent).await?;
    let parent_refs = path_refs(parent);
    let parent_cid = resolve_dir(&state.tree, &root, parent_refs.as_slice()).await?;
    if find_entry(&state.tree, &parent_cid, name)
        .await
        .map_err(webdav_internal_error)?
        .is_some()
    {
        return Err((StatusCode::METHOD_NOT_ALLOWED, "already exists".into()));
    }
    let dir = state
        .tree
        .put_directory(Vec::new())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let root = state
        .tree
        .set_entry(&root, parent_refs.as_slice(), name, &dir, 0, LinkType::Dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    publish_webdav_root(state, root).await?;
    Ok(status_response(StatusCode::CREATED))
}

pub(crate) async fn webdav_move_or_copy(
    state: &GatewayState,
    headers: &HeaderMap,
    uri: &Uri,
    source_segments: &[String],
    remove_source: bool,
) -> Result<Response, (StatusCode, String)> {
    let destination = destination_path(headers, uri)?;
    validate_webdav_path(&destination)?;
    if webdav_ignored_path(source_segments) || webdav_ignored_path(&destination) {
        return Ok(status_response(StatusCode::NO_CONTENT));
    }
    if source_segments == destination.as_slice() {
        return Ok(status_response(StatusCode::NO_CONTENT));
    }

    let (source_name, source_parent) = split_webdav_parent(source_segments)?;
    let (dest_name, dest_parent) = split_webdav_parent(&destination)?;
    let overwrite = headers
        .get("overwrite")
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| !value.eq_ignore_ascii_case("f"));

    let mut root = current_webdav_root(state).await?;
    let source_parent_refs = path_refs(source_parent);
    let source_parent_cid = resolve_dir(&state.tree, &root, source_parent_refs.as_slice()).await?;
    let source_entry = find_entry(&state.tree, &source_parent_cid, source_name)
        .await
        .map_err(webdav_internal_error)?
        .ok_or_else(|| (StatusCode::NOT_FOUND, "not found".into()))?;

    root = ensure_webdav_parent_dirs(&state.tree, root, dest_parent).await?;
    let dest_parent_refs = path_refs(dest_parent);
    let dest_parent_cid = resolve_dir(&state.tree, &root, dest_parent_refs.as_slice()).await?;
    if find_entry(&state.tree, &dest_parent_cid, dest_name)
        .await
        .map_err(webdav_internal_error)?
        .is_some()
        && !overwrite
    {
        return Err((StatusCode::PRECONDITION_FAILED, "destination exists".into()));
    }

    let cid = entry_cid(&source_entry);
    root = state
        .tree
        .set_entry(
            &root,
            dest_parent_refs.as_slice(),
            dest_name,
            &cid,
            source_entry.size,
            source_entry.link_type,
        )
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if remove_source {
        root = state
            .tree
            .remove_entry(&root, source_parent_refs.as_slice(), source_name)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    publish_webdav_root(state, root).await?;
    Ok(status_response(StatusCode::CREATED))
}

pub(crate) async fn current_webdav_root(state: &GatewayState) -> Result<Cid, (StatusCode, String)> {
    let config_mtime = config_modified_time(&state.config_dir);
    let mut pinned_root = None;
    let mut pinned_until = None;
    {
        let cache = state.webdav_root.lock().await;
        if let Some(root) = cache.root.as_ref() {
            if cache.config_mtime == config_mtime {
                return Ok(root.clone());
            }
            if let Some(deadline) = cache.pinned_until
                && Instant::now() < deadline
            {
                pinned_root = Some(root.clone());
                pinned_until = Some(deadline);
            }
        }
    }

    if let Some(root) = pinned_root
        && let Some(merged) = webdav_root_including_pending_root(state, &root).await?
    {
        let mut cache = state.webdav_root.lock().await;
        cache.root = Some(merged.clone());
        cache.config_mtime = config_mtime;
        cache.pinned_until = pinned_until;
        return Ok(merged);
    }

    let daemon = Daemon::open(state.config_dir.as_ref())
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e.to_string()))?;
    let visible = crate::primary_merged_root(daemon.tree(), daemon.config())
        .await
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e.to_string()))?;
    let mut cache = state.webdav_root.lock().await;
    cache.root = Some(visible.root_cid.clone());
    cache.config_mtime = config_mtime;
    cache.pinned_until = None;
    Ok(visible.root_cid)
}

pub(crate) async fn webdav_root_including_pending_root(
    state: &GatewayState,
    pending_root: &Cid,
) -> Result<Option<Cid>, (StatusCode, String)> {
    let daemon = Daemon::open(state.config_dir.as_ref())
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e.to_string()))?;
    let mut config = daemon.config().clone();
    let Some(account) = config.account.as_ref() else {
        return Ok(Some(pending_root.clone()));
    };
    let Some(mut drive) = config.drive(PRIMARY_DRIVE_ID).cloned() else {
        return Ok(Some(pending_root.clone()));
    };
    let root_meta = crate::indexer::read_root_meta(daemon.tree(), pending_root)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let mut root = root_meta.map_or_else(
        || DeviceRootRef::legacy(pending_root.to_string(), unix_now_seconds(), 0),
        |meta| DeviceRootRef::from_meta(pending_root.to_string(), meta.created_at, &meta),
    );
    root.materialized_only = false;
    drive
        .device_roots
        .insert(account.device_pubkey.clone(), root);
    config.upsert_drive(drive);
    let visible = crate::primary_merged_root(daemon.tree(), &config)
        .await
        .map_err(|e| (StatusCode::SERVICE_UNAVAILABLE, e.to_string()))?;
    Ok(Some(visible.root_cid))
}

pub(crate) async fn publish_webdav_root(
    state: &GatewayState,
    root: Cid,
) -> Result<(), (StatusCode, String)> {
    let Some(tx) = state.root_update_tx.as_ref() else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            "webdav writes require the iris-drive daemon".into(),
        ));
    };
    {
        let mut cache = state.webdav_root.lock().await;
        cache.root = Some(root.clone());
        cache.config_mtime = config_modified_time(&state.config_dir);
        cache.pinned_until = Some(Instant::now() + WEBDAV_WRITE_PIN);
    }
    tx.send(root).map_err(|_| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            "root update worker stopped".into(),
        )
    })
}

pub(crate) fn config_modified_time(config_dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(config_path_in(config_dir))
        .and_then(|metadata| metadata.modified())
        .ok()
}

pub(crate) fn unix_now_seconds() -> i64 {
    use std::time::UNIX_EPOCH;
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}

pub(crate) async fn resolve_webdav_node<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    segments: &[String],
) -> Result<Option<WebDavNode>, (StatusCode, String)> {
    if segments.is_empty() {
        return Ok(Some(WebDavNode::Directory { cid: root.clone() }));
    }

    let mut current = root.clone();
    for (index, segment) in segments.iter().enumerate() {
        let Some(entry) = find_entry(tree, &current, segment)
            .await
            .map_err(webdav_internal_error)?
        else {
            return Ok(None);
        };
        let cid = entry_cid(&entry);
        if index + 1 == segments.len() {
            return if entry.link_type == LinkType::Dir {
                Ok(Some(WebDavNode::Directory { cid }))
            } else {
                let path = segments.join("/");
                Ok(Some(WebDavNode::File {
                    cid,
                    size: entry.size,
                    mime_type: mime_type_for_path(&path, entry.meta.as_ref()),
                    path,
                }))
            };
        }
        if entry.link_type != LinkType::Dir {
            return Ok(None);
        }
        current = cid;
    }

    Ok(None)
}

pub(crate) async fn ensure_webdav_parent_dirs<S: Store>(
    tree: &HashTree<S>,
    mut root: Cid,
    parent: &[String],
) -> Result<Cid, (StatusCode, String)> {
    for depth in 1..=parent.len() {
        root = ensure_webdav_dir(tree, root, &parent[..depth]).await?;
    }
    Ok(root)
}

pub(crate) async fn ensure_webdav_dir<S: Store>(
    tree: &HashTree<S>,
    root: Cid,
    path: &[String],
) -> Result<Cid, (StatusCode, String)> {
    let (name, parent) = split_webdav_parent(path)?;
    let parent_refs = path_refs(parent);
    let parent_cid = resolve_dir(tree, &root, parent_refs.as_slice()).await?;
    if let Some(existing) = find_entry(tree, &parent_cid, name)
        .await
        .map_err(webdav_internal_error)?
    {
        if existing.link_type == LinkType::Dir {
            return Ok(root);
        }
        return Err((StatusCode::CONFLICT, "path parent is a file".into()));
    }
    let dir = tree
        .put_directory(Vec::new())
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    tree.set_entry(&root, parent_refs.as_slice(), name, &dir, 0, LinkType::Dir)
        .await
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub(crate) async fn resolve_dir<S: Store>(
    tree: &HashTree<S>,
    root: &Cid,
    path: &[&str],
) -> Result<Cid, (StatusCode, String)> {
    let mut current = root.clone();
    for segment in path {
        let entry = find_entry(tree, &current, segment)
            .await
            .map_err(webdav_internal_error)?
            .ok_or_else(|| (StatusCode::CONFLICT, "parent directory is missing".into()))?;
        if entry.link_type != LinkType::Dir {
            return Err((StatusCode::CONFLICT, "path parent is a file".into()));
        }
        current = entry_cid(&entry);
    }
    Ok(current)
}

pub(crate) fn split_webdav_parent(
    path: &[String],
) -> Result<(&str, &[String]), (StatusCode, String)> {
    let Some((name, parent)) = path.split_last() else {
        return Err((StatusCode::METHOD_NOT_ALLOWED, "root is read-only".into()));
    };
    Ok((name.as_str(), parent))
}

pub(crate) fn path_refs(path: &[String]) -> Vec<&str> {
    path.iter().map(String::as_str).collect()
}

pub(crate) fn destination_path(
    headers: &HeaderMap,
    uri: &Uri,
) -> Result<Vec<String>, (StatusCode, String)> {
    let destination = headers
        .get("destination")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "missing Destination header".into()))?;
    let parsed = destination
        .parse::<Uri>()
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    if let Some(authority) = parsed.authority()
        && let Some(request_authority) = uri.authority()
        && normalize_host(authority.as_str()) != normalize_host(request_authority.as_str())
    {
        return Err((
            StatusCode::BAD_GATEWAY,
            "cross-host WebDAV moves are not supported".into(),
        ));
    }
    let (segments, route) =
        parse_gateway_path(parsed.path()).map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
    match route {
        Some(PathRoute::WebDav) => Ok(segments),
        _ => Err((
            StatusCode::BAD_REQUEST,
            "Destination must be under /dav".into(),
        )),
    }
}

pub(crate) fn push_webdav_prop_response(
    xml: &mut String,
    path_segments: &[String],
    node: &WebDavNode,
) {
    let (is_dir, cid, size, mime_type) = match node {
        WebDavNode::Directory { cid } => (true, cid, None, None),
        WebDavNode::File {
            cid,
            size,
            mime_type,
            ..
        } => (false, cid, Some(*size), Some(mime_type.as_str())),
    };
    let href = webdav_href(path_segments, is_dir);
    let display_name = path_segments.last().map_or("", String::as_str);
    xml.push_str("<D:response><D:href>");
    xml.push_str(&html_escape(&href));
    xml.push_str("</D:href><D:propstat><D:prop><D:displayname>");
    xml.push_str(&html_escape(display_name));
    xml.push_str("</D:displayname><D:resourcetype>");
    if is_dir {
        xml.push_str("<D:collection/>");
    }
    xml.push_str("</D:resourcetype><D:getetag>");
    xml.push_str(&html_escape(&etag_for(cid)));
    xml.push_str(
        "</D:getetag><D:getlastmodified>Thu, 01 Jan 1970 00:00:00 GMT</D:getlastmodified>",
    );
    if let Some(size) = size {
        xml.push_str("<D:getcontentlength>");
        xml.push_str(&size.to_string());
        xml.push_str("</D:getcontentlength>");
    }
    if let Some(mime_type) = mime_type {
        xml.push_str("<D:getcontenttype>");
        xml.push_str(&html_escape(mime_type));
        xml.push_str("</D:getcontenttype>");
    }
    xml.push_str("</D:prop><D:status>HTTP/1.1 200 OK</D:status></D:propstat></D:response>");
}

pub(crate) fn webdav_href(path_segments: &[String], is_dir: bool) -> String {
    let mut href = String::from("/dav");
    if path_segments.is_empty() {
        href.push('/');
        return href;
    }
    for segment in path_segments {
        href.push('/');
        href.push_str(&percent_encode_path_segment(segment));
    }
    if is_dir {
        href.push('/');
    }
    href
}

pub(crate) fn status_response(status: StatusCode) -> Response {
    response_builder(status, false)
        .body(Body::empty())
        .expect("response")
}

#[allow(clippy::needless_pass_by_value)]
pub(crate) fn webdav_internal_error(error: GatewayError) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, error.to_string())
}
