FROM rust:1.67.0 as builder

WORKDIR /usr/src/breeze
COPY . .
RUN [ "cargo", "install", "--path",  "." ]

FROM debian:bullseye-slim
COPY --from=builder /usr/local/cargo/bin/breeze /usr/local/bin/breeze

ENTRYPOINT [ "breeze" ]