FROM mcr.microsoft.com/devcontainers/rust:latest

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
    build-essential \
    clang \
    cmake \
    libclang-dev \
    libpq-dev \
    libssl-dev \
    lldb \
    pkg-config \
    postgresql-client \
    ripgrep \
    vim \
    xclip \
    && rm -rf /var/lib/apt/lists/*
