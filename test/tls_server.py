import asyncio
import ssl

HOST = "0.0.0.0"
PORT = 8443

CERT_FILE = "./test/server.crt"
KEY_FILE = "./test/server.key"

async def handle_echo(reader: asyncio.StreamReader, writer: asyncio.StreamWriter):
    addr = writer.get_extra_info('peername')
    print(f"Connection from {addr}")

    try:
        while True:
            data = await reader.read(4096)
            if not data:
                print(f"Connection closed by {addr}")
                break

            writer.write(data)
            await writer.drain()

    except asyncio.CancelledError:
        pass
    except Exception as e:
        print(f"Error with {addr}: {e}")
    finally:
        writer.close()
        await writer.wait_closed()
        print(f"Connection with {addr} closed")

async def main():
    ssl_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ssl_context.load_cert_chain(certfile=CERT_FILE, keyfile=KEY_FILE)

    server = await asyncio.start_server(handle_echo, HOST, PORT, ssl=ssl_context)
    addr = server.sockets[0].getsockname()
    print(f"Serving on {addr}")

    async with server:
        await server.serve_forever()

if __name__ == '__main__':
    asyncio.run(main())
