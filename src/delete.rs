use std::sync::{Arc, atomic::Ordering};

use axum::extract::{Query, State};
use base64::{Engine as _, prelude::BASE64_URL_SAFE_NO_PAD};
use bytes::{Buf, BytesMut};
use hmac::Mac;
use http::StatusCode;
use serde::Deserialize;

use crate::engine::{Engine, update_hmac};

#[derive(Deserialize)]
pub struct DeleteRequest {
    name: String,
    hash: String,
    hmac: String,
}

pub async fn delete(
    State(engine): State<Arc<Engine>>,
    Query(req): Query<DeleteRequest>,
) -> (StatusCode, &'static str) {
    let Some(mut hmac) = engine.deletion_hmac.clone() else {
        return (StatusCode::CONFLICT, "Deletion is not enabled");
    };

    // -- decode provided data

    // decode user-given hmac
    let Ok(provided_hmac) = BASE64_URL_SAFE_NO_PAD.decode(req.hmac) else {
        return (StatusCode::BAD_REQUEST, "Could not decode hmac");
    };

    // decode hash from base64
    let Ok(mut provided_hash_data) = BASE64_URL_SAFE_NO_PAD
        .decode(req.hash)
        .map(|v| BytesMut::from(&v[..]))
    else {
        return (StatusCode::BAD_REQUEST, "Could not decode partial hash");
    };
    // read hash
    if provided_hash_data.len() != 16 {
        return (StatusCode::BAD_REQUEST, "Partial hash length is invalid");
    }
    let provided_hash = provided_hash_data.get_u128();

    // -- verify it

    // check if info is valid
    let is_hmac_valid = {
        // update hmad
        update_hmac(&mut hmac, &req.name, provided_hash);
        // verify..
        hmac.verify_slice(&provided_hmac).is_ok()
    };
    if !is_hmac_valid {
        return (StatusCode::BAD_REQUEST, "Hmac is invalid");
    }

    // -- ensure hash matches

    // okay, now check if we compute the same hash as the req
    // this makes sure it's (probably) the same file
    let actual_hash = match engine.get_hash(&req.name).await {
        Ok(Some(h)) => h,
        Ok(None) => return (StatusCode::NOT_FOUND, "File not found"),
        Err(err) => {
            tracing::error!(%err, "failed to get hash");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal server error!!");
        }
    };
    // compare
    if provided_hash != actual_hash {
        return (StatusCode::BAD_REQUEST, "Partial hash did not match");
    }

    // -- delete file

    // everything seems okay so try to delete
    if let Err(err) = engine.remove(&req.name).await {
        tracing::error!(%err, "failed to delete upload");
        return (StatusCode::INTERNAL_SERVER_ERROR, "Delete failed");
    }

    // decrement upload count
    engine.upl_count.fetch_sub(1, Ordering::Relaxed);

    (StatusCode::OK, "Deleted successfully!")
}
