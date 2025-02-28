use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use axum::{
    body::Body,
    extract::{Query, State},
};
use axum_extra::TypedHeader;
use headers::ContentLength;
use http::StatusCode;
use serde::Deserialize;
use serde_with::{serde_as, DurationSeconds};
use tracing::error;

use crate::engine::ProcessOutcome;

fn default_keep_exif() -> bool {
    false
}

#[serde_as]
#[derive(Deserialize)]
pub struct NewRequest {
    name: String,
    key: Option<String>,

    #[serde(rename = "lastfor")]
    #[serde_as(as = "Option<DurationSeconds>")]
    last_for: Option<Duration>,

    #[serde(rename = "keepexif", default = "default_keep_exif")]
    keep_exif: bool,
}

/// The request handler for the /new path.
/// This handles all new uploads.
pub async fn new(
    State(engine): State<Arc<crate::engine::Engine>>,
    Query(req): Query<NewRequest>,
    TypedHeader(ContentLength(content_length)): TypedHeader<ContentLength>,
    body: Body,
) -> Result<String, StatusCode> {
    // check upload key, if i need to
    if !engine.cfg.upload_key.is_empty() && req.key.unwrap_or_default() != engine.cfg.upload_key {
        return Err(StatusCode::FORBIDDEN);
    }

    // the original file name wasn't given, so i can't work out what the extension should be
    if req.name.is_empty() {
        return Err(StatusCode::BAD_REQUEST);
    }

    // -- try to figure out a file extension..

    fn extension(pb: &Path) -> Option<String> {
        pb.extension().and_then(OsStr::to_str).map(str::to_string)
    }

    let pb = PathBuf::from(req.name);
    let mut ext = extension(&pb);

    // common extensions that usually have a second extension before themselves
    const ADDITIVE: &[&str] = &["gz", "xz", "bz2", "lz4", "zst"];

    // if the extension is one of those, try to find that second extension
    if ext
        .as_ref()
        .is_some_and(|ext| ADDITIVE.contains(&ext.as_str()))
    {
        // try to parse out another extension
        let stem = pb.file_stem().unwrap(); // SAFETY: if extension is Some(), this will also be

        if let Some(second_ext) = extension(&PathBuf::from(stem)) {
            // there is another extension,
            // try to make sure it's one we want
            // 4 is enough for most common file extensions
            // and not many false positives, hopefully
            if second_ext.len() <= 4 {
                // seems ok so combine them
                ext = ext.as_ref().map(|first_ext| second_ext + "." + first_ext);
            }
        }
    }

    // turn body into stream
    let stream = Body::into_data_stream(body);

    // pass it off to the engine to be processed
    // --
    // also, error responses here don't get presented properly in ShareX most of the time
    // they don't expect the connection to close before they're done uploading, i think
    // so it will just present the user with a "connection closed" error
    match engine
        .process(ext, content_length, stream, req.last_for, req.keep_exif)
        .await
    {
        Ok(outcome) => match outcome {
            // 200 OK
            ProcessOutcome::Success(url) => Ok(url),

            // 413 Payload Too Large
            ProcessOutcome::UploadTooLarge | ProcessOutcome::TemporaryUploadTooLarge => {
                Err(StatusCode::PAYLOAD_TOO_LARGE)
            }

            // 400 Bad Request
            ProcessOutcome::TemporaryUploadLifetimeTooLong => Err(StatusCode::BAD_REQUEST),
        },

        // 500 Internal Server Error
        Err(err) => {
            error!("failed to process upload!! {err:#}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}
