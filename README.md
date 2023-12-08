# breeze
breeze is a simple, performant file upload server.

The primary instance is https://picture.wtf.

## Features
Compared to the old Express.js backend, breeze has
- Streamed uploading
- Streamed downloading (on larger files)
- Upload caching
- Generally faster speeds overall

At this time, breeze does not support encrypted uploads on disk.

## Installation
I wrote breeze with the intention of running it in a container, but it runs just fine outside of one.

Either way, you need to start off by cloning the Git repository.
```bash
git clone https://git.min.rip/min/breeze.git
```

To run it in Docker, I recommend using Docker Compose. An example `docker-compose.yaml` configuration is below. You can start it using `docker compose up -d`.
```
version: '3.6'

services:
  breeze:
    build: ./breeze
    restart: unless-stopped

    volumes:
      - /srv/uploads:/data
      - ./breeze.toml:/etc/breeze.toml

    ports:
      - 8000:8000
```
For this configuration, it is expected that:
* there is a clone of the Git repository in the `./breeze` folder.
* there is a `breeze.toml` config file in current directory
* there is a directory at `/srv/uploads` for storing uploads

It can also be installed directly if you have the Rust toolchain installed:
```bash
cargo install --path .
```

## Usage
### Hosting
Configuration is read through a toml file.

By default it'll try to read `./breeze.toml`, but you can specify a different path using the `-c`/`--config` command line switch.

Here is an example config file:
```toml
[engine]
# The base URL that the HTTP server will be accessible on.
# This is used for formatting upload URLs.
# Setting it to "https://picture.wtf" would result in
#  upload urls of "https://picture.wtf/p/abcdef.png", etc.
base_url = "http://127.0.0.1:8000"

# The location that uploads will be saved to.
# It should be a path to a directory on disk that you can write to.
save_path = "/data"

# OPTIONAL - If set, the static key specified will be required to upload new files.
# If it is not set, no key will be required.
upload_key = "hiiiiiiii"

# OPTIONAL - specifies what to show when the site is visited on http
# It is sent with text/plain content type.
# There are two variables you can use:
#  %uplcount% - total number of uploads present on the server
#  %version%  - current breeze version (e.g. 0.1.5)
motd = "my image host, currently hosting %uplcount% files"

[engine.cache]
# The file size (in bytes) that a file must be under
# to get cached.
max_length = 134_217_728

# How long a cached upload will remain cached. (in seconds)
upload_lifetime = 1800

# How often the cache will be checked for expired uploads.
# It is not a continuous scan, and only is triggered upon a cache operation.
scan_freq = 60

# How much memory (in bytes) the cache is allowed to consume.
mem_capacity = 4_294_967_295

[http]
# The address that the HTTP server will listen on. (ip:port)
# Use 0.0.0.0 as the IP to listen publicly, 127.0.0.1 only lets your
# computer access it
listen_on = "127.0.0.1:8000"

[logger]
# OPTIONAL - the current log level.
# Default level is warn.
level = "warn"
```

### Uploading
The HTTP API is fairly simple, and it's pretty easy to make a ShareX configuration for it.

Uploads should be sent to `/new?name={original filename}` as a POST request. If the server uses upload keys, it should be sent to `/new?name={original filename}&key={upload key}`. The uploaded file's content should be sent as raw binary in the request body.

Here's an example ShareX configuration for it (with a key):
```json
{
  "Version": "14.1.0",
  "Name": "breeze example",
  "DestinationType": "ImageUploader, TextUploader, FileUploader",
  "RequestMethod": "POST",
  "RequestURL": "http://127.0.0.1:8000/new",
  "Parameters": {
    "name": "{filename}",
    "key": "hiiiiiiii"
  },
  "Body": "Binary"
}
```
