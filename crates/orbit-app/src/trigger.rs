//! The Shift-drag trigger: listen to global input, and open the ring when a
//! shifted drag begins.
//!
//! The interesting part is a small state machine with no operating-system code
//! in it, so the definition of "a Shift-drag" is testable without a mouse:
//! Shift down, button down, move more than [`MOVE_THRESHOLD`] pixels. Pressing
//! Shift after the drag has started counts too, which is how people recover
//! when they forget the modifier until the file is already moving.
//!
//! The listener itself is `rdev`, which speaks to the native input APIs on
//! Windows and macOS and to `XRecord` on X11. Wayland deliberately has no
//! equivalent: a client cannot observe the compositor's input, so the trigger
//! is reported as unavailable there and the compositor binding, tray and
//! "Open with Orbit" remain the ways in.

use std::time::{Duration, Instant};

use rdev::{Button, EventType, Key};
use tauri::{AppHandle, Runtime};

/// How far the pointer must move before a Shift-press becomes a drag.
pub const MOVE_THRESHOLD: f64 = 12.0;

/// Ignore a second drag that starts immediately after one ends; this is the
/// double-click-with-a-wobble case, not a deliberate second gesture.
pub const REPEAT_GUARD: Duration = Duration::from_millis(350);

/// What the detector wants the shell to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriggerAction {
    /// The gesture crossed the threshold: show the ring.
    Show,
    /// The button came up after a trigger: hide the ring unless a file landed.
    Released,
}

/// A pure Shift-drag detector.
#[derive(Debug, Default)]
pub struct Detector {
    shift: bool,
    button: bool,
    /// Where the button went down. Used to tell a Shift-click from a drag.
    press: Option<(f64, f64)>,
    /// The last pointer position, so pressing Shift mid-drag can be judged.
    last: Option<(f64, f64)>,
    origin: Option<(f64, f64)>,
    fired: bool,
    last_fire: Option<Instant>,
}

impl Detector {
    /// A detector with nothing held.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one input event.
    ///
    /// `now` is passed in rather than read from the clock so the cooldown is
    /// testable without sleeping.
    #[allow(clippy::cast_precision_loss)] // pixel coordinates are far below f64's limit
    pub fn handle(&mut self, event: &EventType, now: Instant) -> Option<TriggerAction> {
        match event {
            EventType::KeyPress(Key::ShiftLeft | Key::ShiftRight) => {
                self.shift = true;
                // Shift pressed mid-drag is a drag the user means to convert —
                // but only if the pointer actually travelled. A Shift-click to
                // extend a selection must not open the ring.
                if self.button && !self.fired && self.cool(now) {
                    if let (Some(origin), Some(last)) = (self.press, self.last) {
                        if distance(origin, last) >= MOVE_THRESHOLD {
                            return Some(self.fire(now));
                        }
                    }
                }
                None
            }
            EventType::KeyRelease(Key::ShiftLeft | Key::ShiftRight) => {
                self.shift = false;
                self.origin = None;
                None
            }
            EventType::ButtonPress(Button::Left) => {
                self.button = true;
                self.press = self.last;
                self.origin = None;
                self.fired = false;
                None
            }
            EventType::ButtonRelease(Button::Left) => {
                let fired = self.fired;
                self.button = false;
                self.press = None;
                self.origin = None;
                self.fired = false;
                fired.then_some(TriggerAction::Released)
            }
            EventType::MouseMove { x, y } => {
                self.last = Some((*x, *y));
                if !(self.button && self.shift && !self.fired && self.cool(now)) {
                    return None;
                }
                match self.origin {
                    None => {
                        self.origin = Some((*x, *y));
                        None
                    }
                    Some(origin) => {
                        (distance(origin, (*x, *y)) >= MOVE_THRESHOLD).then(|| self.fire(now))
                    }
                }
            }
            _ => None,
        }
    }

    fn cool(&self, now: Instant) -> bool {
        self.last_fire
            .is_none_or(|last| now.duration_since(last) >= REPEAT_GUARD)
    }

    fn fire(&mut self, now: Instant) -> TriggerAction {
        self.fired = true;
        self.last_fire = Some(now);
        TriggerAction::Show
    }
}

/// Euclidean distance between two pointer positions.
fn distance(a: (f64, f64), b: (f64, f64)) -> f64 {
    ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
}

/// What the settings window says about the listener.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TriggerStatus {
    /// The user's setting: should Shift-drag open the ring?
    pub enabled: bool,
    /// Whether the input listener is actually running.
    pub listening: bool,
    /// Why it is not, when it cannot be.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl TriggerStatus {
    /// The listener could not start here.
    #[must_use]
    pub fn unavailable(enabled: bool, note: impl Into<String>) -> Self {
        Self {
            enabled,
            listening: false,
            note: Some(note.into()),
        }
    }
}

