#!/bin/bash

docker build -t reth-build --file Dockerfile.build . || exit 1
docker run --rm -v "$(pwd)":/app reth-build cargo build
