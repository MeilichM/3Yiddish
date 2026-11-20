//! What push-to-talk needs from macOS before a take: the microphone, and the permission to
//! press keys in other apps (to paste the words). Both are asked for when push-to-talk is
//! switched on or first pressed, not mid-sentence, so no take is lost to a permission dialog.
//! Other systems ask for neither.

/// Whether push-to-talk can record and paste now. With `ask`, the first missing permission
/// is asked for (macOS's dialog, or its System Settings page once the dialog was answered).
pub fn ready(ask: bool) -> bool {
    #[cfg(target_os = "macos")]
    match mac::mic_status() {
        mac::AUTHORIZED => {}
        mac::NOT_DETERMINED => {
            if ask {
                // Once the microphone is allowed, the paste permission is asked for next.
                mac::request_mic(|granted| {
                    if granted {
                        three_yiddish::dictate::can_type(true);
                    }
                });
            }
            return false;
        }
        _ => {
            if ask {
                let _ = std::process::Command::new("open")
                    .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone")
                    .spawn();
            }
            return false;
        }
    }
    three_yiddish::dictate::can_type(ask)
}

#[cfg(target_os = "macos")]
mod mac {
    use block2::RcBlock;
    use objc2::{
        class, msg_send,
        runtime::{AnyObject, Bool},
    };

    /// AVAuthorizationStatus values.
    pub const NOT_DETERMINED: isize = 0;
    pub const AUTHORIZED: isize = 3;

    #[link(name = "AVFoundation", kind = "framework")]
    unsafe extern "C" {
        static AVMediaTypeAudio: &'static AnyObject;
    }

    pub fn mic_status() -> isize {
        unsafe {
            msg_send![class!(AVCaptureDevice), authorizationStatusForMediaType: AVMediaTypeAudio]
        }
    }

    /// Show macOS's microphone dialog; `then` hears the answer (on another thread).
    pub fn request_mic(then: impl Fn(bool) + Send + Sync + 'static) {
        let block: RcBlock<dyn Fn(Bool)> = RcBlock::new(move |granted: Bool| then(granted.as_bool()));
        unsafe {
            let _: () = msg_send![
                class!(AVCaptureDevice),
                requestAccessForMediaType: AVMediaTypeAudio,
                completionHandler: &*block
            ];
        }
    }
}
