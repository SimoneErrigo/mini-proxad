#!/bin/sh

cargo run -- \
	--service-name test \
	--client-port 9443 \
	--server-port 8443 \
	--tls \
	--tls-cert-file ./test/server.crt \
	--tls-key-file ./test/server.key
