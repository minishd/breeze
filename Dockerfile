# builder
FROM rust:1.73 as builder

WORKDIR /usr/src/breeze
COPY . .
RUN cargo install --path .

# runner
FROM debian:bullseye-slim

RUN apt-get update && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/cargo/bin/breeze /usr/local/bin/breeze

RUN useradd -m runner
USER runner

EXPOSE 8000
CMD [ "breeze" ]
