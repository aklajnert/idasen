extern crate btleplug;

use btleplug::api::{Central, Characteristic, Peripheral, UUID};
#[cfg(target_os = "linux")]
use btleplug::bluez::{adapter::ConnectedAdapter, manager::Manager};
#[cfg(target_os = "macos")]
use btleplug::corebluetooth::{adapter::Adapter, manager::Manager};
#[cfg(target_os = "windows")]
use btleplug::winrtble::{adapter::Adapter, manager::Manager};
use std::collections::HashMap;
use std::thread;
use std::time::Duration;

// adapter retreival works differently depending on your platform right now.
// API needs to be aligned.

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn get_central(manager: &Manager) -> Adapter {
    let adapters = manager.adapters().unwrap();
    adapters.into_iter().nth(0).unwrap()
}

#[cfg(target_os = "linux")]
fn get_central(manager: &Manager) -> ConnectedAdapter {
    let adapters = manager.adapters().unwrap();
    let adapter = adapters.into_iter().nth(0).unwrap();
    adapter.connect().unwrap()
}

const CONTROL_UUID: UUID = UUID::B128([
    0x8a, 0xf7, 0x15, 0x02, 0x9c, 0x00, 0x49, 0x8a, 0x24, 0x10, 0x8a, 0x33, 0x02, 0x00, 0xfa, 0x99,
]);

const STATUS_UUID: UUID = UUID::B128([
    0x8a, 0xf7, 0x15, 0x02, 0x9c, 0x00, 0x49, 0x8a, 0x24, 0x10, 0x8a, 0x33, 0x21, 0x00, 0xfa, 0x99,
]);
const POSITION_UUID: UUID = UUID::B128([
    0x8a, 0xf7, 0x15, 0x02, 0x9c, 0x00, 0x49, 0x8a, 0x24, 0x10, 0x8a, 0x33, 0x20, 0x00, 0xfa, 0x99,
]);

const UP: [u8; 2] = [0x47, 0x00];
const DOWN: [u8; 2] = [0x46, 0x00];
const STOP: [u8; 2] = [0xFF, 0x00];

const MIN_HEIGHT: f32 = 0.62;
const MAX_HEIGHT: f32 = 1.27;

fn bytes_to_meters(bytes: [u8; 4]) -> f32 {
    0.0
}

pub fn main() -> Vec<Characteristic> {
    let manager = Manager::new().unwrap();

    // get the first bluetooth adapter
    //
    // connect to the adapter
    let central = get_central(&manager);

    // start scanning for devices
    central.start_scan().unwrap();
    // instead of waiting, you can use central.event_receiver() to fetch a channel and
    // be notified of new devices
    thread::sleep(Duration::from_secs(2));

    // find the device we're interested in
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
    println!("{:?}", desk);

    // connect to the device
    desk.connect().unwrap();

    // discover characteristics
    let characteristics = desk.discover_characteristics().unwrap();
    let control_characteristic = characteristics
        .iter()
        .find(|characteristic| characteristic.uuid == CONTROL_UUID)
        .unwrap();
    let status_characteristic = characteristics
        .iter()
        .find(|characteristics| characteristics.uuid == STATUS_UUID)
        .unwrap();

    println!("{:?}", desk.command(&control_characteristic, &UP));
    println!("{:?}", desk.command(&control_characteristic, &STOP));
    thread::sleep(Duration::from_secs(1));
    println!("{:?}", desk.command(&control_characteristic, &DOWN));
    println!("{:?}", desk.command(&control_characteristic, &STOP));

    loop {
        let response = desk.read_by_type(&status_characteristic, status_characteristic.uuid);
        println!("H: {:?}", response);
        thread::sleep(Duration::from_secs(1));
    }

    characteristics
}
