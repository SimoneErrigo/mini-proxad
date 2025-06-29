import socket

HOST = "127.0.0.1"
PORT = 9000

payload = b"A" * 100_000

with socket.create_connection((HOST, PORT)) as sock:
    print(f"Connected to {HOST}:{PORT}, sending {len(payload)} bytes...")
    sock.sendall(payload)

    # sock.shutdown(socket.SHUT_WR)  # Optional: signal no more data will be sent

    received = b""
    expected_len = len(payload)

    while len(received) < expected_len:
        chunk = sock.recv(4096)
        if not chunk:
            break
        received += chunk

    print(f"Received {len(received)} bytes.")
    assert received == payload, "Received data does not match sent data!"

    print("Success! Closing connection.")
