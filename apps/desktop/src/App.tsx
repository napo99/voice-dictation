import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface StatusInfo {
  state: string;
  is_recording: boolean;
  is_processing: boolean;
  models_available: boolean;
}

interface ModelInfo {
  name: string;
  filename: string;
  url: string;
  size_bytes: number;
  downloaded: boolean;
}

function App() {
  const [status, setStatus] = useState<StatusInfo>({
    state: 'Idle',
    is_recording: false,
    is_processing: false,
    models_available: false,
  });
  const [models, setModels] = useState<ModelInfo[]>([]);
  const [modelsDir, setModelsDir] = useState<string>('');
  const [error, setError] = useState<string | null>(null);
  const [initialized, setInitialized] = useState(false);

  // Fetch initial data
  useEffect(() => {
    const init = async () => {
      try {
        const modelInfo = await invoke<ModelInfo[]>('get_model_info');
        setModels(modelInfo);

        const dir = await invoke<string>('get_models_dir');
        setModelsDir(dir);

        const modelsAvailable = await invoke<boolean>('check_models_available');
        if (modelsAvailable) {
          await invoke('initialize_engine');
          setInitialized(true);
        }

        await refreshStatus();
      } catch (e) {
        setError(String(e));
      }
    };

    init();
  }, []);

  // Poll status
  useEffect(() => {
    const interval = setInterval(async () => {
      await refreshStatus();
    }, 500);

    return () => clearInterval(interval);
  }, []);

  const refreshStatus = async () => {
    try {
      const s = await invoke<StatusInfo>('get_status');
      setStatus(s);
    } catch (e) {
      console.error('Failed to get status:', e);
    }
  };

  const handleStartRecording = async () => {
    try {
      await invoke('start_recording');
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const handleStopRecording = async () => {
    try {
      await invoke('stop_recording');
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const handleCancel = async () => {
    try {
      await invoke('cancel_operation');
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  const formatBytes = (bytes: number): string => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  };

  const allModelsDownloaded = models.every((m) => m.downloaded);

  return (
    <div className="container">
      <header>
        <h1>Voice-Dict</h1>
        <p className="subtitle">Local voice dictation</p>
      </header>

      <main>
        {/* Status Display */}
        <section className="status-section">
          <div className={`status-indicator ${status.state.toLowerCase()}`}>
            <div className="status-dot" />
            <span>{status.state}</span>
          </div>
        </section>

        {/* Model Status */}
        {!allModelsDownloaded && (
          <section className="models-section">
            <h2>Models Required</h2>
            <p className="models-info">
              Download the following models to:
              <code>{modelsDir}</code>
            </p>
            <ul className="models-list">
              {models.map((model) => (
                <li key={model.filename} className={model.downloaded ? 'downloaded' : ''}>
                  <span className="model-name">{model.name}</span>
                  <span className="model-size">{formatBytes(model.size_bytes)}</span>
                  <span className="model-status">
                    {model.downloaded ? '✓' : '⬇'}
                  </span>
                </li>
              ))}
            </ul>
            <p className="download-hint">
              Run the model download script or download manually.
            </p>
          </section>
        )}

        {/* Controls */}
        {allModelsDownloaded && initialized && (
          <section className="controls-section">
            {!status.is_recording && !status.is_processing && (
              <button
                className="record-button"
                onClick={handleStartRecording}
              >
                Start Recording
              </button>
            )}

            {status.is_recording && (
              <button
                className="stop-button"
                onClick={handleStopRecording}
              >
                Stop Recording
              </button>
            )}

            {status.is_processing && (
              <div className="processing">
                <div className="spinner" />
                <span>Processing...</span>
              </div>
            )}

            {(status.is_recording || status.is_processing) && (
              <button className="cancel-button" onClick={handleCancel}>
                Cancel
              </button>
            )}
          </section>
        )}

        {/* Error Display */}
        {error && (
          <section className="error-section">
            <p>{error}</p>
            <button onClick={() => setError(null)}>Dismiss</button>
          </section>
        )}
      </main>

      <footer>
        <p>Hold hotkey to record, release to transcribe</p>
      </footer>
    </div>
  );
}

export default App;
