extern crate btleplug;

use btleplug::api::{Central, Characteristic, Peripheral, UUID};
#[cfg(target_os = "linux")]
use btleplug::bluez::{
    adapter::ConnectedAdapter, manager::Manager, peripheral::Peripheral as PeripheralStruct,
};
#[cfg(target_os = "macos")]
use btleplug::corebluetooth::{
    adapter::Adapter, manager::Manager, peripheral::Peripheral as PeripheralStruct,
};
#[cfg(target_os = "windows")]
use btleplug::winrtble::{
    adapter::Adapter, manager::Manager, peripheral::Peripheral as PeripheralStruct,
};
use std::cmp::{max, min, Ordering};
use std::thread;
use std::time::Duration;

// adapter retreival works differently depending on your platform right now.
// API needs to be aligned.

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn get_central(manager: &Manager) -> Adapter {
    let adapters = manager.adapters().unwrap();
    adapters.into_iter().next().unwrap()
}

#[cfg(target_os = "linux")]
fn get_central(manager: &Manager) -> ConnectedAdapter {
    let adapters = manager.adapters().unwrap();
    let adapter = adapters.into_iter().next().unwrap();
    adapter.connect().unwrap()
}

const CONTROL_UUID: UUID = UUID::B128([
    0x8a, 0xf7, 0x15, 0x02, 0x9c, 0x00, 0x49, 0x8a, 0x24, 0x10, 0x8a, 0x33, 0x02, 0x00, 0xfa, 0x99,
]);

const POSITION_UUID: UUID = UUID::B128([
    0x8a, 0xf7, 0x15, 0x02, 0x9c, 0x00, 0x49, 0x8a, 0x24, 0x10, 0x8a, 0x33, 0x21, 0x00, 0xfa, 0x99,
]);

const UP: [u8; 2] = [0x47, 0x00];
const DOWN: [u8; 2] = [0x46, 0x00];
const STOP: [u8; 2] = [0xFF, 0x00];

pub const MIN_HEIGHT: i16 = 6200;
pub const MAX_HEIGHT: i16 = 12700;

/// convert desk response from bytes to meters
///
/// ```
/// assert_eq!(idasen::bytes_to_tenth_milimeters(&[0x64, 0x19, 0x00, 0x00]), idasen::MAX_HEIGHT);
/// assert_eq!(idasen::bytes_to_tenth_milimeters(&[0x00, 0x00, 0x00, 0x00]), idasen::MIN_HEIGHT);
/// assert_eq!(idasen::bytes_to_tenth_milimeters(&[0x51, 0x04, 0x00, 0x00]), 7305);
/// assert_eq!(idasen::bytes_to_tenth_milimeters(&[0x08, 0x08, 0x00, 0x00]), 8256);
/// assert_eq!(idasen::bytes_to_tenth_milimeters(&[0x64, 0x18, 0x00, 0x00]), 12444);
/// ```
pub fn bytes_to_tenth_milimeters(bytes: &[u8]) -> i16 {
    let as_int = ((bytes[1] as i16) << 8) + bytes[0] as i16;
    as_int + MIN_HEIGHT
}

pub struct Idasen {
    desk: PeripheralStruct,
    control_characteristic: Characteristic,
    position_characteristic: Characteristic,
}

pub enum Error {
    PositionNotInRange,
}

impl Idasen {
    pub fn new() -> Self {
        let manager = Manager::new().unwrap();
        let central = get_central(&manager);
        central.start_scan().unwrap();

        thread::sleep(Duration::from_secs(2));

        let desk = central
            .peripherals()
            .into_iter()
            .find(|p| {
                p.properties()
                    .local_name
                    .iter()
                    .any(|name| name.contains("Desk"))
            })
            .unwrap();
        desk.connect().unwrap();

        let characteristics = desk.discover_characteristics().unwrap();
        let control_characteristic = characteristics
            .iter()
            .find(|characteristic| characteristic.uuid == CONTROL_UUID)
            .unwrap()
            .clone();
        let position_characteristic = characteristics
            .iter()
            .find(|characteristics| characteristics.uuid == POSITION_UUID)
            .unwrap()
            .clone();

        Self {
            desk,
            control_characteristic,
            position_characteristic,
        }
    }

    /// Move desk up.
    pub fn up(&self) -> btleplug::Result<()> {
        self.desk.command(&self.control_characteristic, &UP)
    }

    /// Lower the desk's position.
    pub fn down(&self) -> btleplug::Result<()> {
        self.desk.command(&self.control_characteristic, &DOWN)
    }

    /// Stop desk from moving.
    pub fn stop(&self) -> btleplug::Result<()> {
        self.desk.command(&self.control_characteristic, &STOP)
    }

    /// Move desk to a desired position. The precision is decent, usually less than 1mm off.
    pub fn move_to(&self, target_position: i16) -> Result<(), Error> {
        if target_position < MIN_HEIGHT || target_position > MAX_HEIGHT {
            return Err(Error::PositionNotInRange);
        }

        let going_up = match target_position.cmp(&self.position()) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => return Ok(()),
        };

        let mut position_reached = false;
        let mut last_position = self.position();
        let mut speed;
        while !position_reached {
            let current_position = self.position();
            let remaining_distance = (target_position - current_position).abs();
            speed = (last_position - current_position).abs();
            if remaining_distance <= min(speed, 5) {
                position_reached = true;
                let _ = self.stop();
            } else if going_up {
                let _ = self.up();
            } else if !going_up {
                let _ = self.down();
            }
            if remaining_distance < max(speed * 10, 10) {
                let _ = self.stop();
            }
            last_position = current_position;
        }

        Ok(())
    }

    /// Return the desk height in tenth milimeters (1m = 10000)
    pub fn position(&self) -> i16 {
        let response = self.desk.read_by_type(
            &self.position_characteristic,
            self.position_characteristic.uuid,
        );
        bytes_to_tenth_milimeters(&response.unwrap())
    }
}
