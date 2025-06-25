#!/bin/sh

ROOT="${ROOT:-.}"
DIR="$ROOT/test/keys"
CONF="$DIR/cert.conf"

openssl genpkey -algorithm RSA -out $DIR/ca-key.pem

openssl req -x509 -new -key $DIR/ca-key.pem -out $DIR/ca-cert.pem -days 3650 -subj "/CN=MyCA"

openssl genpkey -algorithm RSA -out $DIR/key.pem

openssl req -new -key $DIR/key.pem -out $DIR/csr.pem -config $CONF

openssl x509 -req -in $DIR/csr.pem -CA $DIR/ca-cert.pem -CAkey $DIR/ca-key.pem \
	-set_serial 01 -out $DIR/cert.pem -days 365 -extfile $CONF -extensions req_ext

openssl x509 -in $DIR/cert.pem -noout -text | grep -A3 "Subject Alternative Name"
