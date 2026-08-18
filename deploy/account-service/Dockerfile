# anycode cloud account API + Account Portal SPA.
# Desktop installers live on cluster MinIO (bucket `downloads`), not in this image.
# TLS terminated by K8s Ingress.
#
# Secrets at runtime only (K8s Secret / env — never baked into image):
#   - DATABASE_URL, WECHAT_PAY_API_V3_KEY, WECHAT_PAY_APP_ID, WECHAT_PAY_MCH_ID, ...
#   - PEM files mounted at /app/wechat-certs/ (see k8s/deployment.yaml)

FROM node:22-bookworm AS portal
# Layout mirrors repo so prepare-user-docs + @anycode/site-urls resolve.
WORKDIR /src
COPY scripts/prepare-user-docs.mjs /src/scripts/prepare-user-docs.mjs
COPY docs/user /src/docs/user
COPY brand /src/brand
RUN test -f /src/brand/anycode-mark.svg && test -f /src/brand/site-urls.ts
COPY crates/account-portal/package.json crates/account-portal/package-lock.json /src/crates/account-portal/
WORKDIR /src/crates/account-portal
RUN npm ci
COPY crates/account-portal/ ./
RUN npm run build \
    && rm -rf dist/downloads \
    && echo "portal downloads stripped; installers are on MinIO"

FROM rust:1-bookworm AS rust
WORKDIR /build
# account-service is a standalone crate (excluded from the public workspace).
COPY crates/account-service/Cargo.toml crates/account-service/Cargo.lock* ./
COPY crates/account-service/src ./src
COPY crates/account-service/migrations ./migrations
RUN sed -i '/^\[workspace\]/d' Cargo.toml \
    && cargo build --release

FROM debian:bookworm-slim

# Copy CA bundle from the rust stage instead of apt-get. ACR builders in CN
# often cannot reach deb.debian.org (empty index → no ca-certificates).
COPY --from=rust /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN mkdir -p /app/wechat-certs \
    && chmod 755 /app/wechat-certs

COPY --from=rust /build/target/release/anycode-account /usr/local/bin/anycode-account
COPY --from=portal /src/crates/account-portal/dist /app/portal
RUN chmod -R a+rX /app/portal

ENV ACCOUNT_SERVICE_HOST=0.0.0.0 \
    ACCOUNT_SERVICE_PORT=8080 \
    ACCOUNT_PORTAL_DIR=/app/portal \
    CORS_ORIGINS=https://anycode.work,http://anycode.work \
    DEFAULT_PAYMENT_PROVIDER=wechat \
    WECHAT_PAY_PRIVATE_KEY_PATH=/app/wechat-certs/apiclient_key.pem \
    WECHAT_PAY_PUBLIC_KEY_PATH=/app/wechat-certs/pub_key.pem \
    WECHAT_PRICE_CLOUD_5H_CENTS=4900 \
    WECHAT_PRICE_PRO_MONTHLY_CENTS=19900 \
    WECHAT_PRICE_TEAM_MONTHLY_CENTS=69900

EXPOSE 8080

USER nobody
CMD ["anycode-account"]
