# Build Stage
FROM ghcr.io/guilderia/base:latest AS builder
FROM debian:12 AS debian

# Bundle Stage
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /home/rust/src/target/release/revolt-pushd ./
COPY --from=debian /usr/bin/uname /usr/bin/uname

USER nonroot
CMD ["./guilderia-pushd"]