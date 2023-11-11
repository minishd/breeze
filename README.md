# breeze
breeze is a simple, performant file upload server.

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

To run it in Docker, I recommend using Docker Compose. An example `docker-compose.yaml` configuration is below.
```
version: '3.6'

services:
  breeze:
    build: ./breeze
    restart: unless-stopped

    volumes:
      - /srv/uploads:/data

    ports:
      - 8000:8000

    environment:
      - BRZ_BASE_URL=http://127.0.0.1:8000
      - BRZ_SAVE_PATH=/data
      - BRZ_UPLOAD_KEY=hiiiiiiii
      - BRZ_CACHE_UPL_MAX_LENGTH=134217728 # allow files up to ~134 MiB to be cached
      - BRZ_CACHE_UPL_LIFETIME=1800 # let uploads stay in cache for 30 minutes
      - BRZ_CACHE_SCAN_FREQ=60 # scan the cache for expired files if more than 60 seconds have passed since the last scan
      - BRZ_CACHE_MEM_CAPACITY=4294967296 # allow 4 GiB of data to be in the cache at once
```
For this configuration, it is expected that there is a clone of the Git repository in the `./breeze` folder. You can start it using `docker compose up -d`.

It can also be installed directly if you have the Rust toolchain installed:
```bash
cargo install --path .
```

## Usage
### Hosting
Configuration is read through environment variables, because I wanted to run this using Docker Compose.
```
BRZ_BASE_URL - base url for upload urls (ex: http://127.0.0.1:8000 for http://127.0.0.1:8000/p/abcdef.png, http://picture.wtf for http://picture.wtf/p/abcdef.png)
BRZ_SAVE_PATH - this should be a path where uploads are saved to disk (ex: /srv/uploads, C:\brzuploads)
BRZ_UPLOAD_KEY (optional) - if not empty, the key you specify will be required to upload new files.
BRZ_CACHE_UPL_MAX_LENGTH - this is the max length an upload can be in bytes before it won't be cached (ex: 80000000 for 80MB)
BRZ_CACHE_UPL_LIFETIME - this indicates how long an upload will stay in cache (ex: 1800 for 30 minutes, 60 for 1 minute)
BRZ_CACHE_SCAN_FREQ - this is the frequency of full cache scans, which scan for and remove expired uploads (ex: 60 for 1 minute)
BRZ_CACHE_MEM_CAPACITY - this is the amount of memory the cache will hold before dropping entries
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
