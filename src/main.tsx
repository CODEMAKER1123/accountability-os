import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";

import App from "./App";
import { resolveWindowKind } from "./lib/windowKind";
import CaptureWindow from "./windows/CaptureWindow";
import PopupWindow from "./windows/PopupWindow";
import WidgetWindow from "./windows/WidgetWindow";
import "./styles.css";

// One bundle serves every window. Auxiliary windows carry a hash route so
// their App URL stays a valid asset path in both dev and packaged builds.
// Tauri labels remain a fallback for older windows and native-created pages.
let tauriLabel: string | null = null;
if ("__TAURI_INTERNALS__" in window) {
  try {
    tauriLabel = getCurrentWindow().label;
  } catch {
    // A webview can briefly expose Tauri internals before its metadata is ready.
    // Hash routing below still selects the correct auxiliary surface.
  }
}
const kind = resolveWindowKind(window.location.search, tauriLabel, window.location.hash);

function Root() {
  switch (kind) {
    case "popup":
      return <PopupWindow />;
    case "widget":
      return <WidgetWindow />;
    case "capture":
      return <CaptureWindow />;
    default:
      return <App />;
  }
}

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <Root />
  </React.StrictMode>,
);
