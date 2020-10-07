extern crate btleplug;
extern crate failure;
#[macro_use]
extern crate failure_derive;

pub use btleplug::api::Peripheral as Device;
use btleplug::api::{BDAddr, Central, Characteristic, ParseBDAddrError, UUID};
#[cfg(target_os = "linux")]
use btleplug::bluez::{adapter::ConnectedAdapter as Adapter, manager::Manager};
#[cfg(target_os = "macos")]
use btleplug::corebluetooth::{adapter::Adapter, manager::Manager};
#[cfg(target_os = "windows")]
use btleplug::winrtble::{adapter::Adapter, manager::Manager};
use indicatif::{ProgressBar, ProgressStyle};
use std::cmp::{max, min, Ordering};
use std::thread;
use std::thread::current;
use std::time::Duration;

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn get_central(manager: &Manager) -> Adapter {
    let adapters = manager.adapters().unwrap();
    adapters.into_iter().next().unwrap()
}

#[cfg(target_os = "linux")]
fn get_central(manager: &Manager) -> Adapter {
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

    #[fail(display = "Cannot read position.")]
    CannotReadPosition,

    #[fail(display = "Failed to parse mac address.")]
    MacAddrParseFailed(ParseBDAddrError),
}

fn get_desk(mac: Option<BDAddr>) -> Result<impl Device, Error> {
    let manager = Manager::new().unwrap();
    let central = get_central(&manager);
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
    while attempt < 120 {
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
    Ok(Idasen::new(desk)?)
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

        Ok(Self {
            desk,
            mac_addr,
            control_characteristic,
            position_characteristic,
        })
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
        if target_position < MIN_HEIGHT || target_position > MAX_HEIGHT {
            return Err(Error::PositionNotInRange);
        }

        let going_up = match target_position.cmp(&self.position()?) {
            Ordering::Greater => true,
            Ordering::Less => false,
            Ordering::Equal => return Ok(()),
        };

        let mut position_reached = false;
        let mut last_position = self.position()? as i16;
        let mut speed;
        let target_position = target_position as i16;
        while !position_reached {
            let current_position = self.position()? as i16;
            let remaining_distance = (target_position - current_position).abs();
            speed = (last_position - current_position).abs();
            if let Some(ref progress) = progress {
                progress.inc(speed as u64);
                let position_cm = current_position as f32 / 100.0;
                progress.set_message(format!("{}", position_cm).as_str());
            }
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

        if let Some(progress) = progress {
            progress.finish();
        }

        Ok(())
    }

    /// Return the desk height in tenth millimeters (1m = 10000)
    pub fn position(&self) -> Result<u16, Error> {
        let response = self.desk.read_by_type(
            &self.position_characteristic,
            self.position_characteristic.uuid,
        );
        match response {
            Ok(value) => Ok(bytes_to_tenth_millimeters(&value)),
            Err(_) => Err(Error::CannotReadPosition),
        }
    }
}
