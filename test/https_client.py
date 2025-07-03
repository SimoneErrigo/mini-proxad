import asyncio
import ssl
import time
from aiohttp import ClientSession, TCPConnector

URL = "https://localhost:9443"
CA_FILE = "./test/keys/ca-cert.pem"
CONCURRENCY = 400
REQUESTS = 1000
TICK_INTERVAL = 3

total_stats = {
    "success": 0,
    "fail": 0,
    "latencies": [],
}


async def fetch(session, url, stats):
    start = time.perf_counter()
    try:
        async with session.get(url) as response:
            await response.text()
            if response.status == 200:
                stats["success"] += 1
            else:
                stats["fail"] += 1
    except Exception:
        stats["fail"] += 1
    else:
        stats["latencies"].append(time.perf_counter() - start)


async def run_batch(session, sem, stats):
    tasks = [
        asyncio.create_task(fetch_with_semaphore(session, sem, stats))
        for _ in range(REQUESTS)
    ]
    await asyncio.gather(*tasks)


async def fetch_with_semaphore(session, sem, stats):
    async with sem:
        await fetch(session, URL, stats)


def print_tick_stats(stats, tick_num, duration):
    latencies = stats["latencies"]
    success = stats["success"]
    fail = stats["fail"]

    print(f"\n[Tick {tick_num}] Duration: {duration:.2f} sec")
    print(f"  Requests:          {success + fail}")
    print(f"  Successful:        {success}")
    print(f"  Failed:            {fail}")
    if latencies:
        print(f"  Min latency:       {min(latencies):.4f} sec")
        print(f"  Max latency:       {max(latencies):.4f} sec")
        print(f"  Avg latency:       {sum(latencies) / len(latencies):.4f} sec")
        print(f"  Requests/sec:      {success / duration:.2f}")
    else:
        print("  No successful requests in this tick.")


async def benchmark_loop():
    ssl_context = ssl.SSLContext(ssl.PROTOCOL_TLS_CLIENT)
    ssl_context.load_verify_locations(cafile=CA_FILE)
    connector = TCPConnector(ssl=ssl_context, limit=CONCURRENCY)
    sem = asyncio.Semaphore(CONCURRENCY)

    async with ClientSession(connector=connector) as session:
        tick_num = 1
        while True:
            stats = {"success": 0, "fail": 0, "latencies": []}
            start = time.perf_counter()
            await run_batch(session, sem, stats)
            duration = time.perf_counter() - start

            # Update total stats
            total_stats["success"] += stats["success"]
            total_stats["fail"] += stats["fail"]
            total_stats["latencies"].extend(stats["latencies"])

            print_tick_stats(stats, tick_num, duration)

            tick_num += 1
            await asyncio.sleep(TICK_INTERVAL)


if __name__ == "__main__":
    try:
        asyncio.run(benchmark_loop())
    except KeyboardInterrupt:
        print("\nInterrupted by user.")
        if total_stats["latencies"]:
            print("\n=== Total Benchmark Summary ===")
            print(
                f"\t\tTotal Requests:\t\t\t{total_stats['success'] + total_stats['fail']}"
            )
            print(f"\t\tSuccessful:\t\t\t{total_stats['success']}")
            print(f"\t\tFailed:\t\t\t{total_stats['fail']}")
            print(
                f"\t\tAvg Latency:\t\t\t{sum(total_stats['latencies']) / len(total_stats['latencies']):.4f} sec"
            )
        else:
            print("No successful requests recorded.")
