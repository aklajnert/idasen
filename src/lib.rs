extern crate btleplug;

const CONTROL: &str = "99fa0002338a10248a49009c0215f78a";

use btleplug::api::{Central, CharPropFlags, Characteristic, Peripheral, UUID};
#[cfg(target_os = "linux")]
use btleplug::bluez::{adapter::ConnectedAdapter, manager::Manager};
#[cfg(target_os = "macos")]
use btleplug::corebluetooth::{adapter::Adapter, manager::Manager};
#[cfg(target_os = "windows")]
use btleplug::winrtble::{adapter::Adapter, manager::Manager};
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

pub fn main() {
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
    for char in &characteristics {
        println!("{:?}", char.uuid);
    }

    println!("{:?}", desk.characteristics());
}
