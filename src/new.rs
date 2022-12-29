use std::{collections::HashMap, ffi::OsStr, io::Read, path::PathBuf, sync::Arc, time::Duration};

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
        .to_string(); // i hope this never happens. that would suck

    let url = format!("http://127.0.0.1:8000/p/{}", name);

    // process the upload in the background so i can send the URL back immediately!
    // this isn't supported by ShareX (it waits for the request to complete before handling the response)
    tokio::spawn(async move {
        // get the content length, and if parsing it fails, assume it's really big
        // it may be better to make it fully content-length not required because this feels kind of redundant
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
        let mut use_cache = content_length < 80_000_000;

        println!(
            "[upl] content length: {} using cache: {}",
            content_length, use_cache
        );

        // create file to save upload to
        let mut file = File::create(path)
            .await
            .expect("could not open file! make sure your upload path exists");

        let mut data: BytesMut = if use_cache {
            BytesMut::with_capacity(content_length)
        } else {
            BytesMut::new()
        };

        let (tx, mut rx): (Sender<Bytes>, Receiver<Bytes>) = mpsc::channel(1);

        tokio::spawn(async move {
            while let Some(chunk) = rx.recv().await {
                println!("[io] received new chunk");
                file.write_all(&chunk)
                    .await
                    .expect("error while writing file to disk");
            }
        });

        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();

            println!("[upl] sending data to io task");
            tx.send(chunk.clone()).await.unwrap();

            if use_cache {
                println!("[upl] receiving data into cache");
                if data.len() + chunk.len() > data.capacity() {
                    println!("[upl] too much data! the client had an invalid content-length!");

                    // if we receive too much data, drop the buffer and stop using cache (it is still okay to use disk, probably)
                    data = BytesMut::new();
                    use_cache = false;
                } else {
                    data.put(chunk);
                }
            }
        }

        let mut cache = state.cache.lock().unwrap();

        if use_cache {
            println!("[upl] caching upload!!");
            cache.insert(name, data.freeze(), Some(Duration::from_secs(30)));
        }
    });

    Ok(url)
}
