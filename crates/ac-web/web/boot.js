// Starts Audio Constructor in the browser (ARCHITECTURE §13).
//
// The audio engine shares memory with the page, which browsers allow only
// on a cross-origin isolated page. A host that sends the COOP and COEP
// headers (the `_headers` file says which) is isolated from the start;
// where a host can't send headers, sw.js adds them, which takes effect
// after one reload.

const message = document.getElementById("message");

function show(text) {
  message.textContent = text;
  message.style.display = "flex";
}

const RELOADED = "audio-constructor-isolation-reload";

async function isolate() {
  if (window.crossOriginIsolated) return true;
  if (!("serviceWorker" in navigator)) return false;
  if (sessionStorage.getItem(RELOADED)) {
    // The worker is in place and still no isolation: the host blocks it.
    sessionStorage.removeItem(RELOADED);
    return false;
  }
  await navigator.serviceWorker.register("sw.js");
  await navigator.serviceWorker.ready;
  sessionStorage.setItem(RELOADED, "1");
  window.location.reload();
  return new Promise(() => {}); // the reload takes over
}

async function main() {
  if (!window.isSecureContext) {
    show("Audio Constructor needs a secure page: open it over https, or from localhost.");
    return;
  }
  if (!(await isolate())) {
    show(
      "Audio Constructor needs a cross-origin isolated page so its audio engine can share " +
        "memory with the editor.\nThe site has to send the COOP and COEP headers, or allow " +
        "its service worker.",
    );
    return;
  }
  sessionStorage.removeItem(RELOADED);
  if (!window.AudioWorkletNode || !window.SharedArrayBuffer) {
    show("This browser has no audio worklets or shared memory, which Audio Constructor needs.");
    return;
  }
  const { default: init, start } = await import("./ac_web.js");
  await init();
  window.audioConstructor = await start(document.getElementById("editor"));
  message.style.display = "none";
}

main().catch((e) => {
  console.error(e);
  show(`Audio Constructor couldn't start: ${e && e.message ? e.message : e}`);
});
