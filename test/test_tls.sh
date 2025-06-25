#!/bin/sh

cargo run -- \
	--service-name test \
	--client-port 9443 \
	--server-port 8443 \
	--tls \
	--tls-cert-file ./test/cert.pem \
	--tls-key-file ./test/key.pem
