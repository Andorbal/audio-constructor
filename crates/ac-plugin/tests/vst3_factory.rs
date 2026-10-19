//! A VST3 smoke test: open the library cargo built (the same file the
//! `.vst3` bundle holds), ask its factory what it exports the way a host
//! scanning plugins does, and create each class. Processing is covered by
//! the CLAP host test; both formats wrap the same plugin code.

// Calling into a plugin library through its C ABI is unsafe by nature; the
// library is this crate's own build, and loading one can't be done safely.
#![allow(unsafe_code)]

mod common;

use std::ffi::{c_char, c_void};

use ac_plugin::{Effect, Instrument};
use nice_plug::prelude::Vst3Plugin;
use vst3::Steinberg::Vst::{BusDirections_, IComponent, IComponentTrait, MediaTypes_};
use vst3::Steinberg::{
    IPluginFactory, IPluginFactory2, IPluginFactory2Trait, IPluginFactoryTrait, PClassInfo,
    PClassInfo2, kResultOk,
};
use vst3::{ComPtr, Interface};

fn text(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars
        .iter()
        .take_while(|c| **c != 0)
        .map(|c| *c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

#[test]
fn the_vst3_factory_exports_the_effect_and_the_instrument() {
    let path = common::plugin_library();
    // SAFETY: our own build (see the top of the file).
    let lib = unsafe { libloading::Library::new(&path) }.expect("the library loads");
    // SAFETY: `GetPluginFactory` is the VST3 entry point, with this
    // signature, exported by `nice_export_vst3!`.
    let get: libloading::Symbol<'_, unsafe extern "system" fn() -> *mut c_void> =
        unsafe { lib.get(b"GetPluginFactory\0") }.expect("GetPluginFactory is exported");
    // SAFETY: the entry point returns an owned IPluginFactory pointer.
    let factory = unsafe { ComPtr::<IPluginFactory>::from_raw(get().cast()) }.expect("a factory");
    let factory2 = factory
        .cast::<IPluginFactory2>()
        .expect("the factory is an IPluginFactory2");

    // SAFETY: plain calls through a valid factory; the info structs are
    // plain data the factory fills.
    let count = unsafe { factory.countClasses() };
    assert_eq!(count, 2, "one class per plugin");
    let mut classes = Vec::new();
    for i in 0..count {
        let mut info: PClassInfo = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { factory.getClassInfo(i, &mut info) }, kResultOk);
        let mut info2: PClassInfo2 = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { factory2.getClassInfo2(i, &mut info2) }, kResultOk);
        assert_eq!(text(&info.category), "Audio Module Class");
        assert_eq!(text(&info2.vendor), "Audio Constructor");
        assert_eq!(text(&info2.version), env!("CARGO_PKG_VERSION"));
        classes.push((
            text(&info.name),
            text(&info2.subCategories),
            info.cid.map(|c| c as u8),
        ));
    }
    classes.sort();
    assert_eq!(
        classes,
        vec![
            (
                "Audio Constructor Effect".to_string(),
                "Fx|Tools".to_string(),
                Effect::PLATFORM_VST3_CLASS_ID
            ),
            (
                "Audio Constructor Instrument".to_string(),
                "Instrument|Synth".to_string(),
                Instrument::PLATFORM_VST3_CLASS_ID
            ),
        ]
    );

    // Each class instantiates as a component with the expected buses: the
    // effect has an audio input and output, the instrument only an output.
    for (cid, inputs) in [
        (Effect::PLATFORM_VST3_CLASS_ID, 1),
        (Instrument::PLATFORM_VST3_CLASS_ID, 0),
    ] {
        let cid = cid.map(|b| b as c_char);
        let mut obj: *mut c_void = std::ptr::null_mut();
        let iid = IComponent::IID.map(|b| b as c_char);
        // SAFETY: valid ids and an out pointer the factory fills.
        let r = unsafe { factory.createInstance(cid.as_ptr(), iid.as_ptr(), &mut obj) };
        assert_eq!(r, kResultOk);
        // SAFETY: createInstance returned an owned IComponent.
        let component = unsafe { ComPtr::<IComponent>::from_raw(obj.cast()) }.expect("a component");
        let audio = MediaTypes_::kAudio as i32;
        let (ins, outs) = unsafe {
            (
                component.getBusCount(audio, BusDirections_::kInput as i32),
                component.getBusCount(audio, BusDirections_::kOutput as i32),
            )
        };
        assert_eq!((ins, outs), (inputs, 1));
    }
}
