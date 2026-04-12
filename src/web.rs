use rppal::{
    self,
    gpio::{Gpio, Level, Trigger},
    i2c::I2c,
    system::DeviceInfo,
};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicI32, Ordering},
    mpsc::{self, Sender},
};
use std::thread;
use std::time::Duration;
use std::{collections::HashMap, error::Error};
//use linux_embedded_hal::I2cdev;
use anyhow::{Result, bail};
use embedded_devices::devices::texas_instruments::ina228::{
    INA228Sync,
    address::{Address, Pin},
};
use embedded_devices::sensor::OneshotSensorSync;
use embedded_hal::delay::DelayNs;
use embedded_hal_bus::i2c::MutexDevice;
use embedded_hal_compat::ReverseCompat;
use embedded_interfaces::i2c::I2cDeviceSync;
use mcp230xx::{self, Direction, Mcp230xx, Mcp23017};
use uom::si::electric_current::{ampere, milliampere};
use uom::si::electric_potential::volt;
use uom::si::electrical_resistance::ohm;
use uom::si::f64::{ElectricCurrent, ElectricalResistance};
use uom::si::thermodynamic_temperature::degree_celsius;

const MCP23017_ADDRESS: u8 = 0x20;

#[derive(Clone, Debug, Default)]
pub struct Encoder {
    pub pin_a: u8,
    pub pin_b: u8,
    pub stop: Arc<Mutex<bool>>,
    pub count: Arc<AtomicI32>,
}

impl Encoder {
    pub fn new(pina: u8, pinb: u8) -> Self {
        Self {
            pin_a: pina,
            pin_b: pinb,
            stop: Arc::new(Mutex::new(false)),
            count: Arc::new(AtomicI32::new(0)),
        }
    }

