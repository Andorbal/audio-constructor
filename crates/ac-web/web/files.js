// Files in a browser (DECISIONS D153): open and save with the File System
// Access API where the browser has it (Save then writes back to the same
// file), else a file picker and a download. The page's wasm module calls
// these.

function abort(e) {
  return e && e.name === "AbortError";
}

// Let the person pick a file. Resolves to { name, bytes, handle } or null
// when they cancel. `accept` lists extensions (".acproj").
export async function pickFile(description, accept) {
  if (window.showOpenFilePicker) {
    try {
      const [handle] = await window.showOpenFilePicker({
        types: [{ description, accept: { "application/octet-stream": accept } }],
        multiple: false,
      });
      const file = await handle.getFile();
      return { name: file.name, bytes: new Uint8Array(await file.arrayBuffer()), handle };
    } catch (e) {
      if (abort(e)) return null;
      // A picker the page may not use (a frame, a policy): fall back.
      console.warn(`file picker unavailable, using an upload instead: ${e}`);
    }
  }
  return new Promise((resolve) => {
    const input = document.createElement("input");
    input.type = "file";
    input.accept = accept.join(",");
    input.style.display = "none";
    input.addEventListener("change", async () => {
      const file = input.files && input.files[0];
      input.remove();
      if (!file) return resolve(null);
      resolve({ name: file.name, bytes: new Uint8Array(await file.arrayBuffer()), handle: null });
    });
    input.addEventListener("cancel", () => {
      input.remove();
      resolve(null);
    });
    document.body.appendChild(input);
    input.click();
  });
}

async function writeTo(handle, bytes) {
  const w = await handle.createWritable();
  await w.write(bytes);
  await w.close();
}

// Save `bytes` as `name`. With a handle from an earlier open or save and
// `ask` false, write back there; otherwise ask where (or download).
// Resolves to { name, handle } or null when the person cancels.
export async function saveFile(name, bytes, mime, description, handle, ask) {
  const data = bytes.slice();
  if (handle && !ask && handle.createWritable) {
    try {
      await writeTo(handle, data);
      return { name: handle.name, handle };
    } catch (e) {
      console.warn(`couldn't write back to ${handle.name}, asking where instead: ${e}`);
    }
  }
  if (window.showSaveFilePicker) {
    const dot = name.lastIndexOf(".");
    const ext = dot >= 0 ? name.slice(dot) : "";
    try {
      const h = await window.showSaveFilePicker({
        suggestedName: name,
        types: ext ? [{ description, accept: { [mime]: [ext] } }] : undefined,
      });
      await writeTo(h, data);
      return { name: h.name, handle: h };
    } catch (e) {
      if (abort(e)) return null;
      console.warn(`save picker unavailable, downloading instead: ${e}`);
    }
  }
  const url = URL.createObjectURL(new Blob([data], { type: mime }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.style.display = "none";
  document.body.appendChild(a);
  a.click();
  a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60_000);
  return { name, handle: null };
}

// Fetch a file from the site.
export async function fetchBytes(url) {
  const r = await fetch(url);
  if (!r.ok) throw new Error(`${url}: ${r.status} ${r.statusText}`);
  return new Uint8Array(await r.arrayBuffer());
}

// The page's query parameter `name`, or "".
export function queryParam(name) {
  return new URLSearchParams(window.location.search).get(name) || "";
}
