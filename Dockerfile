# syntax=docker/dockerfile:1.7

FROM node:24-bookworm-slim AS frontend
WORKDIR /app
RUN npm install --global pnpm@12.11.2
COPY package.json pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY index.html tsconfig.json vite.config.ts ./
COPY src ./src
RUN pnpm build

# Ubuntu 24.04 can run the Debian Bookworm Node binary (glibc >= 2.36).
FROM ubuntu:24.04 AS linux-builder
ENV DEBIAN_FRONTEND=noninteractive \
    CARGO_HOME=/root/.cargo \
    RUSTUP_HOME=/root/.rustup \
    PATH=/root/.cargo/bin:/usr/local/bin:$PATH \
    APPIMAGE_EXTRACT_AND_RUN=1
RUN apt-get update && apt-get install --yes --no-install-recommends \
    build-essential \
    ca-certificates \
    curl \
    desktop-file-utils \
    file \
    libayatana-appindicator3-dev \
    librsvg2-dev \
    libssl-dev \
    libwebkit2gtk-4.1-dev \
    openssl \
    patchelf \
    pkg-config \
    wget \
    xdg-utils \
    && rm -rf /var/lib/apt/lists/*
COPY --from=frontend /usr/local/ /usr/local/
RUN curl --proto '=https' --tlsv1.2 --fail --show-error --silent \
    https://sh.rustup.rs -o /tmp/rustup-init.sh \
    && sh /tmp/rustup-init.sh -y --profile minimal --default-toolchain stable \
    && rm /tmp/rustup-init.sh
WORKDIR /app
COPY . .
COPY --from=frontend /app/node_modules ./node_modules
COPY --from=frontend /app/dist ./dist
# Copy the installer out of the target cache in the same layer that builds it.
RUN --mount=type=cache,id=schematadb-linux-amd64-registry,target=/root/.cargo/registry \
    --mount=type=cache,id=schematadb-linux-amd64-git,target=/root/.cargo/git \
    --mount=type=cache,id=schematadb-linux-amd64-target,target=/app/target,sharing=locked \
    rm -rf target/release/bundle/appimage \
    && pnpm exec tauri build --bundles appimage \
      --config '{"build":{"beforeBuildCommand":""}}' -- --locked \
    && mkdir /out \
    && cp target/release/bundle/appimage/*.AppImage /out/

FROM scratch AS linux
COPY --from=linux-builder /out/ /

# OSXCross supplies the Apple SDK and linker; all build tools run on Linux x86_64.
FROM joseluisq/rust-linux-darwin-builder:2.0.0-beta.1 AS mac-builder
RUN rustup toolchain install stable --profile minimal \
    && rustup default stable \
    && rustup target add aarch64-apple-darwin
ENV MACOSX_DEPLOYMENT_TARGET=11.0 \
    CC_aarch64_apple_darwin=arm64e-apple-darwin22.4-clang \
    CXX_aarch64_apple_darwin=arm64e-apple-darwin22.4-clang++ \
    AR_aarch64_apple_darwin=arm64e-apple-darwin22.4-ar \
    CFLAGS_aarch64_apple_darwin="-arch arm64" \
    CXXFLAGS_aarch64_apple_darwin="-arch arm64"
WORKDIR /app
COPY . .
COPY --from=frontend /app/dist ./dist
RUN --mount=type=cache,id=schematadb-mac-registry,target=/root/.cargo/registry \
    --mount=type=cache,id=schematadb-mac-git,target=/root/.cargo/git \
    --mount=type=cache,id=schematadb-mac-target,target=/app/target,sharing=locked \
    cargo build --release --locked --package schematadb \
      --target aarch64-apple-darwin --features tauri/custom-protocol \
    && mkdir -p /artifacts \
    && cargo metadata --locked --no-deps --format-version 1 > /artifacts/cargo-metadata.json \
    && cp target/aarch64-apple-darwin/release/schematadb /artifacts/

FROM frontend AS mac-package
RUN apt-get update && apt-get install --yes --no-install-recommends \
    build-essential ca-certificates curl cpio file libarchive-tools \
    && rm -rf /var/lib/apt/lists/*
# Pin the BOM writer; it is a build-only tool, not shipped in the application.
RUN curl --fail --location --silent --show-error \
      https://codeload.github.com/hogliux/bomutils/tar.gz/3f7dc2dbbc36ca1c957ec629970026f45594a52c \
      -o /tmp/bomutils.tar.gz \
    && mkdir /tmp/bomutils \
    && tar -xzf /tmp/bomutils.tar.gz -C /tmp/bomutils --strip-components=1 \
    && make -C /tmp/bomutils CXXFLAGS="-Wall -std=c++11" \
    && cp /tmp/bomutils/build/bin/mkbom /usr/local/bin/ \
    && cp /tmp/bomutils/build/bin/lsbom /usr/local/bin/ \
    && rm -rf /tmp/bomutils /tmp/bomutils.tar.gz
RUN curl --fail --location --silent --show-error \
      https://github.com/indygreg/apple-platform-rs/releases/download/apple-codesign/0.29.0/apple-codesign-0.29.0-x86_64-unknown-linux-musl.tar.gz \
      -o /tmp/rcodesign.tar.gz \
    && curl --fail --location --silent --show-error \
      https://github.com/indygreg/apple-platform-rs/releases/download/apple-codesign/0.29.0/apple-codesign-0.29.0-x86_64-unknown-linux-musl.tar.gz.sha256 \
      -o /tmp/rcodesign.sha256 \
    && echo "$(cat /tmp/rcodesign.sha256)  /tmp/rcodesign.tar.gz" | sha256sum --check \
    && tar -xzf /tmp/rcodesign.tar.gz -C /usr/local/bin --strip-components=1 \
    && rm /tmp/rcodesign.tar.gz /tmp/rcodesign.sha256
COPY scripts/package-macos.mjs ./scripts/package-macos.mjs
COPY src-tauri/tauri.conf.json ./src-tauri/tauri.conf.json
COPY src-tauri/icons/icon.icns ./src-tauri/icons/icon.icns
COPY --from=mac-builder /artifacts/schematadb ./target/aarch64-apple-darwin/release/schematadb
COPY --from=mac-builder /artifacts/cargo-metadata.json ./target/cargo-metadata.json
RUN file target/aarch64-apple-darwin/release/schematadb \
    && node scripts/package-macos.mjs

FROM scratch AS mac
COPY --from=mac-package /out/ /
