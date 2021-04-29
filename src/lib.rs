extern crate btleplug;
extern crate failure;
#[macro_use]
extern crate failure_derive;

pub use btleplug::api::Peripheral as Device;
use uuid::Uuid;
use btleplug::api::{BDAddr, Central, Characteristic, ParseBDAddrError, WriteType};
#[cfg(target_os = "linux")]
use btleplug::bluez::{adapter::Adapter, manager::Manager};
#[cfg(target_os = "macos")]
use btleplug::corebluetooth::{adapter::Adapter, manager::Manager};
#[cfg(target_os = "windows")]
use btleplug::winrtble::{adapter::Adapter, manager::Manager};
use indicatif::{ProgressBar, ProgressStyle};
use std::thread;
use std::time::Duration;
use std::{
    cmp::{max, Ordering},
    time::Instant,
};

const CONTROL_UUID: Uuid = Uuid::from_bytes([
    0x99, 0xfa, 0x00, 0x02, 0x33, 0x8a, 0x10, 0x24, 0x8a, 0x49, 0x00, 0x9c, 0x02, 0x15, 0xf7, 0x8a,
]);

const POSITION_UUID: Uuid = Uuid::from_bytes([
    0x99, 0xfa, 0x00, 0x21, 0x33, 0x8a, 0x10, 0x24, 0x8a, 0x49, 0x00, 0x9c, 0x02, 0x15, 0xf7, 0x8a,
]);

const UP: [u8; 2] = [0x47, 0x00];
const DOWN: [u8; 2] = [0x46, 0x00];
const STOP: [u8; 2] = [0xFF, 0x00];

pub const MIN_HEIGHT: u16 = 6200;
pub const MAX_HEIGHT: u16 = 12700;

/// convert desk response from bytes to meters
///
/// ```
/// assert_eq!(idasen::bytes_to_tenth_millimeters(&[0x64, 0x19, 0x00, 0x00]), idasen::MAX_HEIGHT);
/// assert_eq!(idasen::bytes_to_tenth_millimeters(&[0x00, 0x00, 0x00, 0x00]), idasen::MIN_HEIGHT);
/// assert_eq!(idasen::bytes_to_tenth_millimeters(&[0x51, 0x04, 0x00, 0x00]), 7305);
/// assert_eq!(idasen::bytes_to_tenth_millimeters(&[0x08, 0x08, 0x00, 0x00]), 8256);
/// assert_eq!(idasen::bytes_to_tenth_millimeters(&[0x64, 0x18, 0x00, 0x00]), 12444);
/// ```
pub fn bytes_to_tenth_millimeters(bytes: &[u8]) -> u16 {
    let as_int = ((bytes[1] as u16) << 8) + bytes[0] as u16;
    as_int + MIN_HEIGHT
}

#[derive(Debug, Fail)]
pub enum Error {
    #[fail(display = "Cannot find the device.")]
    CannotFindDevice,

    #[fail(display = "Cannot connect to the device.")]
    ConnectionFailed,

    #[fail(display = "Cannot scan for devices.")]
    ScanFailed,

    #[fail(display = "Permission denied.")]
    PermissionDenied,

    #[fail(display = "Cannot discover Bluetooth characteristics.")]
    CharacteristicsDiscoveryFailed,

    #[fail(display = "Bluetooth characteristics not found: '{}'.", _0)]
    CharacteristicsNotFound(String),

    #[fail(display = "Desired position has to be between MIN_HEIGHT and MAX_HEIGHT.")]
    PositionNotInRange,

    #[fail(display = "Cannot subscribe to read position.")]
    CannotSubscribePosition,

    #[fail(display = "Cannot read position.")]
    CannotReadPosition,

    #[fail(display = "Failed to parse mac address.")]
    MacAddrParseFailed(ParseBDAddrError),
}

fn get_desk(mac: Option<BDAddr>) -> Result<impl Device, Error> {
    let manager = Manager::new().unwrap();
    let adapters = manager.adapters().unwrap();
    let central = adapters.into_iter().next().unwrap();
    if let Err(err) = central.start_scan() {
        return Err(match err {
            btleplug::Error::PermissionDenied => Error::PermissionDenied,
            _ => Error::ScanFailed,
        });
    };

    let desk = find_desk(central, mac);
    if desk.is_none() {
        return Err(Error::CannotFindDevice);
    }
    let desk = desk.unwrap();
    if desk.connect().is_err() {
        return Err(Error::ConnectionFailed);
    }
    Ok(desk)
}

fn find_desk(central: Adapter, mac: Option<BDAddr>) -> Option<impl Device> {
    let mut attempt = 0;
    while attempt < 240 {
        let desk = central.peripherals().into_iter().find(|p| match mac {
            Some(mac) => p.properties().address == mac,
            None => p
                .properties()
                .local_name
                .iter()
                .any(|name| name.contains("Desk")),
        });
        if desk.is_some() {
            return desk;
        }
        attempt += 1;
        thread::sleep(Duration::from_millis(50));
    }
    None
}

/// Get instance of `Idasen` struct. The desk will be discovered by the name.
pub fn get_instance() -> Result<Idasen<impl Device>, Error> {
    let desk = get_desk(None)?;
    Idasen::new(desk)
}

