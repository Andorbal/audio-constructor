// Audio Constructor's audio worklet (ARCHITECTURE §13, DECISIONS D150,
// D151). It runs the engine module (ac_audio.wasm) one render quantum at a
// time. The page reaches it through three SharedArrayBuffers (a ring of
// records to the engine, a ring back, and a telemetry snapshot), which the
// module reads and writes through the `env` functions below, and posts
// projects to compile, which arrive as messages and are handled between
// render quanta, never inside process().
//
// Nothing in process() allocates: the views on the module's memory are
// made once (again only if the memory grows, which happens while a project
// compiles, not while audio runs).


class AudioConstructorProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const o = options.processorOptions;
    this.shared = [o.toAudio, o.fromAudio, o.snapshot].map((sab) => ({
      u8: new Uint8Array(sab),
      i32: new Int32Array(sab),
    }));
    this.inChannels = o.inChannels;
    this.outChannels = o.outChannels;
    this.maxBlock = o.maxBlock;
    this.exports = null;
    this.failed = false;
    this.waiting = []; // messages that came before the module was ready
    this.memory = null;
    this.views = { buffer: null, frames: 0, u8: null, i32: null, inputs: [], outputs: [] };
    this.port.onmessage = (e) => this.onMessage(e.data);
    const env = {
      ac_shared_len: (b) => this.shared[b].u8.length,
      ac_shared_load: (b, w) => Atomics.load(this.shared[b].i32, w),
      ac_shared_store: (b, w, v) => {
        Atomics.store(this.shared[b].i32, w, v);
      },
      ac_shared_read: (b, offset, ptr, len) => this.copy(this.shared[b], offset, true, ptr, len),
      ac_shared_write: (b, offset, ptr, len) => this.copy(this.shared[b], offset, false, ptr, len),
    };
    WebAssembly.instantiate(o.wasm, { env })
      .then(({ instance }) => {
        const x = instance.exports;
        this.memory = x.memory;
        if (x.ac_init(sampleRate, this.maxBlock, this.inChannels, this.outChannels) !== 1) {
          throw new Error("the shared buffers don't hold rings");
        }
        this.exports = x;
        for (const m of this.waiting) this.handle(m);
        this.waiting = [];
        this.port.postMessage({ type: "ready" });
      })
      .catch((e) => this.fail(e));
  }

  fail(e) {
    this.failed = true;
    this.port.postMessage({ type: "error", text: String(e && e.message ? e.message : e) });
  }

  // Views on the module's memory, made again only when it grew (or the
  // quantum's length changed).
  ensureViews(frames) {
    const v = this.views;
    const buffer = this.memory.buffer;
    if (v.buffer === buffer && v.frames === frames) return v;
    v.buffer = buffer;
    v.frames = frames;
    v.u8 = new Uint8Array(buffer);
    v.i32 = new Int32Array(buffer);
    const input = this.exports.ac_input();
    const output = this.exports.ac_output();
    v.inputs = [];
    for (let c = 0; c < this.inChannels; c++) {
      v.inputs.push(new Float32Array(buffer, input + c * frames * 4, frames));
    }
    v.outputs = [];
    for (let c = 0; c < this.outChannels; c++) {
      v.outputs.push(new Float32Array(buffer, output + c * frames * 4, frames));
    }
    return v;
  }

  // Copy between a shared buffer and the module's memory, by words when
  // everything lines up (telemetry), else by bytes (ring records).
  copy(sh, offset, intoModule, ptr, len) {
    const v = this.ensureViews(this.views.frames || 128);
    if (((offset | ptr | len) & 3) === 0) {
      const a = sh.i32;
      const m = v.i32;
      const so = offset >> 2;
      const mo = ptr >> 2;
      const n = len >> 2;
      if (intoModule) for (let i = 0; i < n; i++) m[mo + i] = a[so + i];
      else for (let i = 0; i < n; i++) a[so + i] = m[mo + i];
    } else {
      const a = sh.u8;
      const m = v.u8;
      if (intoModule) for (let i = 0; i < len; i++) m[ptr + i] = a[offset + i];
      else for (let i = 0; i < len; i++) a[offset + i] = m[ptr + i];
    }
  }

  onMessage(data) {
    if (!data || data.type !== "posted") return;
    if (!this.exports) {
      this.waiting.push(data.bytes);
      return;
    }
    this.handle(data.bytes);
  }

  // A posted message: a project to compile, its audio files, or a request
  // to free retired graphs. Here, between render quanta, allocating is
  // fine.
  handle(bytes) {
    if (this.failed) return;
    try {
      const src = new Uint8Array(bytes);
      const ptr = this.exports.ac_message_buffer(src.length);
      new Uint8Array(this.memory.buffer, ptr, src.length).set(src);
      const r = this.exports.ac_message(src.length);
      if (r === 0) this.port.postMessage({ type: "warning", text: "a message the engine couldn't read" });
    } catch (e) {
      this.fail(e);
    }
  }

  process(inputs, outputs) {
    const out = outputs[0];
    if (!this.exports || this.failed || !out || out.length === 0) return true;
    const frames = out[0].length;
    try {
      const v = this.ensureViews(frames);
      const input = inputs[0] || [];
      for (let c = 0; c < this.inChannels; c++) {
        const ch = input[c] || input[0];
        if (ch) v.inputs[c].set(ch);
        else v.inputs[c].fill(0);
      }
      this.exports.ac_process(frames);
      // Compiling never happens in here, but if the memory moved anyway,
      // read the output from where it is now.
      const w = this.ensureViews(frames);
      for (let c = 0; c < out.length; c++) out[c].set(w.outputs[Math.min(c, w.outputs.length - 1)]);
    } catch (e) {
      this.fail(e);
    }
    return true;
  }
}

registerProcessor("audio-constructor", AudioConstructorProcessor);
