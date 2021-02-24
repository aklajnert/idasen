# Idasen

Control IKEA IDÅSEN standing desk via Bluetooth.  

[![Crates.io Version](https://img.shields.io/crates/v/idasen)](https://crates.io/crates/idasen)
[![Crates.io License](https://img.shields.io/crates/l/idasen)](https://crates.io/crates/idasen)

## Usage

```rust
use idasen::{get_instance, Idasen, Device};

// instantiate the struct, this will attempt to connect to the desk 
// and discover its characteristics
let desk: Idasen<impl Device> = get_instance()?;

// alternatively, if there's more than one desk you can get the 
// correct one by it's mac address 
// for some reason, using MAC seems to be more reliable when it 
// comes to device discovering:
// let desk = get_instance_by_mac("EC:86:F6:44:D3:31")?;

// move desk up and down
desk.up();
desk.down();

// stop desk from moving
desk.stop();

// move desk to desired position: 
//  minimum: 6200 (62cm), maximum: 12700 (1.27m)
desk.move_to(7400);

// get the position as an integer (10 = 1mm)
println!("Position: {}", desk.position()?);
```
