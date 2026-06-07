import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

function App() {
  const [engineStatus, setEngineStatus] = useState("checking backend");

  useEffect(() => {
    invoke<string>("engine_status")
      .then(setEngineStatus)
      .catch(() => setEngineStatus("backend unavailable"));
  }, []);

  return (
    <main className="app-shell">
      <section className="workspace">
        <header className="topbar">
          <div>
            <p className="eyebrow">Pulse</p>
            <h1>Local music engine</h1>
          </div>
          <div className="status-pill">{engineStatus}</div>
        </header>

        <div className="stage">
          <div className="meter-panel" aria-hidden="true">
            <div className="meter meter-a" />
            <div className="meter meter-b" />
            <div className="meter meter-c" />
            <div className="meter meter-d" />
            <div className="meter meter-e" />
          </div>

          <div className="summary">
            <p className="label">Current milestone</p>
            <h2>Validate bit-perfect playback before the library UI.</h2>
            <p>
              The app shell is ready to host Tauri commands while the CLI keeps
              driving the standalone audio engine.
            </p>
          </div>
        </div>
      </section>
    </main>
  );
}

export default App;
