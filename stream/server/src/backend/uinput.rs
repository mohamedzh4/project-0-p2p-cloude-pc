//! Virtual input devices in the guest through /dev/uinput: a keyboard, a
//! relative mouse (games), an absolute pointer (desktop) and up to four
//! Xbox 360-style gamepads (Steam and SDL know that layout).
//!
//! Every code and range from the client is checked: only keyboard keys,
//! the five mouse buttons and gamepad buttons are accepted.

use super::*;
use dcl_stream_proto::msg::MouseButton;
use evdev::uinput::{VirtualDevice, VirtualDeviceBuilder};
use evdev::{AbsInfo, AbsoluteAxisType, AttributeSet, BusType, EventType, InputEvent as Ev, InputId, Key, RelativeAxisType, UinputAbsSetup};
use std::collections::HashMap;

pub const MAX_PADS: u8 = 4;
const KEY_MAX_KEYBOARD: u16 = 248; // KEY_MICMUTE; above are buttons and special codes
const PAD_BUTTONS: [u16; 11] = [
    0x130, 0x131, 0x133, 0x134, // BTN_SOUTH, EAST, NORTH, WEST
    0x136, 0x137, // BTN_TL, BTN_TR
    0x13a, 0x13b, 0x13c, // BTN_SELECT, START, MODE
    0x13d, 0x13e, // BTN_THUMBL, THUMBR
];

pub struct Uinput {
    keyboard: VirtualDevice,
    mouse: VirtualDevice,
    pointer: VirtualDevice,
    pads: HashMap<u8, VirtualDevice>,
}

fn abs(axis: AbsoluteAxisType, min: i32, max: i32) -> UinputAbsSetup {
    UinputAbsSetup::new(axis, AbsInfo::new(0, min, max, 0, 0, 1))
}

impl Uinput {
    pub fn new() -> anyhow::Result<Self> {
        let mut keys = AttributeSet::<Key>::new();
        for code in 1..=KEY_MAX_KEYBOARD {
            keys.insert(Key::new(code));
        }
        let keyboard = VirtualDeviceBuilder::new()?.name("DCL Stream keyboard").with_keys(&keys)?.build()?;

        let mut buttons = AttributeSet::<Key>::new();
        for b in [Key::BTN_LEFT, Key::BTN_RIGHT, Key::BTN_MIDDLE, Key::BTN_SIDE, Key::BTN_EXTRA] {
            buttons.insert(b);
        }
        let mut rel = AttributeSet::<RelativeAxisType>::new();
        for r in [
            RelativeAxisType::REL_X,
            RelativeAxisType::REL_Y,
            RelativeAxisType::REL_WHEEL,
            RelativeAxisType::REL_HWHEEL,
            RelativeAxisType::REL_WHEEL_HI_RES,
            RelativeAxisType::REL_HWHEEL_HI_RES,
        ] {
            rel.insert(r);
        }
        let mouse = VirtualDeviceBuilder::new()?.name("DCL Stream mouse").with_keys(&buttons)?.with_relative_axes(&rel)?.build()?;

        let mut tbtn = AttributeSet::<Key>::new();
        tbtn.insert(Key::BTN_LEFT);
        let pointer = VirtualDeviceBuilder::new()?
            .name("DCL Stream pointer")
            .with_keys(&tbtn)?
            .with_absolute_axis(&abs(AbsoluteAxisType::ABS_X, 0, 65535))?
            .with_absolute_axis(&abs(AbsoluteAxisType::ABS_Y, 0, 65535))?
            .build()?;
        Ok(Uinput { keyboard, mouse, pointer, pads: HashMap::new() })
    }

    fn pad(&mut self, pad: u8) -> anyhow::Result<&mut VirtualDevice> {
        if pad >= MAX_PADS {
            anyhow::bail!("gamepad {pad} out of range");
        }
        if !self.pads.contains_key(&pad) {
            let mut keys = AttributeSet::<Key>::new();
            for b in PAD_BUTTONS {
                keys.insert(Key::new(b));
            }
            let stick = |a| abs(a, -32768, 32767);
            let dev = VirtualDeviceBuilder::new()?
                .name("Microsoft X-Box 360 pad (DCL Stream)")
                .input_id(InputId::new(BusType::BUS_USB, 0x045e, 0x028e, 0x0110))
                .with_keys(&keys)?
                .with_absolute_axis(&stick(AbsoluteAxisType::ABS_X))?
                .with_absolute_axis(&stick(AbsoluteAxisType::ABS_Y))?
                .with_absolute_axis(&stick(AbsoluteAxisType::ABS_RX))?
                .with_absolute_axis(&stick(AbsoluteAxisType::ABS_RY))?
                .with_absolute_axis(&abs(AbsoluteAxisType::ABS_Z, 0, 255))?
                .with_absolute_axis(&abs(AbsoluteAxisType::ABS_RZ, 0, 255))?
                .with_absolute_axis(&abs(AbsoluteAxisType::ABS_HAT0X, -1, 1))?
                .with_absolute_axis(&abs(AbsoluteAxisType::ABS_HAT0Y, -1, 1))?
                .build()?;
            self.pads.insert(pad, dev);
        }
        Ok(self.pads.get_mut(&pad).expect("just inserted"))
    }
}

