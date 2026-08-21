FROM rust:1.97.1-bookworm AS builder
WORKDIR /app
COPY . .
RUN apt-get update \
    && apt-get install -y --no-install-recommends binaryen clang cmake git pkg-config python3 \
    && rustup target add wasm32-unknown-unknown \
    && locked_wasm_bindgen=$(grep -m1 'name = "wasm-bindgen"' -A2 Cargo.lock | grep 'version = ' | cut -d'"' -f2) \
    && cargo install wasm-bindgen-cli --version "$locked_wasm_bindgen" --locked \
    && PWMTF_BUILD_ID=production ./scripts/build-wasm.sh \
    && cargo build --locked --release -p pwmtf_server --bin pwmtf-server

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --create-home pwmtf \
    && mkdir -p /app/dist /data \
    && chown -R pwmtf:pwmtf /app /data
COPY --from=builder /app/target/release/pwmtf-server /usr/local/bin/pwmtf-server
COPY --from=builder /app/dist /app/dist
USER pwmtf
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/pwmtf-server"]
