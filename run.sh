#!/bin/sh

cd "$(dirname "$0")"

. ./.env
export RUST_LOG=sandy
export SID="$SID"

cargo r
