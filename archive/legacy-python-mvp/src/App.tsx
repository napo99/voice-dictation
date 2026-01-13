import { useState, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/tauri";
import { appWindow } from "@tauri-apps/api/window";
import { Mic, Check, Loader2, X } from "lucide-react";

function App() {
    const [status, setStatus] = useState("idle"); // idle, listening, processing, final_result
    const [transcript, setTranscript] = useState("");
    const ws = useRef<WebSocket | null>(null);

    useEffect(() => {
        // Connect to Python Engine
        ws.current = new WebSocket("ws://localhost:8765");

        ws.current.onopen = () => console.log("Connected to AI Engine");

        ws.current.onmessage = async (event) => {
            const msg = JSON.parse(event.data);
            if (msg.status === "listening") setStatus("listening");
            if (msg.status === "processing") setStatus("processing");
            if (msg.status === "final_result") {
                const text = msg.data.text;
                setTranscript(text);
                setStatus("success");

                // Inject into OS
                try {
                    await invoke("inject_text", { text: text + " " }); // Add space
                    setTimeout(() => setStatus("idle"), 1500);
                } catch (e) {
                    console.error(e);
                }
            }
        };

        return () => ws.current?.close();
    }, []);

    const toggleRecording = () => {
        if (status === "idle") {
            ws.current?.send(JSON.stringify({ action: "start" }));
            setStatus("listening");
        } else {
            ws.current?.send(JSON.stringify({ action: "stop" }));
            setStatus("processing");
        }
    };

    return (
        <div
            className="flex items-center gap-2 bg-black/80 backdrop-blur-md text-white p-2 rounded-full border border-gray-700 shadow-2xl transition-all select-none"
            data-tauri-drag-region
            style={{ fontFamily: "Inter, sans-serif" }}
        >
            {/* Draggable Handle */}
            <div className="w-2 h-8 bg-gray-600 rounded-full cursor-move" data-tauri-drag-region />

            {/* Status Icon */}
            <button
                onClick={toggleRecording}
                className={`p-3 rounded-full transition-all ${status === 'listening' ? 'bg-red-500 animate-pulse' : 'bg-blue-600 hover:bg-blue-500'}`}
            >
                {status === 'idle' && <Mic size={20} />}
                {status === 'listening' && <div className="w-5 h-5 bg-white rounded-sm" />}
                {status === 'processing' && <Loader2 className="animate-spin" size={20} />}
                {status === 'success' && <Check size={20} />}
            </button>

            {/* Transcript / Status Text */}
            <div className="w-48 overflow-hidden whitespace-nowrap text-sm text-gray-300 px-2">
                {status === 'idle' ? "Ready to dictate..." :
                    status === 'listening' ? "Listening..." :
                        status === 'processing' ? "Polishing..." :
                            transcript}
            </div>

            {/* Close App */}
            <button onClick={() => appWindow.close()} className="text-gray-500 hover:text-white p-1">
                <X size={16} />
            </button>
        </div>
    );
}

export default App;
