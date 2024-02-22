# builder
FROM rust:1.74 as builder

WORKDIR /usr/src/breeze
COPY . .
RUN cargo install --path .

# runner
FROM debian:bookworm-slim

RUN apt-get update && rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/cargo/bin/breeze /usr/local/bin/breeze

CMD [ "breeze", "--config", "/etc/breeze.toml" ]
