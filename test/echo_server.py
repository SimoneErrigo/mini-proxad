import asyncio

HOST = "0.0.0.0"
PORT = 8000

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
    server = await asyncio.start_server(handle_echo, HOST, PORT)
    addr = server.sockets[0].getsockname()
    print(f"Serving on {addr}")

    async with server:
        await server.serve_forever()

if __name__ == '__main__':
    asyncio.run(main())
