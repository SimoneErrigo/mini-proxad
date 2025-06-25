import asyncio
import ssl

HOST = "127.0.0.1"
PORT = 8443

CERT_FILE = "./test/keys/cert.pem"
KEY_FILE = "./test/keys/key.pem"
CA_FILE = "./test/keys/ca-cert.pem"

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
    ssl_context.load_verify_locations(cafile=CA_FILE)
    #ssl_context.verify_mode = ssl.CERT_OPTIONAL

    server = await asyncio.start_server(handle_echo, HOST, PORT, ssl=ssl_context)
    addr = server.sockets[0].getsockname()
    print(f"Serving on {addr}")

    async with server:
        await server.serve_forever()

if __name__ == '__main__':
    asyncio.run(main())
