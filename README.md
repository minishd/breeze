# breeze

breeze is a simple, performant file upload server.

The primary instance is https://picture.wtf.

## Features

- Basic upload API tailored towards ShareX
- Streamed uploading
- Streamed downloading (on larger files)
- Pause/continue download support with `Range` header
- Upload caching in memory
- Support for ShareX file deletion URLs
- Temporary uploads
- Automatic exif data removal

## Installation

On picture.wtf, breeze is ran with the NixOS module provided by `flake.nix`. [Take a look at the config](https://git.min.rip/min/infra/src/branch/main/nixos/hosts/silver/services/breeze.nix) if you want!

Containerised and bare-metal deployments are also supported. Instructions for those are below.

To begin, clone the Git repository:

```bash
git clone https://git.min.rip/min/breeze.git
```

If you want to run it as a Docker container, here is an example `docker-compose.yaml` that may be useful for reference.

```
version: '3.6'

services:
  breeze:
    build: ./breeze
    restart: unless-stopped

    volumes:
      - /srv/uploads:/data
      - ./breeze.toml:/etc/breeze.toml

    user: 1000:1000

    ports:
      - 8383:8000
```

With this configuration, it is expected that:

- there is a clone of the Git repository in the `./breeze` folder
- there is a `breeze.toml` config file in current directory
- there is a directory at `/srv/uploads` for storing uploads
- port 8383 will be made accessible to the Internet somehow (either forwarding the port through your firewall directly, or passing it through a reverse proxy)
- you want the uploads to be owned by the user on your system with id 1000. (this is usually your user)

It can also be installed directly if the Rust toolchain is installed:

```bash
cd breeze
cargo install --path .

# then, you can run w/ a path to your `breeze.toml` config file
breeze --config /path/to/breeze.toml
```

### Exposing publicly

If you want to expose a breeze server to the internet, I highly recommend using a reverse proxy instead of just forwarding its HTTP port.

Caddy is probably the easiest to set up if you are new to reverse proxies. Here is an example `Caddyfile` for the Docker Compose file above (assuming `yourdomain.com` is a domain that points to your server's IP).

```
yourdomain.com {
  # enable compression
  encode

  # forward request to breeze
  reverse_proxy 127.0.0.1:8383
}
```

## Usage

### Hosting

Configuration is read through a toml file.

The config file path is specified using the `-c`/`--config` command line switch.

Here is an example config file:

```toml
[engine]
# The base URL that the HTTP server will be accessible on.
# This is used for formatting upload URLs.
# Setting it to "https://picture.wtf" would result in
#  upload urls of "https://picture.wtf/p/abcdef.png", etc.
base_url = "http://127.0.0.1:8000"

# OPTIONAL - If set, the static key specified will be required to upload new files.
# If it is not set, no key will be required.
upload_key = "hiiiiiiii"

# OPTIONAL - specifies what to show when the site is visited on http
# It is sent with text/plain content type.
# There are two variables you can use:
#  %uplcount% - total number of uploads present on the server
#  %version%  - current breeze version (e.g. 0.1.5)
motd = "my image host, currently hosting %uplcount% files"

# The maximum lifetime a temporary upload may be given, in seconds.
# It's okay to leave this somewhat high because large temporary uploads
# will just be bumped out of the cache when a new upload needs to be
# cached anyways.
max_temp_lifetime = 43200

# OPTIONAL - the maximum length (in bytes) a file being uploaded may be.
# A word of warning about this: the error shown to ShareX users who
# hit the limit is *not* very clear. ("connection closed" or similar)
max_upload_len = 2_147_483_648

# The maximum length (in bytes) an image file may be before the server
# will skip removing its EXIF data.
# The performance impact of breeze's EXIF data removal is not
# very high in everyday usage, so something like 16MiB is reasonable.
max_strip_len = 16_777_216

[engine.disk]
# The location that uploads will be saved to.
# It should be a path to a directory on disk that you can write to.
save_path = "/data"

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
mem_capacity = 4_294_967_296

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

The HTTP API is pretty simple, and it's easy to make a ShareX configuration for it.

Uploads should be sent to `/new?name={original filename}` as a POST request. If the server uses upload keys, it should be sent to `/new?name={original filename}&key={upload key}`. The uploaded file's content should be sent as raw binary in the request body.

Also you can specify `&lastfor={time in seconds}` to make your upload temporary, or `&keepexif=true` to tell the server not to clear EXIF data on image uploads. (if you don't know what EXIF data is, you can leave it as default. you'll know if you need it)

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
