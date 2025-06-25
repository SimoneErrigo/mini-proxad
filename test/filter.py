def server_filter(chunk: bytes) -> bytes:
    return chunk.replace(b"PING", b"PONG")

def client_filter(chunk: bytes) -> bytes:
    if b"EVIL" in chunk:
        print("EVIL CONNECTION")
    return chunk
