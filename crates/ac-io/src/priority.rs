//! Real-time scheduling for device callback threads (ARCHITECTURE §11).
//!
//! Where the backend already runs callbacks at real-time priority (CoreAudio,
//! JACK) or cpal promotes them (MMCSS on Windows), there's nothing to do.
//! cpal's promotion on Linux needs D-Bus (RealtimeKit), which this crate
//! leaves out, so for ALSA the first callback asks for `SCHED_FIFO` itself.
//! That works when the user may raise priority (an `rtprio` limit, as the
//! `audio` group usually grants); a refusal is reported, never fatal.
//!
//! The request is one system call, made once at the start of the stream,
//! outside the allocation guard; steady-state callbacks make none.

/// Below the priority JACK and PipeWire give their own data threads, so a
/// server and this process don't fight, and well above everything else.
#[cfg(target_os = "linux")]
const RT_PRIORITY: u8 = 70;

/// Ask for real-time priority for the calling thread. Returns false if the
/// OS refused.
#[cfg(target_os = "linux")]
pub fn promote_current_thread() -> bool {
    use thread_priority::{
        RealtimeThreadSchedulePolicy, ThreadPriority, ThreadSchedulePolicy,
        set_thread_priority_and_policy, thread_native_id,
    };
    let Ok(priority) = ThreadPriority::try_from(RT_PRIORITY) else {
        return false;
    };
    set_thread_priority_and_policy(
        thread_native_id(),
        priority,
        ThreadSchedulePolicy::Realtime(RealtimeThreadSchedulePolicy::Fifo),
    )
    .is_ok()
}

/// Other platforms promote callback threads in the backend.
#[cfg(not(target_os = "linux"))]
pub fn promote_current_thread() -> bool {
    true
}

/// Whether callbacks from this cpal host need [`promote_current_thread`].
pub fn host_needs_promotion(host_name: &str) -> bool {
    cfg!(target_os = "linux") && host_name.eq_ignore_ascii_case("ALSA")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promotion_never_panics() {
        // A container or an ordinary desktop account is usually refused;
        // either answer is fine. Run it on a scratch thread so a success
        // doesn't leave the test runner's thread at real-time priority.
        let ok = std::thread::spawn(promote_current_thread).join().unwrap();
        let _ = ok;
        assert_eq!(host_needs_promotion("alsa"), cfg!(target_os = "linux"));
        assert!(!host_needs_promotion("JACK"));
    }
}
