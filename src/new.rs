use std::{collections::HashMap, ffi::OsStr, path::PathBuf, sync::Arc};

use axum::{
    extract::{BodyStream, Query, State},
    http::HeaderValue,
};
use bytes::{BufMut, Bytes, BytesMut};
use hyper::{header, HeaderMap, StatusCode};
use rand::Rng;
use tokio::{
    fs::File,
    io::AsyncWriteExt,
    sync::mpsc::{self, Receiver, Sender},
};
use tokio_stream::StreamExt;

use crate::cache;

// create an upload name from an original file name
fn gen_path(original_name: &String) -> PathBuf {
    // extract extension from original name
    let extension = PathBuf::from(original_name)
        .extension()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_string();

    // generate a 6-character alphanumeric string
    let id: String = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(6)
        .map(char::from)
        .collect();

    // create the path
    let mut path = PathBuf::new();
    path.push("uploads/");
    path.push(id);
    path.set_extension(extension);

    // if we're already using it, try again
    if path.exists() {
        gen_path(original_name)
    } else {
        path
    }
}

#[axum::debug_handler]
pub async fn new(
    State(state): State<Arc<crate::state::AppState>>,
    headers: HeaderMap,
    Query(params): Query<HashMap<String, String>>,
    mut stream: BodyStream,
) -> Result<String, StatusCode> {
    // require name parameter, it's used for determining the file extension
    if !params.contains_key("name") {
        return Err(StatusCode::BAD_REQUEST);
    }

    // generate a path, take the name, format a url
    let path = gen_path(params.get("name").unwrap());

    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or_default()
        .to_string();

    // if we fail generating a name, stop now
    if name.is_empty() {
        return Err(StatusCode::INTERNAL_SERVER_ERROR);
    }

    let url = format!("http://127.0.0.1:8000/p/{}", name);

    // get the content length, and if parsing it fails, assume it's really big so it doesn't cache
    let content_length = headers
        .get(header::CONTENT_LENGTH)
        .unwrap_or(&HeaderValue::from_static(""))
        .to_str()
        .and_then(|s| Ok(usize::from_str_radix(s, 10)))
        .unwrap()
        .unwrap_or(usize::MAX);

    // if the upload size exceeds 80 MB, we skip caching!
    // previously, i was going to use redis with a 500 MB max (redis's is 512MiB)
    // with or without redis, 500 MB is still a bit much..
    // it could probably be read from disk before anyone could fully download it
    let mut use_cache = content_length < cache::MAX_LENGTH;

    info!(
        target: "new",
        "received an upload! content length: {}, using cache: {}",
        content_length, use_cache
    );

    // create file to save upload to
    let mut file = File::create(path)
        .await
        .expect("could not open file! make sure your upload path exists");

    // if we're using cache, make some space to store the upload in
    let mut data = if use_cache {
        BytesMut::with_capacity(content_length)
    } else {
        BytesMut::new()
    };

    // start a task that handles saving files to disk (we can save to cache/disk in parallel that way)
    let (tx, mut rx): (Sender<Bytes>, Receiver<Bytes>) = mpsc::channel(1);

    tokio::spawn(async move {
        // receive chunks and save them to file
        while let Some(chunk) = rx.recv().await {
            debug!(target: "new", "writing chunk to disk (length: {})", chunk.len());
            file.write_all(&chunk)
                .await
                .expect("error while writing file to disk");
        }
    });

    // read and save upload
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.unwrap();

        // send chunk to io task
        debug!(target: "new", "sending data to io task");
        tx.send(chunk.clone())
            .await
            .expect("failed to send data to io task");

        if use_cache {
            debug!(target: "new", "receiving data into buffer");
            if data.len() + chunk.len() > data.capacity() {
                error!(target: "new", "too much data! the client had an invalid content-length!");

                // if we receive too much data, drop the buffer and stop using cache (it is still okay to use disk, probably)
                data = BytesMut::new();
                use_cache = false;
            } else {
                data.put(chunk);
            }
        }
    }

    // insert upload into cache if necessary
    if use_cache {
        let mut cache = state.cache.lock().await;

        info!(target: "new", "caching upload!");
        cache.insert(name, data.freeze(), Some(cache::DURATION));
    }

    Ok(url)
}
