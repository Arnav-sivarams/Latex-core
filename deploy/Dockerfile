FROM rust:1.97-bookworm AS build
WORKDIR /src
COPY . .
# The repository's development toolchain tracks stable. Pin image builds to the
# compiler already present in this tagged base so `docker compose up --build`
# does not try to download a newer toolchain during a release deployment.
RUN cargo +1.97.0 build --release -p server --bins

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates docker.io && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/latex-core-api /usr/local/bin/latex-core-api
COPY --from=build /src/target/release/latex-core-worker /usr/local/bin/latex-core-worker
COPY --from=build /src/target/release/latex-core-doctor /usr/local/bin/latex-core-doctor
COPY --from=build /src/target/release/latex-core-admin /usr/local/bin/latex-core-admin
