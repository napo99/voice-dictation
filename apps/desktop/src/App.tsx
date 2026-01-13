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

  const handleMouseDown = useCallback((e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest('.settings-button')) {
      return; // Don't drag when clicking settings
    }

    setIsDragging(true);
    setDragOffset({
      x: e.clientX,
      y: e.clientY,
    });
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
  // Render
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
        return 'Press Ctrl+Shift+Space';
    }
  };

  // Models not available - show download prompt
  if (!modelsAvailable) {
    return (
      <div className="pill models-missing" ref={pillRef} onMouseDown={handleMouseDown}>
        <div className="status-icon error">!</div>
        <div className="content">
          <span className="text">Models required</span>
          <span className="subtext">{modelsDir}</span>
        </div>
      </div>
    );
  }

  // Error state
  if (error) {
    return (
      <div className="pill error-state" ref={pillRef} onMouseDown={handleMouseDown}>
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

  // Normal pill
  return (
    <div
      className={`pill ${getStateClass()} ${isDragging ? 'dragging' : ''}`}
      ref={pillRef}
      onMouseDown={handleMouseDown}
    >
      <div className="status-icon">
        <div className="dot" />
        {status.is_recording && (
          <div
            className="audio-ring"
            style={{
              transform: `scale(${1 + status.audio_level * 0.5})`,
              opacity: 0.3 + status.audio_level * 0.7,
            }}
          />
        )}
      </div>

      <div className="content">
        <span className="text">{getStatusText()}</span>
      </div>

      <button className="settings-button" title="Settings">
        ⚙
      </button>
    </div>
  );
}

export default App;