/// Get the desk instance by it's Bluetooth MAC address (BD_ADDR).
/// The address can be obtained also by accessing `mac_addr` property
/// on instantiated `Idasen` instance.
pub fn get_instance_by_mac(mac: &str) -> Result<Idasen<impl Device>, Error> {
    let addr = mac.parse::<BDAddr>();
    match addr {
        Ok(addr) => {
            let desk = get_desk(Some(addr))?;
            Ok(Idasen::new(desk)?)
        }
        Err(err) => Err(Error::MacAddrParseFailed(err)),
    }
}

pub struct Idasen<T>
where
    T: Device,
{
    pub mac_addr: BDAddr,
    desk: T,
    control_characteristic: Characteristic,
    position_characteristic: Characteristic,
}

impl<T: Device> Idasen<T> {
    /// Instantiate the struct. Requires `Device` instance.
    pub fn new(desk: T) -> Result<Self, Error> {
        let mac_addr = desk.address();

        let characteristics = desk.discover_characteristics();
        if characteristics.is_err() {
            return Err(Error::CharacteristicsDiscoveryFailed);
        };
        let characteristics = characteristics.unwrap();

        let control_characteristic = characteristics
            .iter()
            .find(|characteristic| characteristic.uuid == CONTROL_UUID);
        if control_characteristic.is_none() {
            return Err(Error::CharacteristicsNotFound("Control".to_string()));
        }
        let control_characteristic = control_characteristic.unwrap().clone();

        let position_characteristic = characteristics
            .iter()
            .find(|characteristics| characteristics.uuid == POSITION_UUID);
        if position_characteristic.is_none() {
            return Err(Error::CharacteristicsNotFound("Position".to_string()));
        }
        let position_characteristic = position_characteristic.unwrap().clone();
        if desk.subscribe(&position_characteristic).is_err() {
            return Err(Error::CannotSubscribePosition)
        };

        Ok(Self {
            desk,
            mac_addr,
            control_characteristic,
            position_characteristic,
        })
    }

    /// Move desk up.
    pub fn up(&self) -> btleplug::Result<()> {
        self.desk.write(&self.control_characteristic, &UP, WriteType::WithoutResponse)
    }

    /// Lower the desk's position.
    pub fn down(&self) -> btleplug::Result<()> {
        self.desk.write(&self.control_characteristic, &DOWN, WriteType::WithoutResponse)
    }

    /// Stop desk from moving.
    pub fn stop(&self) -> btleplug::Result<()> {
        self.desk.write(&self.control_characteristic, &STOP, WriteType::WithoutResponse)
    }

    /// Move desk to a desired position. The precision is decent, usually less than 1mm off.
    pub fn move_to(&self, target_position: u16) -> Result<(), Error> {
        self.move_to_target(target_position, None)
    }

    pub fn move_to_with_progress(&self, target_position: u16) -> Result<(), Error> {
        let initial_position = (target_position as i16 - self.position()? as i16).abs();
        let progress = ProgressBar::new(initial_position as u64);
        progress.set_style(ProgressStyle::default_bar().template("{spinner} {wide_bar} [{msg}cm]"));
        self.move_to_target(target_position, Some(progress))
    }

    fn move_to_target(
        &self,
        target_position: u16,
        progress: Option<ProgressBar>,
    ) -> Result<(), Error> {
        if !(MIN_HEIGHT..=MAX_HEIGHT).contains(&target_position) {
            return Err(Error::PositionNotInRange);
        }

        let mut position_reached = false;
        let mut last_position = self.position()? as i16;
        let mut last_position_read_at = Instant::now();
        let target_position = target_position as i16;
        while !position_reached {
            let current_position = self.position()? as i16;
            let going_up = match target_position.cmp(&current_position) {
                Ordering::Greater => true,
                Ordering::Less => false,
                Ordering::Equal => return Ok(()),
            };
            let remaining_distance = (target_position - current_position).abs();
            let elapsed_millis = last_position_read_at.elapsed().as_millis();
            let moved_height = (last_position - current_position).abs();

            // Tenth of millimetres per second
            let speed = ((moved_height as f64 / elapsed_millis as f64) * 1000f64) as i16;

            if let Some(ref progress) = progress {
                progress.inc(speed as u64);
                let position_cm = current_position as f32 / 100.0;
                progress.set_message(format!("{}", position_cm).as_str());
            }

            if remaining_distance <= 10 {
                // Millimetre or less is good enough.
                position_reached = true;
                let _ = self.stop();
            } else if going_up {
                let _ = self.up();
            } else if !going_up {
                let _ = self.down();
            }

            // If we're either:
            // * less than 5 millimetres, or:
            // * less than half a second from target
            // then we need to stop every iteration so that we don't overshoot
            if remaining_distance < max(speed / 2, 50) {
                let _ = self.stop();
            }

            // Read last_position again to avoid weird speed readings when switching direction
            last_position = self.position()? as i16;
            last_position_read_at = Instant::now();
        }

        if let Some(progress) = progress {
            progress.finish();
        }

        Ok(())
    }

    /// Return the desk height in tenth millimeters (1m = 10000)
    pub fn position(&self) -> Result<u16, Error> {
        let response = self.desk.read(&self.position_characteristic);
        match response {
            Ok(value) => Ok(bytes_to_tenth_millimeters(&value)),
            Err(_) => Err(Error::CannotReadPosition),
        }
    }
}
