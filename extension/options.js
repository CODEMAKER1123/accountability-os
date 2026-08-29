const portEl = document.getElementById("port");
const tokenEl = document.getElementById("token");
const statusEl = document.getElementById("status");

async function load() {
  const stored = await chrome.storage.local.get({ port: 43117, token: "" });
  portEl.value = stored.port;
  tokenEl.value = stored.token;
}

function setStatus(text, ok) {
  statusEl.textContent = text;
  statusEl.className = ok ? "ok" : "err";
}

document.getElementById("save").addEventListener("click", async () => {
  const port = Math.min(65535, Math.max(1024, Number(portEl.value) || 43117));
  await chrome.storage.local.set({ port, token: tokenEl.value.trim() });
  setStatus("Saved.", true);
});

document.getElementById("test").addEventListener("click", async () => {
  const port = Number(portEl.value) || 43117;
  try {
    const resp = await fetch(`http://127.0.0.1:${port}/v1/ping`);
    const body = await resp.json();
    if (body && body.app === "accountability-os") {
      setStatus("Connected to Accountability OS.", true);
    } else {
      setStatus("Something answered, but it isn't Accountability OS.", false);
    }
  } catch {
    setStatus("No response — is the desktop app running on that port?", false);
  }
});

load();
