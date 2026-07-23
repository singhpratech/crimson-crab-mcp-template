# syntax=docker/dockerfile:1

# ---- Build stage ---------------------------------------------------------
FROM rust:1-slim-bookworm AS builder

WORKDIR /app

# Build dependencies first so they cache independently of source changes.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs \
    && cargo build --release \
    && rm -rf src

# Build the real binary.
COPY src ./src
RUN touch src/main.rs && cargo build --release

# ---- Runtime stage -------------------------------------------------------
FROM debian:bookworm-slim AS runtime

# TLS roots for outbound HTTPS to the Anthropic API at run time.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /app/target/release/crimson-crab-mcp-template /usr/local/bin/crimson-crab-mcp-template

# The MCP server reads ANTHROPIC_API_KEY from the environment at startup.
# A placeholder is baked in so the server can boot and answer MCP introspection
# (tool listing) without a real key — introspection never calls Claude.
# Real deployments MUST override this with a valid key, e.g.:
#   docker run -e ANTHROPIC_API_KEY=sk-ant-... <image>
ENV ANTHROPIC_API_KEY=sk-ant-placeholder-override-at-runtime

# MCP speaks over stdio: stdout is the protocol channel, logs go to stderr.
ENTRYPOINT ["crimson-crab-mcp-template"]
