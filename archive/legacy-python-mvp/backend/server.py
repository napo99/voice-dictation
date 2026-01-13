import asyncio
import websockets
import json
import logging
from engine import DictationEngine

# Configure logging
logging.basicConfig(level=logging.INFO)

# Global engine instance
engine = None
connected_clients = set()

async def broadcast_status(status, data=None):
    if not connected_clients: return
    message = json.dumps({"status": status, "data": data})
    tasks = [client.send(message) for client in connected_clients]
    await asyncio.gather(*tasks)

def engine_callback(status, data=None):
    # This runs in a thread, so we need to bridge to asyncio
    asyncio.run(broadcast_status(status, data))

async def handler(websocket):
    global engine
    connected_clients.add(websocket)
    logging.info("Client connected")
    
    # Init engine if not needed
    if engine is None:
        engine = DictationEngine(engine_callback)

    try:
        async for message in websocket:
            cmd = json.loads(message)
            action = cmd.get("action")
            
            if action == "start":
                mic_id = cmd.get("mic_id")
                logging.info(f"Starting recording on mic {mic_id}")
                engine.start_recording(mic_id)
            
            elif action == "stop":
                logging.info("Stopping recording")
                engine.stop_recording()
            
            elif action == "get_mics":
                mics = engine.list_microphones()
                await websocket.send(json.dumps({"status": "mics_list", "data": mics}))

    except websockets.ConnectionClosed:
        logging.info("Client disconnected")
    finally:
        connected_clients.remove(websocket)

async def main():
    async with websockets.serve(handler, "localhost", 8765):
        logging.info("Server started on ws://localhost:8765")
        await asyncio.Future()  # run forever

if __name__ == "__main__":
    asyncio.run(main())
