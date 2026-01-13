import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { getCurrentWindow, PhysicalPosition } from '@tauri-apps/api/window';
import './App.css';

// ============================================================================
// Types
// ============================================================================

interface PillStatus {
  state: string;
  transcript: string;
  is_recording: boolean;
  audio_level: number;
}

interface PipelineEvent {
  StateChanged?: string;
  AudioLevel?: number;
  PartialTranscript?: string;
  FinalTranscript?: string;
  TextInjected?: string;
  Error?: string;
}

// ============================================================================
// Sound Wave Component
// ============================================================================

interface SoundWaveProps {
  audioLevel: number;
  isRecording: boolean;
}

function SoundWave({ audioLevel, isRecording }: SoundWaveProps) {
  // Generate 5 bar heights based on audio level when recording
  const getBarHeights = () => {
    if (!isRecording) return [8, 8, 8, 8, 8];

    // Create varied heights based on audio level
    const base = 6;
    const maxHeight = 24;
    const level = Math.min(1, audioLevel);

    return [
      base + level * (maxHeight - base) * 0.7,
      base + level * (maxHeight - base) * 1.0,
      base + level * (maxHeight - base) * 0.85,
      base + level * (maxHeight - base) * 0.95,
      base + level * (maxHeight - base) * 0.6,
    ];
  };

  const heights = getBarHeights();

  return (
    <div className={`sound-wave ${isRecording ? 'reactive' : ''}`}>
      {heights.map((height, i) => (
        <div
          key={i}
          className="bar"
          style={isRecording ? { height: `${height}px` } : undefined}
        />
      ))}
    </div>
  );
}

// ============================================================================
// Pill Component
// ============================================================================

