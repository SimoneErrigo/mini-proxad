#!/bin/sh

openssl req -x509 \
    -newkey rsa:4096 \
    -keyout ./test/key.pem \
    -out ./test/cert.pem \
    -sha256 \
    -days 2 \
    -nodes \
    -subj "/C=XX/ST=Italy/L=Rome/O=localhost/OU=AttackDefence/CN=127.0.0.1"

