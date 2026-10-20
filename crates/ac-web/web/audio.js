// The page's audio (ARCHITECTURE §13): an AudioContext, the audio worklet
// running the engine, and the microphone for Audio In nodes. The page's
// wasm module calls these; the worklet itself is worklet.js.

let context = null;
let node = null;
let analyser = null;
let micStream = null;
let micSource = null;
let micState = "off"; // off | asking | on | denied | unavailable
let micError = "";
let worklet = { state: "loading", message: "" }; // loading | ready | failed

// Browsers start audio only after the person clicks or presses a key. A
// context made before that is "suspended", or "interrupted" in Safari,
// which reports that state (not "suspended") for a context the page
// made without a gesture; both resume on one.
function resume() {
  if (!context) return;
  const s = context.state;
  if (s === "suspended" || s === "interrupted") context.resume().catch(() => {});
}

// Make the context, load the worklet and the engine module, and start the
// engine with the shared buffers the page made. Resolves to the context's
// sample rate.
export async function createAudio(shared, options) {
  context = new AudioContext({ latencyHint: "interactive" });
  const [wasm] = await Promise.all([
    fetch(options.wasmUrl).then((r) => {
      if (!r.ok) throw new Error(`${options.wasmUrl}: ${r.status}`);
      return r.arrayBuffer();
    }),
    context.audioWorklet.addModule(options.workletUrl),
  ]);
  node = new AudioWorkletNode(context, "audio-constructor", {
    numberOfInputs: 1,
    numberOfOutputs: 1,
    outputChannelCount: [options.outChannels],
    channelCount: Math.max(1, options.inChannels),
    channelCountMode: "explicit",
    channelInterpretation: "speakers",
    processorOptions: {
      wasm,
      toAudio: shared.toAudio,
      fromAudio: shared.fromAudio,
      snapshot: shared.snapshot,
      inChannels: options.inChannels,
      outChannels: options.outChannels,
      maxBlock: options.maxBlock,
    },
  });
  node.port.onmessage = (e) => {
    const m = e.data || {};
    if (m.type === "ready") worklet = { state: "ready", message: "" };
    if (m.type === "error") {
      worklet = { state: "failed", message: m.text };
      console.error(`Audio Constructor's audio stopped: ${m.text}`);
    }
    if (m.type === "warning") console.warn(`Audio Constructor: ${m.text}`);
  };
  node.onprocessorerror = () => {
    worklet = { state: "failed", message: "the audio worklet stopped" };
    console.error("Audio Constructor's audio worklet stopped");
  };
  node.connect(context.destination);
  // For people and scripts that want to look at the output (and the tests).
  analyser = context.createAnalyser();
  analyser.fftSize = 8192;
  analyser.smoothingTimeConstant = 0;
  node.connect(analyser);
  globalThis.audioConstructorAudio = { context, node, analyser };
  for (const ev of ["pointerdown", "keydown", "touchend"]) {
    document.addEventListener(ev, resume, { capture: true });
  }
  resume();
  return context.sampleRate;
}

// Hand the worklet a posted message (a copy: the bytes are a view of the
// page module's memory).
export function postToAudio(bytes) {
  if (!node) return;
  const copy = bytes.slice();
  node.port.postMessage({ type: "posted", bytes: copy.buffer }, [copy.buffer]);
}

// "suspended" (or Safari's "interrupted") until the person clicks or
// presses a key, then "running".
export function audioState() {
  return context ? context.state : "none";
}

// "loading", "ready", or "failed".
export function workletState() {
  return worklet.state;
}

export function workletMessage() {
  return worklet.message;
}

// Ask for the microphone and feed it to the engine's input. Echo
// cancellation, noise suppression, and automatic gain are off: an
// instrument wants its own sound.
export async function enableMic() {
  if (!context || !node) return false;
  if (micState === "on") return true;
  if (!navigator.mediaDevices || !navigator.mediaDevices.getUserMedia) {
    micState = "unavailable";
    micError = "this browser offers no microphone here (it needs a secure page)";
    return false;
  }
  micState = "asking";
  try {
    micStream = await navigator.mediaDevices.getUserMedia({
      audio: { echoCancellation: false, noiseSuppression: false, autoGainControl: false },
    });
    micSource = context.createMediaStreamSource(micStream);
    micSource.connect(node);
    micState = "on";
    micError = "";
    resume();
    return true;
  } catch (e) {
    micState = e && e.name === "NotAllowedError" ? "denied" : "unavailable";
    micError = String(e && e.message ? e.message : e);
    return false;
  }
}

export function disableMic() {
  if (micSource) micSource.disconnect();
  if (micStream) for (const t of micStream.getTracks()) t.stop();
  micSource = null;
  micStream = null;
  micState = "off";
}

export function micStatus() {
  return micState;
}

export function micMessage() {
  return micError;
}

// The microphone's name, once it's on.
export function micLabel() {
  const t = micStream && micStream.getAudioTracks()[0];
  return t ? t.label : "";
}
