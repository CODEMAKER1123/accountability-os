import React from "react";
import ReactDOM from "react-dom/client";

import App from "./App";
import CaptureWindow from "./windows/CaptureWindow";
import PopupWindow from "./windows/PopupWindow";
import WidgetWindow from "./windows/WidgetWindow";
import "./styles.css";

// One bundle serves every window; ?window= picks the surface.
const kind = new URLSearchParams(window.location.search).get("window");

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
