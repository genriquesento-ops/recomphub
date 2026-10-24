//! The controller brings RecompHUB back: holding the Xbox/PS button, or View + Menu together,
//! shows the window again from inside a game. The page reads the controller only while it has
//! the focus (Gamepad API), so this is read natively, through XInput (Xbox controllers, and
//! PlayStation controllers through Steam Input or DS4Windows).

use std::time::{Duration, Instant};
use tauri::AppHandle;

pub const GUIDE: u16 = 0x0400;
pub const BACK: u16 = 0x0020;
pub const START: u16 = 0x0010;
/// How long the buttons must be held: a short press of the Xbox button is the Game Bar's.
pub const HOLD: Duration = Duration::from_millis(700);
const TICK: Duration = Duration::from_millis(100);
/// Asking an empty slot is slow in XInput, so empty slots are asked only every 2 s.
const EMPTY_EVERY: u32 = 20;

#[repr(C)]
#[derive(Default)]
#[allow(dead_code)]
struct XState {
    packet: u32,
    buttons: u16,
    left_trigger: u8,
    right_trigger: u8,
    lx: i16,
    ly: i16,
    rx: i16,
    ry: i16,
}

type GetState = unsafe extern "system" fn(u32, *mut XState) -> u32;

/// XInputGetStateEx (ordinal 100) also reports the Xbox button; plain XInputGetState does not.
fn load() -> Option<GetState> {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
    for dll in ["xinput1_4.dll", "xinput1_3.dll"] {
        let name: Vec<u16> = dll.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: a NUL-terminated name; the library stays loaded for the life of the program.
        unsafe {
            let lib = LoadLibraryW(name.as_ptr());
            if lib.is_null() {
                continue;
            }
            let f = GetProcAddress(lib, 100usize as *const u8).or_else(|| GetProcAddress(lib, b"XInputGetState\0".as_ptr()));
            if let Some(f) = f {
                return Some(std::mem::transmute::<unsafe extern "system" fn() -> isize, GetState>(f));
            }
        }
    }
    None
}

/// The buttons that call RecompHUB back.
pub fn calls_back(buttons: u16) -> bool {
    buttons & GUIDE != 0 || (buttons & BACK != 0 && buttons & START != 0)
}

/// Holding state: fires once per hold, after `HOLD`.
#[derive(Default)]
pub struct Hold {
    since: Option<Instant>,
    fired: bool,
}

impl Hold {
    pub fn update(&mut self, pressed: bool, now: Instant) -> bool {
        if !pressed {
            *self = Hold::default();
            return false;
        }
        let since = *self.since.get_or_insert(now);
        if !self.fired && now.duration_since(since) >= HOLD {
            self.fired = true;
            return true;
        }
        false
    }
}

pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || {
        let Some(get) = load() else { return };
        let mut connected = [true; 4];
        let mut hold = Hold::default();
        let mut tick: u32 = 0;
        loop {
            std::thread::sleep(TICK);
            tick = tick.wrapping_add(1);
            let mut pressed = false;
            for (slot, on) in connected.iter_mut().enumerate() {
                if !*on && tick % EMPTY_EVERY != 0 {
                    continue;
                }
                let mut s = XState::default();
                // SAFETY: `s` has XInput's XINPUT_STATE layout and lives across the call.
                *on = unsafe { get(slot as u32, &mut s) } == 0;
                pressed |= *on && calls_back(s.buttons);
            }
            if hold.update(pressed, Instant::now()) {
                crate::tray::show_window(&app);
                // show() is carried out by the window thread; give it a moment.
                std::thread::sleep(Duration::from_millis(150));
                crate::play::bring_own_window();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn which_buttons_call_back() {
        assert!(calls_back(GUIDE));
        assert!(calls_back(BACK | START | 0x1000));
        assert!(!calls_back(BACK));
        assert!(!calls_back(START | 0x1000));
        assert_eq!(std::mem::size_of::<XState>(), 16);
    }

    #[test]
    fn fires_once_after_holding() {
        let t = Instant::now();
        let mut h = Hold::default();
        assert!(!h.update(true, t));
        assert!(!h.update(true, t + Duration::from_millis(300)));
        assert!(h.update(true, t + HOLD));
        assert!(!h.update(true, t + HOLD * 3));
        assert!(!h.update(false, t + HOLD * 4));
        assert!(!h.update(true, t + HOLD * 5));
        assert!(h.update(true, t + HOLD * 6));
    }
}
