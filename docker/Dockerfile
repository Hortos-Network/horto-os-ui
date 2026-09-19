# Multi-stage: build status-api only, run as nonroot on distroless.
FROM rust:slim-trixie AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    ca-certificates \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY . .

RUN cargo build --release -p horto-os-ui-status-api

FROM gcr.io/distroless/cc-debian13:nonroot AS runtime

WORKDIR /app

COPY --from=builder /app/target/release/horto-os-ui-status-api /app/horto-os-ui-status-api
COPY --from=builder /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt

ENV SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt
ENV HORTO_API_BIND=0.0.0.0:8787

EXPOSE 8787

USER nonroot:nonroot

CMD ["/app/horto-os-ui-status-api"]
