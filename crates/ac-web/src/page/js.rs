//! The page's scripts the wasm module calls (`web/audio.js`,
//! `web/files.js`), copied next to the module by the bindings generator.

use wasm_bindgen::prelude::*;

#[wasm_bindgen(module = "/web/audio.js")]
extern "C" {
    /// Make the audio context and start the worklet with the shared
    /// buffers; resolves to the sample rate.
    #[wasm_bindgen(catch, js_name = createAudio)]
    pub async fn create_audio(shared: &JsValue, options: &JsValue) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = postToAudio)]
    pub fn post_to_audio(bytes: &[u8]);
    #[wasm_bindgen(js_name = audioState)]
    pub fn audio_state() -> String;
    #[wasm_bindgen(js_name = workletState)]
    pub fn worklet_state() -> String;
    #[wasm_bindgen(js_name = workletMessage)]
    pub fn worklet_message() -> String;
    #[wasm_bindgen(js_name = enableMic)]
    pub async fn enable_mic() -> JsValue;
    #[wasm_bindgen(js_name = disableMic)]
    pub fn disable_mic();
    #[wasm_bindgen(js_name = micStatus)]
    pub fn mic_status() -> String;
    #[wasm_bindgen(js_name = micMessage)]
    pub fn mic_message() -> String;
    #[wasm_bindgen(js_name = micLabel)]
    pub fn mic_label() -> String;
}

#[wasm_bindgen(module = "/web/files.js")]
extern "C" {
    /// Resolves to `{ name, bytes, handle }` or null.
    #[wasm_bindgen(catch, js_name = pickFile)]
    pub async fn pick_file(description: &str, accept: Vec<String>) -> Result<JsValue, JsValue>;
    /// Resolves to `{ name, handle }` or null.
    #[wasm_bindgen(catch, js_name = saveFile)]
    pub async fn save_file(
        name: &str,
        bytes: &[u8],
        mime: &str,
        description: &str,
        handle: &JsValue,
        ask: bool,
    ) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(catch, js_name = fetchBytes)]
    pub async fn fetch_bytes(url: &str) -> Result<JsValue, JsValue>;
    #[wasm_bindgen(js_name = queryParam)]
    pub fn query_param(name: &str) -> String;
}

/// A property of a script's result object.
pub fn get(v: &JsValue, key: &str) -> JsValue {
    js_sys::Reflect::get(v, &JsValue::from_str(key)).unwrap_or(JsValue::UNDEFINED)
}

/// Text for an error a script threw.
pub fn error_text(e: &JsValue) -> String {
    if let Some(err) = e.dyn_ref::<js_sys::Error>() {
        return String::from(err.message());
    }
    e.as_string().unwrap_or_else(|| format!("{e:?}"))
}
