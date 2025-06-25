#!/bin/sh

# Generate the CA
openssl genpkey -algorithm RSA -out ca-key.pem
openssl req -new -x509 -key ca-key.pem -out ca-cert.pem -subj "/CN=MyCA"

# Generate cert and key
openssl genpkey -algorithm RSA -out key.pem

openssl req -new -key key.pem -subj "/CN=server.local" | \
	openssl x509 -req -CA ca-cert.pem -CAkey ca-key.pem \
	-CAcreateserial -out cert.pem -days 365
