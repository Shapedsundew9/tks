FROM mcr.microsoft.com/devcontainers/rust:latest

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
    build-essential \
    libssl-dev \
    lldb \
    pkg-config \
    ripgrep \
    vim \
    xclip \
    && rm -rf /var/lib/apt/lists/*
