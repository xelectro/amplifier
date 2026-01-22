pub mod encoder {
    use rppal::gpio::{Gpio, Level, Mode};
    use rppal::system::DeviceInfo;
    use serde::{Deserialize, Serialize};
    use std::sync::mpsc;
    use std::sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
    };
    use std::thread;
    use std::time::Duration;
    extern crate rppal;

    const GPIO_PIN_CLK: u8 = 24;
    const GPIO_PIN_DAT: u8 = 23;
    /*
       fn main() {
           let mut my_enc = Encoder::new();
           match  &mut my_enc.run() {
               Ok(_) => {}
               e => {
                   eprintln!("Error: {:?}", e);
               }
           }
       }
    */
    #[derive(Clone)]
    pub struct Encoder {
        pub pin_a: u8,
        pub pin_b: u8,
        pub count: Arc<AtomicI32>,
    }
    impl Encoder {
        pub fn new(pina: u8, pinb: u8) -> Self {
            Self {
                pin_a: pina,
                pin_b: pinb,
                count: Arc::new(AtomicI32::new(0)),
            }
        }
        pub fn run(&mut self) -> Result<(), Box<::rppal::gpio::Error>> {
            let device_info = DeviceInfo::new().unwrap();
            println!(
                "Model: {} (SoC: {})",
                device_info.model(),
                device_info.soc()
            );
            let master_count = Arc::clone(&self.count);

            //let (tx, rx) = mpsc::channel();

            thread::spawn(move || {
                let gpio = Gpio::new().unwrap();
                let pin1 = gpio.get(GPIO_PIN_CLK).unwrap().into_input_pullup();
                let pin2 = gpio.get(GPIO_PIN_DAT).unwrap().into_input_pullup();

                let mut last_clk_state = Level::High;

                loop {
                    let state = pin1.read();
                    match state {
                        Level::High => {
                            if last_clk_state == Level::Low {
                                if let Level::Low = pin2.read() {
                                    //tx.send(1).unwrap();
                                    master_count.fetch_add(1, Ordering::Relaxed);
                                } else {
                                    //tx.send(-1).unwrap();
                                    master_count.fetch_add(-1, Ordering::Relaxed);
                                }

                                last_clk_state = Level::High;
                            }
                        }

                        Level::Low => {
                            last_clk_state = state;
                        }
                    }
                    thread::sleep(Duration::from_micros(10));
                }
            });
            /*
                        let mut  count = 0;
                        for received in rx {
                            count += received;

                            println!("Got: {} for a count of {}", received, count);

                        }

            */
            Ok(())
        }
        pub fn enc(&self) -> i32 {
            self.count.load(Ordering::Relaxed)
        }
    }
}

pub mod stepper {
    use rppal::gpio::{Gpio, Level, Mode};
    use serde::{Deserialize, Serialize};
    use std::collections::HashMap;
    use std::sync::{
        Arc,
        atomic::{AtomicI32, Ordering},
    };
    use std::thread;
    use std::time::Duration;

    #[derive(Clone)]
    pub struct Stepper {
        pub name: String,
        pub pin_a: Option<u8>,
        pub pin_b: Option<u8>,
        pub ena: Option<u8>,
        pub ratio: u8,
        pub pos: Arc<AtomicI32>,
        pub mem: HashMap<String, Arc<AtomicI32>>,
        pub max: Arc<AtomicI32>,
        pub speed: Duration,
        pub operate: bool,
    }
    impl Stepper {
        pub fn new(name: &str) -> Self {
            Self {
                name: name.to_string(),
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
                operate: false,
            }
        }
        pub fn run(&self, val: u32) {
            let mut steps: u32 = 0;
            let mut dir: String = String::new();
            let pos: u32 = self.pos.load(Ordering::Relaxed) as u32;
            let gpio = Gpio::new().unwrap();
            let mut pulse_pin = gpio.get(self.pin_a.unwrap()).unwrap().into_output();
            let mut dir_pin = gpio.get(self.pin_b.unwrap()).unwrap().into_output();
            let mut count = 0;
            pulse_pin.set_low();
            if val > pos {
                dir = "CW".to_string();
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
                dir = "CCW".to_string();
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
    }
}
pub mod mcp {
    use std::collections::HashMap;
    use linux_embedded_hal::I2cdev;
    use mcp230xx::{Direction, Mcp230xx, Mcp23017, Level};

    pub struct Mcp {
        pub pins: HashMap<String, Mcp23017>,
        pub mcp: Mcp230xx<I2cdev, Mcp23017>,
        pub message: String,
        pub switch: HashMap<String, String>
    }
    impl Mcp {
        // default function that sets all pins as output.
        pub fn new() -> Self {
            let i2c= I2cdev::new("/dev/i2c-1").unwrap();
            let mut mcp: Mcp230xx<_, Mcp23017> = Mcp230xx::new(i2c, 0x20).unwrap();
            let all_pins = [
                Mcp23017::A0, Mcp23017::A1, Mcp23017::A2,
                Mcp23017::A3, Mcp23017::A4, Mcp23017::A5,
                Mcp23017::A6, Mcp23017::A7, Mcp23017::B0,
                Mcp23017::B1, Mcp23017::B2, Mcp23017::B3,
                Mcp23017::B4, Mcp23017::B5, Mcp23017::B6,
                Mcp23017::B7,
            ];
            for i in 0..=15 {
                let pin = Mcp23017::try_from(i).unwrap();
                println!("{:?}", pin);
                if let Ok(_) = mcp.set_direction(pin, Direction::Output){
                    println!("Pin: {:?} Configured as output", pin);
                }
                mcp.set_gpio(all_pins[i], Level::Low);
            }
            Self {
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
                mcp: mcp,
                message: String::from("MCP Intioalized ! ! !"),
                switch: HashMap::new(),
                }
                
        }
    }
}

