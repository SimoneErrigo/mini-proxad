#!/bin/sh

cargo run -- \
	--service-name test \
	--client-port 9000 \
	--server-port 8000 \
	--verbose
