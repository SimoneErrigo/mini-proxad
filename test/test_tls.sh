#!/bin/sh

cargo run -- \
	--service-name test \
	--client-port 9443 \
	--server-port 8443 \
	--tls \
	--tls-cert-file ./test/keys/cert.pem \
	--tls-key-file ./test/keys/key.pem \
	--tls-ca-file ./test/keys/ca-cert.pem \
	--python-script ./test/filter.py
