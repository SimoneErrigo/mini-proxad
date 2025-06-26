import asyncio
import random
import time

HOST = "0.0.0.0"
PORT = 9000

CONCURRENT_CONNECTIONS = 1000
MESSAGES_PER_CONNECTION = 4
MAX_MSG_SIZE = 1024
TICK_INTERVAL = 4


async def tcp_echo_client(client_id: int):
    start = time.perf_counter()
    reader, writer = await asyncio.open_connection(HOST, PORT)
    total_sent = 0
    total_received = 0

    for _ in range(MESSAGES_PER_CONNECTION):
        size = random.randint(1, MAX_MSG_SIZE)
        data = (
            random.randbytes(size)
            if hasattr(random, "randbytes")
            else bytes(random.getrandbits(8) for _ in range(size))
        )

        writer.write(data)
        await writer.drain()
        total_sent += len(data)

        resp = await reader.readexactly(len(data))
        total_received += len(resp)

        assert resp == data, f"Echo mismatch on client {client_id}"

    writer.close()
    await writer.wait_closed()
    end = time.perf_counter()
    duration = end - start
    return total_sent, total_received, duration


async def run_tick():
    tasks = [
        asyncio.create_task(tcp_echo_client(i)) for i in range(CONCURRENT_CONNECTIONS)
    ]
    results = await asyncio.gather(*tasks)

    total_sent = sum(r[0] for r in results)
    total_received = sum(r[1] for r in results)
    durations = [r[2] for r in results]

    min_time = min(durations)
    max_time = max(durations)
    avg_time = sum(durations) / len(durations)

    return total_sent, total_received, min_time, max_time, avg_time


async def main_loop():
    while True:
        start_time = time.time()
        sent, received, min_t, max_t, avg_t = await run_tick()
        elapsed = time.time() - start_time

        print(
            f"""Tick finished: Sent {sent} bytes, Received {received} bytes in {elapsed:.2f}s
Throughput: {sent / elapsed / 1024:.2f} KB/s
Request time: min={min_t:.3f}s max={max_t:.3f}s avg={avg_t:.3f}s
"""
        )

        await asyncio.sleep(max(0, TICK_INTERVAL - elapsed))


if __name__ == "__main__":
    asyncio.run(main_loop())
