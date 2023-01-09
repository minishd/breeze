use std::{ffi::OsStr, path::PathBuf, sync::atomic::AtomicUsize, time::Duration};

use axum::{
    extract::BodyStream,
    http::HeaderValue,
    response::{IntoResponse, Response},
};
use bytes::{Bytes, BytesMut};
use memory_cache::MemoryCache;
use mime_guess::mime;

pub const MAX_LENGTH: usize = 80_000_000;
pub const DURATION: Duration = Duration::from_secs(8);
pub const FULL_SCAN_FREQ: Duration = Duration::from_secs(1);

pub fn get_response(cache: &mut MemoryCache<String, Bytes>, original_path: PathBuf) -> Response {
    let name = original_path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_string();

    let cache_item = cache.get(&name.clone());

    let data = cache_item.unwrap().clone();

    let content_type = mime_guess::from_path(original_path)
        .first()
        .unwrap_or(mime::APPLICATION_OCTET_STREAM)
        .to_string();

    let mut res = data.into_response();
    let headers = res.headers_mut();

    headers.clear();
    headers.insert(
        "content-type",
        HeaderValue::from_str(content_type.as_str()).unwrap(),
    );

    return res;
}
