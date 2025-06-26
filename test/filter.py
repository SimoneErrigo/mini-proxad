counter = 0


def server_filter(chunk: bytes) -> bytes:
    return chunk.replace(b"PING", b"PONG")


def client_filter(chunk: bytes) -> bytes:
    global counter
    if b"EVIL" in chunk:
        counter += 1
        print("Evil message number", counter)
    return chunk
