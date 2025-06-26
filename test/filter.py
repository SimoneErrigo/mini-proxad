import uuid
import sys
import types


def init_state(m):
    m.counter = 0


try:
    import persist

    print("Imported persistent state")

except ImportError:
    persist = types.ModuleType("persist")
    sys.modules["persist"] = persist
    print("Stored persistent state")
    init_state(persist)


def server_filter_history(
    id: uuid.UUID, chunk: bytes, client_history: bytes, server_history: bytes
) -> bytes:
    # print("CLIENT", client_history)
    # print("SERVER", server_history)
    return chunk.replace(b"PING", b"PONG")


def client_filter(id: uuid.UUID, chunk: bytes) -> bytes:
    global counter
    if b"EVIL" in chunk:
        persist.counter += 1
        print(f"flow {id} is evil (number {persist.counter})")

    return chunk