fn mouse_code(b: MouseButton) -> u16 {
    match b {
        MouseButton::Left => Key::BTN_LEFT.code(),
        MouseButton::Right => Key::BTN_RIGHT.code(),
        MouseButton::Middle => Key::BTN_MIDDLE.code(),
        MouseButton::Back => Key::BTN_SIDE.code(),
        MouseButton::Forward => Key::BTN_EXTRA.code(),
    }
}

impl InputSink for Uinput {
    fn event(&mut self, e: InputEvent) -> anyhow::Result<()> {
        match e {
            InputEvent::Key { code, down } if (1..=KEY_MAX_KEYBOARD).contains(&code) => {
                self.keyboard.emit(&[Ev::new(EventType::KEY, code, down as i32)])?;
            }
            InputEvent::Key { .. } => {}
            InputEvent::MouseButton { button, down } => {
                self.mouse.emit(&[Ev::new(EventType::KEY, mouse_code(button), down as i32)])?;
            }
            InputEvent::Scroll { dx, dy } => {
                let mut ev = Vec::new();
                if dy != 0 {
                    // client: positive dy = scroll down; Linux: positive REL_WHEEL = up
                    ev.push(Ev::new(EventType::RELATIVE, RelativeAxisType::REL_WHEEL_HI_RES.0, -(dy as i32)));
                    ev.push(Ev::new(EventType::RELATIVE, RelativeAxisType::REL_WHEEL.0, -(dy as i32) / 120));
                }
                if dx != 0 {
                    ev.push(Ev::new(EventType::RELATIVE, RelativeAxisType::REL_HWHEEL_HI_RES.0, dx as i32));
                    ev.push(Ev::new(EventType::RELATIVE, RelativeAxisType::REL_HWHEEL.0, dx as i32 / 120));
                }
                self.mouse.emit(&ev)?;
            }
            InputEvent::GamepadConnect { pad } => {
                self.pad(pad)?;
            }
            InputEvent::GamepadDisconnect { pad } => {
                self.pads.remove(&pad);
            }
            InputEvent::GamepadButton { pad, button, down } if PAD_BUTTONS.contains(&button) => {
                self.pad(pad)?.emit(&[Ev::new(EventType::KEY, button, down as i32)])?;
            }
            InputEvent::GamepadButton { .. } => {}
        }
        Ok(())
    }

    fn motion(&mut self, d: InputDatagram) -> anyhow::Result<()> {
        match d {
            InputDatagram::MouseMove { dx, dy, .. } => {
                self.mouse.emit(&[
                    Ev::new(EventType::RELATIVE, RelativeAxisType::REL_X.0, dx as i32),
                    Ev::new(EventType::RELATIVE, RelativeAxisType::REL_Y.0, dy as i32),
                ])?;
            }
            InputDatagram::MouseAbs { x, y, .. } => {
                self.pointer.emit(&[
                    Ev::new(EventType::ABSOLUTE, AbsoluteAxisType::ABS_X.0, x as i32),
                    Ev::new(EventType::ABSOLUTE, AbsoluteAxisType::ABS_Y.0, y as i32),
                ])?;
            }
            InputDatagram::GamepadAxes { pad, axes, hat_x, hat_y, .. } => {
                let a = [
                    AbsoluteAxisType::ABS_X,
                    AbsoluteAxisType::ABS_Y,
                    AbsoluteAxisType::ABS_RX,
                    AbsoluteAxisType::ABS_RY,
                    AbsoluteAxisType::ABS_Z,
                    AbsoluteAxisType::ABS_RZ,
                ];
                let mut ev: Vec<Ev> = a
                    .iter()
                    .zip(axes)
                    .enumerate()
                    .map(|(i, (ax, v))| {
                        let v = if i >= 4 { (v as i32).clamp(0, 255) } else { v as i32 };
                        Ev::new(EventType::ABSOLUTE, ax.0, v)
                    })
                    .collect();
                ev.push(Ev::new(EventType::ABSOLUTE, AbsoluteAxisType::ABS_HAT0X.0, (hat_x as i32).clamp(-1, 1)));
                ev.push(Ev::new(EventType::ABSOLUTE, AbsoluteAxisType::ABS_HAT0Y.0, (hat_y as i32).clamp(-1, 1)));
                self.pad(pad)?.emit(&ev)?;
            }
        }
        Ok(())
    }
}
