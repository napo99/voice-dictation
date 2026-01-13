import os
import queue
import time
import threading
import json
import numpy as np
import pyaudio
import webrtcvad
from faster_whisper import WhisperModel
import ollama

# --- Configuration ---
VAD_AGGRESSIVENESS = 3      # 3 is most aggressive (filters most non-speech)
FRAME_DURATION_MS = 30      # VAD frame size
SAMPLE_RATE = 16000         # Standard for Whisper
CHANNELS = 1
SILENCE_TIMEOUT = 0.5       # Seconds of silence to finish an utterance (Low latency!)
MODEL_SIZE = "base.en"      # fast and good enough
OLLAMA_MODEL = "llama3.2:1b"

class DictationEngine:
    def __init__(self, status_callback):
        self.status_callback = status_callback
        self.vad = webrtcvad.Vad(VAD_AGGRESSIVENESS)
        self.audio_queue = queue.Queue()
        self.is_recording = False
        self.model = WhisperModel(MODEL_SIZE, device="cpu", compute_type="int8") # Optimized
        self.p = pyaudio.PyAudio()
        self.status_callback("ready")

    def list_microphones(self):
        info = self.p.get_host_api_info_by_index(0)
        numdevices = info.get('deviceCount')
        mics = []
        for i in range(0, numdevices):
            if (self.p.get_device_info_by_host_api_device_index(0, i).get('maxInputChannels')) > 0:
                name = self.p.get_device_info_by_host_api_device_index(0, i).get('name')
                mics.append({"id": i, "name": name})
        return mics

    def start_recording(self, device_index=None):
        if self.is_recording: return
        self.is_recording = True
        
        try:
            self.stream = self.p.open(format=pyaudio.paInt16,
                                      channels=CHANNELS,
                                      rate=SAMPLE_RATE,
                                      input=True,
                                      input_device_index=device_index,
                                      frames_per_buffer=int(SAMPLE_RATE * FRAME_DURATION_MS / 1000))
        except Exception as e:
            self.status_callback("error", f"Mic Error: {str(e)}")
            self.is_recording = False
            return

        self.listen_thread = threading.Thread(target=self._listen_loop)
        self.listen_thread.start()
        self.status_callback("listening")

    def stop_recording(self):
        self.is_recording = False
        if hasattr(self, 'stream'):
            try:
                self.stream.stop_stream()
                self.stream.close()
            except: pass
        self.status_callback("idle")

    def _listen_loop(self):
        frames = []
        silence_start = None
        has_speech = False
        chunk_size = int(SAMPLE_RATE * FRAME_DURATION_MS / 1000)

        while self.is_recording:
            try:
                frame = self.stream.read(chunk_size, exception_on_overflow=False)
                is_speech = self.vad.is_speech(frame, SAMPLE_RATE)
                
                if is_speech:
                    has_speech = True
                    silence_start = None
                    frames.append(frame)
                    # self.status_callback("speech_active") # Optimization: Don't spam UI
                else:
                    if has_speech:
                        frames.append(frame)
                        if silence_start is None:
                            silence_start = time.time()
                        elif time.time() - silence_start > SILENCE_TIMEOUT:
                            # End of utterance
                            audio_data = b''.join(frames)
                            self._transcribe_and_process(audio_data)
                            
                            # Reset for next phrase (Continuous Mode)
                            frames = []
                            has_speech = False
                            silence_start = None
                            # If Toggle Mode: self.stop_recording()
            except Exception as e:
                print(f"Loop Error: {e}")
                break

    def _transcribe_and_process(self, audio_data):
        self.status_callback("processing")
        start_t = time.time()
        
        # Whisper Inference
        audio_np = np.frombuffer(audio_data, dtype=np.int16).astype(np.float32) / 32768.0
        segments, _ = self.model.transcribe(audio_np, beam_size=2) # efficient beam
        raw_text = " ".join([s.text for s in segments]).strip()
        
        if not raw_text:
            self.status_callback("listening") # False alarm
            return

        print(f"Whisper ({time.time()-start_t:.2f}s): {raw_text}")
        
        # Ollama Polishing
        polished = self._polish_with_ollama(raw_text)
        print(f"Ollama ({time.time()-start_t:.2f}s total): {polished}")
        
        # Send FINAL result to frontend
        self.status_callback("final_result", {"text": polished})
        self.status_callback("listening")

    def _polish_with_ollama(self, text):
        # Ultra-concise prompt for speed
        prompt = f"Fix grammar and remove filler words. Output ONLY the text: {text}"
        try:
            # stream=False for simple request
            response = ollama.generate(model=OLLAMA_MODEL, prompt=prompt)
            return response['response'].strip()
        except:
            return text
