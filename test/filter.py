import uuid


counter = 0


def server_filter_history(
    id: uuid.UUID, chunk: bytes, client_history: bytes, server_history: bytes
) -> bytes:
    # print("CLIENT", client_history)
    # print("SERVER", server_history)
    return chunk.replace(b"PING", b"PONG")


def client_filter(id: uuid.UUID, chunk: bytes) -> bytes:
    global counter
    if b"EVIL" in chunk:
        counter += 1
        print(f"flow {id} is evil (number {counter})")

    return chunk