    pub fn run(&mut self) -> Result<()> {
        let device_info = DeviceInfo::new().unwrap();
        println!(
            "Model: {} (SoC: {})",
            device_info.model(),
            device_info.soc()
        );

        let master_count = Arc::clone(&self.count);

        let pin_a = self.pin_a;
        let pin_b = self.pin_b;
        let stop = self.stop.clone();

        thread::spawn(move || {
            let gpio = Gpio::new().unwrap();

            // Keep these pin objects alive for the lifetime of the thread.
            let mut pin1 = gpio.get(pin_a).unwrap().into_input_pullup(); // A
            let pin2 = gpio.get(pin_b).unwrap().into_input_pullup(); // B

            // Interrupt ONLY on pin1 (A), read pin2 (B) for direction.
            pin1.set_async_interrupt(Trigger::RisingEdge, None, move |_| {
                // Preserve your existing direction convention:
                // if B is Low at A rising -> +1 else -1
                if let Level::Low = pin2.read() {
                    master_count.fetch_add(1, Ordering::Relaxed);
                } else {
                    master_count.fetch_add(-1, Ordering::Relaxed);
                }
            })
            .unwrap();

            // Keep thread alive; stop flag ends it cleanly.
            loop {
                if *stop.lock().unwrap() {
                    // Stop interrupts before exiting the thread.
                    println!("Stop Encoder");
                    pin1.clear_async_interrupt().ok();
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            }
        });

        Ok(())
    }

    pub fn enc(&self) -> i32 {
        self.count.load(Ordering::Relaxed)
    }
}

#[derive(Clone)]
pub struct Stepper {
    pub name: String,
    pub channel: Option<Sender<(u32, bool, bool)>>,
    pub pin_a: Option<u8>,
    pub pin_b: Option<u8>,
    pub ena: Option<u8>,
    pub ratio: u8,
    pub pos: Arc<AtomicI32>,
    pub mem: HashMap<String, Arc<AtomicI32>>,
    pub max: Arc<AtomicI32>,
    pub speed: Duration,
    pub operate: Arc<Mutex<bool>>,
}
impl Stepper {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            channel: None,
            pin_a: None,
            pin_b: None,
            ena: None,
            ratio: 1,
            pos: Arc::new(AtomicI32::new(0)),
            mem: HashMap::from([
                ("10M".to_string(), Arc::new(AtomicI32::new(0))),
                ("11M".to_string(), Arc::new(AtomicI32::new(0))),
                ("20M".to_string(), Arc::new(AtomicI32::new(0))),
                ("40M".to_string(), Arc::new(AtomicI32::new(0))),
                ("80M".to_string(), Arc::new(AtomicI32::new(0))),
            ]),
            max: Arc::new(AtomicI32::new(100000)),
            speed: Duration::from_micros(100),
            operate: Arc::new(Mutex::new(false)),
        }
    }
    pub fn run(&self, val: u32) {
        let pos: u32 = self.pos.load(Ordering::Relaxed) as u32;
        let gpio = Gpio::new().unwrap();
        let mut pulse_pin = gpio.get(self.pin_a.unwrap()).unwrap().into_output();
        let mut dir_pin = gpio.get(self.pin_b.unwrap()).unwrap().into_output();
        let mut count = 0;
        pulse_pin.set_low();
        if val > pos {
            dir_pin.set_high();
            while val > self.pos.load(Ordering::Relaxed) as u32 {
                count += 1;
                pulse_pin.set_high();
                thread::sleep(self.speed);
                pulse_pin.set_low();
                thread::sleep(self.speed);
                if count % 2 == 0 {
                    self.pos.fetch_add(1, Ordering::Relaxed);
                }
            }
        } else if val < pos {
            dir_pin.set_low();
            while val < self.pos.load(Ordering::Relaxed) as u32 {
                count += 1;
                pulse_pin.set_high();
                thread::sleep(self.speed);
                pulse_pin.set_low();
                thread::sleep(self.speed);
                if count % 2 == 0 {
                    self.pos.fetch_add(-1, Ordering::Relaxed);
                }
            }
        }
    }

    pub fn run_2(&mut self) {
        println!("Inside run 2");
        let (tx, rx) = mpsc::channel();
        self.channel = Some(tx);
        let gpio = Gpio::new().unwrap();
        let mut pulse_pin = gpio.get(self.pin_a.unwrap()).unwrap().into_output();
        let mut dir_pin = gpio.get(self.pin_b.unwrap()).unwrap().into_output();
        let mut steps_taken = 0;
        let pos = self.pos.clone();
        let mut speed = self.speed.clone();
        let operate = self.operate.clone();
        let name = self.name.clone();
        let get_speed = Stepper::calc_speed;
        thread::spawn(move || {
            loop {
                if let Ok((val, stop, manual)) = rx.recv() {
                    let mut target = val;
                    let mut manual_mode = manual;
                    while let Ok((next_val, next_stop, next_manual)) = rx.try_recv() {
                        if next_stop {
                            println!("Stopping stepper loop to delete stepper.");
                            return;
                        }
                        target = next_val;
                        manual_mode = next_manual;
                    }
                    if stop {
                        println!("Stopping stepper loop to delete stepper.");
                        break;
                    }
                    pulse_pin.set_low();
                    if target > pos.load(Ordering::Relaxed) as u32 {
                        *operate.lock().unwrap() = true;
                        dir_pin.set_high();
                        while target > pos.load(Ordering::Relaxed) as u32 {
                            if name == "ind" && !manual_mode {
                                speed = get_speed(
                                    target - pos.load(Ordering::Relaxed) as u32,
                                    steps_taken,
                                );
                            } else if name == "ind" && manual_mode {
                                speed = Duration::from_micros(200);
                            }
                            steps_taken += 1;
                            pulse_pin.set_high();
                            thread::sleep(speed);
                            pulse_pin.set_low();
                            thread::sleep(speed);
                            if steps_taken % 2 == 0 {
                                pos.fetch_add(1, Ordering::Relaxed);
                            }
                            while let Ok((next_val, next_stop, next_manual)) = rx.try_recv() {
                                if next_stop {
                                    println!("Stopping stepper loop to delete stepper.");
                                    pulse_pin.set_low();
                                    *operate.lock().unwrap() = false;
                                    return;
                                }
                                target = next_val;
                                manual_mode = next_manual;
                                if target < pos.load(Ordering::Relaxed) as u32 {
                                    dir_pin.set_low();
                                }
                            }
                        }
                        *operate.lock().unwrap() = false;
                    } else if target < pos.load(Ordering::Relaxed) as u32 {
                        *operate.lock().unwrap() = true;
                        dir_pin.set_low();
                        while target < pos.load(Ordering::Relaxed) as u32 {
                            if name == "ind" && !manual_mode {
                                speed = get_speed(
                                    pos.load(Ordering::Relaxed) as u32 - target,
                                    steps_taken,
                                );
                            } else if name == "ind" && manual_mode {
                                speed = Duration::from_micros(200);
                            }
                            steps_taken += 1;
                            pulse_pin.set_high();
                            thread::sleep(speed);
                            pulse_pin.set_low();
                            thread::sleep(speed);
                            if steps_taken % 2 == 0 {
                                pos.fetch_add(-1, Ordering::Relaxed);
                            }
                            while let Ok((next_val, next_stop, next_manual)) = rx.try_recv() {
                                if next_stop {
                                    println!("Stopping stepper loop to delete stepper.");
                                    pulse_pin.set_low();
                                    *operate.lock().unwrap() = false;
                                    return;
                                }
                                target = next_val;
                                manual_mode = next_manual;
                                if target > pos.load(Ordering::Relaxed) as u32 {
                                    dir_pin.set_high();
                                }
                            }
                        }
                        *operate.lock().unwrap() = false;
                    }
                    steps_taken = 0;
                }
            }
        });
    }
    fn calc_speed(remaining_steps: u32, steps_taken: i32) -> Duration {
        fn delay_for_window(steps: u32) -> Duration {
            match steps {
                0..20 => Duration::from_micros(5000),
                20..50 => Duration::from_micros(3500),
                50..100 => Duration::from_micros(2500),
                100..180 => Duration::from_micros(1600),
                180..300 => Duration::from_micros(1100),
                300..450 => Duration::from_micros(800),
                450..650 => Duration::from_micros(600),
                650..900 => Duration::from_micros(400),
                900..1200 => Duration::from_micros(300),
                1200_u32..=u32::MAX => Duration::from_micros(200),
            }
        }

        let accel_window = if steps_taken <= 0 {
            0
        } else {
            steps_taken as u32
        };
        let accel_delay = delay_for_window(accel_window);
        let decel_delay = delay_for_window(remaining_steps);

        // The slower side wins so the inductor never accelerates beyond
        // what the remaining stopping distance can safely handle.
        accel_delay.max(decel_delay)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StdDelay;

impl DelayNs for StdDelay {
    fn delay_ns(&mut self, ns: u32) {
        // Good enough for device init delays on Linux
        std::thread::sleep(Duration::from_nanos(ns as u64));
    }

    fn delay_us(&mut self, us: u32) {
        std::thread::sleep(Duration::from_micros(us as u64));
    }

    fn delay_ms(&mut self, ms: u32) {
        std::thread::sleep(Duration::from_millis(ms as u64));
    }
}
#[derive(Clone)]
pub struct Mcp {
    pub all_pins: [Mcp23017; 16],
    pub pins: HashMap<String, Mcp23017>,
    pub bus: Arc<Mutex<I2c>>,
    pub device_list: Vec<u16>,
    pub mcp_present: bool,
    pub message: String,
    pub switch: HashMap<String, String>,
}
impl Mcp {
    // default function that sets all pins as output.
    pub fn new() -> Self {
        //let i2c= I2cdev::new("/dev/i2c-1").unwrap();
        let all_pins = [
            Mcp23017::A0,
            Mcp23017::A1,
            Mcp23017::A2,
            Mcp23017::A3,
            Mcp23017::A4,
            Mcp23017::A5,
            Mcp23017::A6,
            Mcp23017::A7,
            Mcp23017::B0,
            Mcp23017::B1,
            Mcp23017::B2,
            Mcp23017::B3,
            Mcp23017::B4,
            Mcp23017::B5,
            Mcp23017::B6,
            Mcp23017::B7,
        ];
        let mut i2c = Self::open_i2c_with_retry();
        let mut devices: Vec<u16> = Vec::new();
        let mcp_present = Self::probe_mcp(&mut i2c);
        if mcp_present {
            devices.push(MCP23017_ADDRESS.into());
        }

        Self {
            all_pins,
            bus: Arc::new(Mutex::new(i2c)),
            device_list: devices,
            mcp_present,
            pins: HashMap::from([
                ("A0".to_string(), Mcp23017::A0),
                ("A1".to_string(), Mcp23017::A1),
                ("A2".to_string(), Mcp23017::A2),
                ("A3".to_string(), Mcp23017::A3),
                ("A4".to_string(), Mcp23017::A4),
                ("A5".to_string(), Mcp23017::A5),
                ("A6".to_string(), Mcp23017::A6),
                ("A7".to_string(), Mcp23017::A7),
                ("B0".to_string(), Mcp23017::B0),
                ("B1".to_string(), Mcp23017::B1),
                ("B2".to_string(), Mcp23017::B2),
                ("B3".to_string(), Mcp23017::B3),
                ("B4".to_string(), Mcp23017::B4),
                ("B5".to_string(), Mcp23017::B5),
                ("B6".to_string(), Mcp23017::B6),
                ("B7".to_string(), Mcp23017::B7),
            ]),
            message: if mcp_present {
                String::from("MCP Intioalized ! ! !")
            } else {
                String::from("MCP not detected")
            },
            switch: HashMap::new(),
        }
    }
    fn probe_mcp(i2c: &mut I2c) -> bool {
        println!("Checking for MCP23017 at 0x{:02X}...", MCP23017_ADDRESS);
        match i2c.set_slave_address(MCP23017_ADDRESS.into()) {
            Ok(_) => match i2c.write(&[]) {
                Ok(_) => {
                    println!("MCP23017 detected at 0x{:02X}", MCP23017_ADDRESS);
                    true
                }
                Err(err) => {
                    eprintln!(
                        "MCP23017 not detected at 0x{:02X}: {}",
                        MCP23017_ADDRESS, err
                    );
                    false
                }
            },
            Err(err) => {
                eprintln!(
                    "Could not select MCP23017 address 0x{:02X}: {}",
                    MCP23017_ADDRESS, err
                );
                false
            }
        }
    }
    fn open_i2c_with_retry() -> I2c {
        const RETRY_COUNT: u8 = 12;
        const RETRY_DELAY: Duration = Duration::from_secs(5);

        for attempt in 1..=RETRY_COUNT {
            match I2c::new() {
                Ok(i2c) => return i2c,
                Err(err) if attempt < RETRY_COUNT => {
                    eprintln!(
                        "I2C bus is not ready yet (attempt {}/{}): {}",
                        attempt, RETRY_COUNT, err
                    );
                    thread::sleep(RETRY_DELAY);
                }
                Err(err) => {
                    panic!("I2C bus did not become ready after {RETRY_COUNT} attempts: {err}")
                }
            }
        }

        unreachable!("startup retry loop always returns");
    }
    pub fn init(&mut self) {
        if !self.mcp_present {
            println!("Skipping MCP23017 init because the device is not present.");
            return;
        }
        let i2c_mcp = MutexDevice::new(&self.bus).reverse();
        let mut mcp: Mcp230xx<_, Mcp23017> = Mcp230xx::new(i2c_mcp, MCP23017_ADDRESS).unwrap();
        for i in 0..=15 {
            let pin = Mcp23017::try_from(i).unwrap();
            println!("{:?}", pin);
            if let Ok(_) = mcp.set_direction(pin, Direction::Output) {
                println!("Pin: {:?} Configured as output", pin);
            }
            let _ = mcp.set_gpio(self.all_pins[i], mcp230xx::Level::Low);
        }
    }
    pub fn read_pin(&mut self, pin: Mcp23017) -> Result<mcp230xx::Level> {
        if !self.mcp_present {
            bail!("MCP23017 is not present; cannot read {:?}", pin);
        }
        let i2c_mcp = MutexDevice::new(&self.bus).reverse();
        let mut mcp: Mcp230xx<_, Mcp23017> = Mcp230xx::new(i2c_mcp, MCP23017_ADDRESS).unwrap();
        Ok(mcp.gpio(pin)?)
    }
    pub fn set_pin(&mut self, pin: Mcp23017, val: mcp230xx::Level) -> Result<()> {
        if !self.mcp_present {
            bail!("MCP23017 is not present; cannot set {:?}", pin);
        }
        let i2c_mcp = MutexDevice::new(&self.bus).reverse();
        let mut mcp: Mcp230xx<_, Mcp23017> = Mcp230xx::new(i2c_mcp, MCP23017_ADDRESS).unwrap();
        mcp.set_gpio(pin, val)?;
        Ok(())
    }
    pub fn read_val(&self) -> Result<[f64; 5], Box<dyn Error>> {
        let i2c_ina = MutexDevice::new(&self.bus);
        let i2c_ina1 = MutexDevice::new(&self.bus);
        let i2c_ina2 = MutexDevice::new(&self.bus);
        let delay = StdDelay::default();
        let mut ina: INA228Sync<StdDelay, I2cDeviceSync<MutexDevice<'_, _>, u8>> =
            INA228Sync::new_i2c(delay, i2c_ina, Address::A0A1(Pin::Gnd, Pin::Gnd));
        let mut ina2: INA228Sync<StdDelay, I2cDeviceSync<MutexDevice<'_, I2c>, u8>> =
            INA228Sync::new_i2c(delay, i2c_ina1, Address::A0A1(Pin::Vcc, Pin::Gnd));
        let mut ina3: INA228Sync<StdDelay, I2cDeviceSync<MutexDevice<'_, I2c>, u8>> =
            INA228Sync::new_i2c(delay, i2c_ina2, Address::A0A1(Pin::Gnd, Pin::Vcc));
        ina.init(
            ElectricalResistance::new::<ohm>(0.015),
            ElectricCurrent::new::<ampere>(3.0),
        )
        .unwrap_or(());
        ina2.init(
            ElectricalResistance::new::<ohm>(0.25),
            ElectricCurrent::new::<ampere>(0.2),
        )
        .unwrap_or(());
        ina3.init(
            ElectricalResistance::new::<ohm>(0.25),
            ElectricCurrent::new::<ampere>(0.2),
        )
        .unwrap_or(());
        let val = [ina.measure()?, ina2.measure()?, ina3.measure()?];
        let mut output: [f64; 5] = [0.0; 5];
        val.iter().enumerate().for_each(|(i, x)| match i {
            0 => {
                output[0] = x.temperature.get::<degree_celsius>();
                output[1] = x.current.get::<ampere>();
                output[2] = x.bus_voltage.get::<volt>();
            }
            1 => {
                output[3] = x.current.get::<milliampere>();
            }
            2 => {
                output[4] = x.current.get::<milliampere>();
            }
            _ => {}
        });
        Ok(output)
    }
}
