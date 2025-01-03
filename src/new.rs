use std::{ffi::OsStr, path::PathBuf, sync::Arc, time::Duration};

use axum::{
    body::Body,
    extract::{Query, State},
};
use axum_extra::TypedHeader;
use headers::ContentLength;
use http::StatusCode;
use serde::Deserialize;
use serde_with::{serde_as, DurationSeconds};

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
#[axum::debug_handler]
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

    let extension = PathBuf::from(req.name)
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_string();

    // turn body into stream
    let stream = Body::into_data_stream(body);

    // pass it off to the engine to be processed
    // --
    // also, error responses here don't get represented properly in ShareX most of the time
    // they don't expect the connection to close before they're done uploading, i think
    // so it will just present the user with a "connection closed" error
    match engine
        .process(
            &extension,
            content_length,
            stream,
            req.last_for,
            req.keep_exif,
        )
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
        Err(_) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}