function App() {
  // State
  const [status, setStatus] = useState<PillStatus>({
    state: 'Idle',
    transcript: '',
    is_recording: false,
    audio_level: 0,
  });
  const [modelsAvailable, setModelsAvailable] = useState(false);
  const [modelsDir, setModelsDir] = useState('');
  const [error, setError] = useState<string | null>(null);

  // Drag state
  const [isDragging, setIsDragging] = useState(false);
  const [dragOffset, setDragOffset] = useState({ x: 0, y: 0 });
  const pillRef = useRef<HTMLDivElement>(null);

  // ============================================================================
  // Initialization
  // ============================================================================

  useEffect(() => {
    const init = async () => {
      try {
        // Check models
        const available = await invoke<boolean>('check_models');
        setModelsAvailable(available);

        if (!available) {
          const dir = await invoke<string>('get_models_dir');
          setModelsDir(dir);
        }
      } catch (e) {
        setError(String(e));
      }
    };

    init();
  }, []);

  // ============================================================================
  // Event Listeners
  // ============================================================================

  useEffect(() => {
    // Listen for hotkey events from backend
    const unlistenPressed = listen('hotkey-pressed', async () => {
      try {
        await invoke('start_recording');
        setError(null);
      } catch (e) {
        setError(String(e));
      }
    });

    const unlistenReleased = listen('hotkey-released', async () => {
      try {
        await invoke('stop_recording');
        setError(null);
      } catch (e) {
        setError(String(e));
      }
    });

    // Listen for pipeline events
    const unlistenPipeline = listen<PipelineEvent>('pipeline-event', (event) => {
      const data = event.payload;

      if (data.StateChanged) {
        setStatus((s) => ({ ...s, state: data.StateChanged! }));
      }
      if (data.AudioLevel !== undefined) {
        setStatus((s) => ({ ...s, audio_level: data.AudioLevel! }));
      }
      if (data.PartialTranscript) {
        setStatus((s) => ({ ...s, transcript: data.PartialTranscript! }));
      }
      if (data.FinalTranscript) {
        setStatus((s) => ({ ...s, transcript: data.FinalTranscript! }));
      }
      if (data.Error) {
        setError(data.Error);
      }
    });

    return () => {
      unlistenPressed.then((f) => f());
      unlistenReleased.then((f) => f());
      unlistenPipeline.then((f) => f());
    };
  }, []);

  // Poll status periodically
  useEffect(() => {
    const interval = setInterval(async () => {
      try {
        const s = await invoke<PillStatus>('get_status');
        setStatus(s);
      } catch (e) {
        // Ignore polling errors
      }
    }, 100);

    return () => clearInterval(interval);
  }, []);

  // ============================================================================
  // Drag Handling
  // ============================================================================

  const handleMouseDown = useCallback(async (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest('.settings-button') ||
        (e.target as HTMLElement).closest('.record-button')) {
      return; // Don't drag when clicking buttons
    }

    // Use Tauri's native window dragging for better compatibility
    try {
      const window = getCurrentWindow();
      await window.startDragging();
    } catch (err) {
      // Fallback to manual dragging
      setIsDragging(true);
      setDragOffset({
        x: e.clientX,
        y: e.clientY,
      });
    }
  }, []);

  const handleMouseMove = useCallback(
    async (e: MouseEvent) => {
      if (!isDragging) return;

      const deltaX = e.screenX - dragOffset.x;
      const deltaY = e.screenY - dragOffset.y;

      const window = getCurrentWindow();
      const pos = await window.outerPosition();

      await window.setPosition(new PhysicalPosition(pos.x + deltaX, pos.y + deltaY));

      setDragOffset({ x: e.screenX, y: e.screenY });
    },
    [isDragging, dragOffset]
  );

  const handleMouseUp = useCallback(() => {
    setIsDragging(false);
  }, []);

  useEffect(() => {
    if (isDragging) {
      window.addEventListener('mousemove', handleMouseMove);
      window.addEventListener('mouseup', handleMouseUp);
    }

    return () => {
      window.removeEventListener('mousemove', handleMouseMove);
      window.removeEventListener('mouseup', handleMouseUp);
    };
  }, [isDragging, handleMouseMove, handleMouseUp]);

  // ============================================================================
  // Render Helpers
  // ============================================================================

  const getStateClass = () => {
    switch (status.state) {
      case 'Listening':
        return 'listening';
      case 'Recording':
        return 'recording';
      case 'Processing':
        return 'processing';
      case 'Injecting':
        return 'injecting';
      default:
        return 'idle';
    }
  };

  const getStatusText = () => {
    if (status.transcript) {
      // Truncate long transcripts
      const maxLen = 35;
      if (status.transcript.length > maxLen) {
        return status.transcript.slice(0, maxLen) + '...';
      }
      return status.transcript;
    }

    switch (status.state) {
      case 'Listening':
        return 'Listening...';
      case 'Recording':
        return 'Recording...';
      case 'Processing':
        return 'Processing...';
      case 'Injecting':
        return 'Injecting...';
      default:
        return 'Press Alt+Shift+V';
    }
  };

  // Manual record handlers
  const handleRecordStart = async () => {
    console.log('Manual record start clicked');
    try {
      await invoke('start_recording');
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const handleRecordStop = async () => {
    console.log('Manual record stop clicked');
    try {
      await invoke('stop_recording');
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  // ============================================================================
  // Render: Models Missing
  // ============================================================================

  if (!modelsAvailable) {
    return (
      <div className="pill models-missing" ref={pillRef} onMouseDown={handleMouseDown} data-tauri-drag-region>
        <SoundWave audioLevel={0} isRecording={false} />
        <div className="content">
          <span className="text">Models required</span>
          <span className="subtext">{modelsDir}</span>
        </div>
      </div>
    );
  }

  // ============================================================================
  // Render: Error State
  // ============================================================================

  if (error) {
    return (
      <div className="pill error-state" ref={pillRef} onMouseDown={handleMouseDown} data-tauri-drag-region>
        <div className="status-icon error">!</div>
        <div className="content">
          <span className="text">{error}</span>
        </div>
        <button className="dismiss-button" onClick={() => setError(null)}>
          ×
        </button>
      </div>
    );
  }

  // ============================================================================
  // Render: Normal Pill
  // ============================================================================

  return (
    <div
      className={`pill ${getStateClass()} ${isDragging ? 'dragging' : ''}`}
      ref={pillRef}
      onMouseDown={handleMouseDown}
      data-tauri-drag-region
    >
      {/* Sound Wave Visualizer */}
      <SoundWave
        audioLevel={status.audio_level}
        isRecording={status.state === 'Recording'}
      />

      {/* Content */}
      <div className="content">
        <span className="text">{getStatusText()}</span>
      </div>

      {/* Buttons */}
      <div className="button-group">
        {/* Record Button */}
        <button
          className={`record-button ${status.is_recording ? 'active' : ''}`}
          title={status.is_recording ? 'Stop Recording' : 'Start Recording'}
          onMouseDown={(e) => {
            e.stopPropagation();
            handleRecordStart();
          }}
          onMouseUp={(e) => {
            e.stopPropagation();
            handleRecordStop();
          }}
          onMouseLeave={() => {
            if (status.is_recording) {
              handleRecordStop();
            }
          }}
        >
          {status.is_recording ? (
            <svg width="14" height="14" viewBox="0 0 14 14" fill="currentColor">
              <rect x="2" y="2" width="10" height="10" rx="1" />
            </svg>
          ) : (
            <svg width="14" height="14" viewBox="0 0 14 14" fill="currentColor">
              <circle cx="7" cy="7" r="5" />
            </svg>
          )}
        </button>

        {/* Settings Button */}
        <button
          className="settings-button"
          title="Settings"
          onClick={async (e) => {
            e.stopPropagation();
            try {
              const devices = await invoke<string[]>('list_audio_devices');
              const deviceList = devices.length > 0
                ? devices.map(d => `  • ${d}`).join('\n')
                : '  (No devices found)';
              alert(`Voice-Dict Settings\n\n` +
                `Hotkey: Alt+Shift+V\n\n` +
                `Audio Input Devices:\n${deviceList}\n\n` +
                `Model: Whisper base.en\n\n` +
                `Tip: Press Alt+Shift+V to toggle recording!`);
            } catch (err) {
              alert(`Settings Error: ${err}`);
            }
          }}
        >
          <svg width="16" height="16" viewBox="0 0 16 16" fill="currentColor">
            <path d="M8 10a2 2 0 100-4 2 2 0 000 4z" />
            <path fillRule="evenodd" d="M8 0a1 1 0 011 1v1.07a5.5 5.5 0 012.36 1.36l.93-.54a1 1 0 011 1.73l-.93.54A5.5 5.5 0 0113 8a5.5 5.5 0 01-.64 2.84l.93.54a1 1 0 01-1 1.73l-.93-.54a5.5 5.5 0 01-2.36 1.36V15a1 1 0 11-2 0v-1.07a5.5 5.5 0 01-2.36-1.36l-.93.54a1 1 0 01-1-1.73l.93-.54A5.5 5.5 0 013 8c0-.99.26-1.92.64-2.84l-.93-.54a1 1 0 011-1.73l.93.54A5.5 5.5 0 017 2.07V1a1 1 0 011-1zm0 4.5a3.5 3.5 0 100 7 3.5 3.5 0 000-7z" clipRule="evenodd" />
          </svg>
        </button>
      </div>
    </div>
  );
}

export default App;