/// Start the global listener on a dedicated thread.
///
/// It runs for the life of the process. Turning the setting off does not stop
/// the thread; the shell checks the setting before acting on an event, so the
/// toggle takes effect immediately and can be turned back on without a restart.
#[must_use]
pub fn start<R: Runtime>(app: &AppHandle<R>) -> TriggerStatus {
    if cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return TriggerStatus::unavailable(
            true,
            "Wayland does not expose global input to apps. Bind Orbit in your compositor instead.",
        );
    }

    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("orbit-trigger".to_owned())
        .spawn(move || {
            let mut detector = Detector::new();
            let result = rdev::listen(move |event| {
                if let Some(action) = detector.handle(&event.event_type, Instant::now()) {
                    match action {
                        TriggerAction::Show => crate::show_from_trigger(&handle),
                        TriggerAction::Released => crate::release_from_trigger(&handle),
                    }
                }
            });
            if let Err(error) = result {
                eprintln!("orbit: the Shift-drag listener stopped ({error:?})");
            }
        });

    match spawned {
        Ok(_) => TriggerStatus {
            enabled: true,
            listening: true,
            note: None,
        },
        Err(error) => TriggerStatus::unavailable(
            true,
            format!("The Shift-drag listener could not start: {error}"),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_shifted_drag_opens_once_at_the_threshold() {
        let base = Instant::now();
        let mut detector = Detector::new();
        assert_eq!(
            detector.handle(&EventType::KeyPress(Key::ShiftLeft), base),
            None
        );
        assert_eq!(
            detector.handle(&EventType::ButtonPress(Button::Left), base),
            None
        );
        // The first move only records the origin.
        assert_eq!(
            detector.handle(&EventType::MouseMove { x: 100.0, y: 100.0 }, base),
            None
        );
        // Below the threshold: still a click.
        assert_eq!(
            detector.handle(&EventType::MouseMove { x: 106.0, y: 106.0 }, base),
            None
        );
        assert_eq!(
            detector.handle(&EventType::MouseMove { x: 120.0, y: 100.0 }, base),
            Some(TriggerAction::Show)
        );
        // Further movement in the same drag does not reopen it.
        assert_eq!(
            detector.handle(&EventType::MouseMove { x: 200.0, y: 200.0 }, base),
            None
        );
        assert_eq!(
            detector.handle(&EventType::ButtonRelease(Button::Left), base),
            Some(TriggerAction::Released)
        );
    }

    #[test]
    fn an_unshifted_drag_does_nothing() {
        let base = Instant::now();
        let mut detector = Detector::new();
        detector.handle(&EventType::ButtonPress(Button::Left), base);
        detector.handle(&EventType::MouseMove { x: 0.0, y: 0.0 }, base);
        assert_eq!(
            detector.handle(&EventType::MouseMove { x: 80.0, y: 80.0 }, base),
            None
        );
        assert_eq!(
            detector.handle(&EventType::ButtonRelease(Button::Left), base),
            None
        );
    }

    #[test]
    fn shift_pressed_after_the_drag_started_still_triggers() {
        let base = Instant::now();
        let mut detector = Detector::new();
        detector.handle(&EventType::MouseMove { x: 0.0, y: 0.0 }, base);
        detector.handle(&EventType::ButtonPress(Button::Left), base);
        detector.handle(&EventType::MouseMove { x: 10.0, y: 10.0 }, base);
        detector.handle(&EventType::MouseMove { x: 60.0, y: 10.0 }, base);
        assert_eq!(
            detector.handle(&EventType::KeyPress(Key::ShiftRight), base),
            Some(TriggerAction::Show)
        );
    }

    #[test]
    fn a_shift_click_without_movement_is_not_a_drag() {
        let base = Instant::now();
        let mut detector = Detector::new();
        detector.handle(&EventType::MouseMove { x: 40.0, y: 40.0 }, base);
        detector.handle(&EventType::ButtonPress(Button::Left), base);
        assert_eq!(
            detector.handle(&EventType::KeyPress(Key::ShiftLeft), base),
            None
        );
        // A pixel of hand tremor is still a click.
        assert_eq!(
            detector.handle(&EventType::MouseMove { x: 41.0, y: 41.0 }, base),
            None
        );
        assert_eq!(
            detector.handle(&EventType::ButtonRelease(Button::Left), base),
            None
        );
    }

    #[test]
    fn a_second_gesture_inside_the_guard_is_ignored() {
        let base = Instant::now();
        let mut detector = Detector::new();
        detector.handle(&EventType::KeyPress(Key::ShiftLeft), base);
        detector.handle(&EventType::ButtonPress(Button::Left), base);
        detector.handle(&EventType::MouseMove { x: 0.0, y: 0.0 }, base);
        assert_eq!(
            detector.handle(&EventType::MouseMove { x: 100.0, y: 0.0 }, base),
            Some(TriggerAction::Show)
        );
        detector.handle(&EventType::ButtonRelease(Button::Left), base);

        // A wobble, not a deliberate drag.
        detector.handle(&EventType::ButtonPress(Button::Left), base);
        detector.handle(&EventType::MouseMove { x: 0.0, y: 0.0 }, base);
        assert_eq!(
            detector.handle(&EventType::MouseMove { x: 100.0, y: 0.0 }, base),
            None
        );
        detector.handle(&EventType::ButtonRelease(Button::Left), base);

        // A moment later the gesture works again.
        let later = base + REPEAT_GUARD + Duration::from_millis(10);
        detector.handle(&EventType::ButtonPress(Button::Left), later);
        detector.handle(&EventType::MouseMove { x: 0.0, y: 0.0 }, later);
        assert_eq!(
            detector.handle(&EventType::MouseMove { x: 100.0, y: 0.0 }, later),
            Some(TriggerAction::Show)
        );
    }

    #[test]
    fn releasing_shift_mid_drag_does_not_fire_later() {
        let base = Instant::now();
        let mut detector = Detector::new();
        detector.handle(&EventType::KeyPress(Key::ShiftLeft), base);
        detector.handle(&EventType::ButtonPress(Button::Left), base);
        detector.handle(&EventType::MouseMove { x: 0.0, y: 0.0 }, base);
        detector.handle(&EventType::KeyRelease(Key::ShiftLeft), base);
        assert_eq!(
            detector.handle(&EventType::MouseMove { x: 100.0, y: 0.0 }, base),
            None
        );
    }
}
