#!/bin/bash

set -euxo pipefail

f="$(mktemp)"

node index.js > $f &
sleep 1
pid="$!"

port="$(cat $f)"

cargo run --example ws_connect -- ws://localhost:$port/socket.io/

code=$?

kill -INT $pid

exit $code
