import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";

import App from "./App";
import { resolveWindowKind } from "./lib/windowKind";
import CaptureWindow from "./windows/CaptureWindow";
import PopupWindow from "./windows/PopupWindow";
import WidgetWindow from "./windows/WidgetWindow";
import "./styles.css";

// One bundle serves every window. Production routing uses the Tauri window
// label; query routing is retained only for normal-browser previews.
const tauriLabel = "__TAURI_INTERNALS__" in window ? getCurrentWindow().label : null;
const kind = resolveWindowKind(window.location.search, tauriLabel);

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
