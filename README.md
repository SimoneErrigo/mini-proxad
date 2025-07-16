# mini-proxad

A small and lightweight proxy tailored for A/D events or general network analysis.

## How to run

The only dependency of this self contained application is the python interpreter (libpython.so). 

```
cargo build --release
```

Then, to run the proxy you need to provide a configuration file.

```
./mini-proxad --config config.yml
```

## Example config

```yml
# Service configuration
service_name: Service_123

# The port and ip that the proxy will expose publicly
# aliases: client_ip, client_port
from_ip: 0.0.0.0
from_port: 9443

from_max_history: 10Mib
from_timeout: 60s

# The port and ip for the internal service
# aliases: server_ip, server_port
to_ip: 127.0.0.1
to_port: 8443

to_max_history: 100Mib
to_timeout: 60s

# The path to the python filter
# This will be automatically reloaded
script_path: ./test/filter.py

# TLS configuration
# If the tls_ca_file is present the proxy will authenticate the service
tls_enabled: false
tls_cert_file: ./test/keys/cert.pem
tls_key_file: ./test/keys/key.pem
#tls_ca_file: ./test/keys/ca-cert.pem

# Pcap dumper configuration
# The dump_format acceps the following format values
#   - {service} will be replaced with the service name
#   - {timestamp} will be replaced with the time of the dump
#   - {client_ip} {from_ip} will be replaced with the client_ip
#   - {server_ip} {to_ip} will be replaced with the server_ip
#
# The current pcap file will be dumped every dump_interval or
# when dump_max_packets is reached
dump_enabled: false
dump_path: "./pcaps"
dump_format: "{service}_{server_port}_{timestamp}.pcap"
dump_interval: 30s
dump_max_packets: 256

# Http configuration
# When enabled the requests and responses are parsed by the proxy.
# A different filter is called with the HttpResponse and HttpRequest.
#
# Keep alive will allow the proxy to reuse the connection to serve
# multiple responses/requests. If enabled dumps will be delayed!
#
# Requests (or responses!) with a body greater then the max body size
# will automatically kill the flow
http_enabled: false
http_keep_alive: false
http_half_close: true
http_date_header: false
http_max_body: 20MB
```

## Known issues

If the `http_keep_alive` option is not set accordingly to the upstream service, connections will become unrealiable.

Right now the only the body is counted in history limitations when in HTTP mode.
