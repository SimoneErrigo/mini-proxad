import asyncio
from aiohttp import web
import ssl

PORT = 8443


async def handle(request):
    return web.Response(text="Hello, world!", content_type="text/plain")


async def main():
    app = web.Application()
    app.router.add_get("/", handle)

    context = ssl.create_default_context(ssl.Purpose.CLIENT_AUTH)
    context.load_cert_chain(certfile="test/keys/cert.pem", keyfile="test/keys/key.pem")
    context.load_verify_locations(cafile="test/keys/ca-cert.pem")
    context.verify_mode = ssl.CERT_OPTIONAL
    context.set_alpn_protocols(["http/1.1"])

    runner = web.AppRunner(app)
    await runner.setup()
    site = web.TCPSite(runner, "0.0.0.0", PORT, ssl_context=context)
    print(f"Serving HTTPS (HTTP/1.1) on port {PORT}")
    await site.start()

    while True:
        await asyncio.sleep(3600)


if __name__ == "__main__":
    asyncio.run(main())
