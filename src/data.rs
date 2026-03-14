use std::collections::HashMap;
use askama::Template;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, mpsc};
use tokio::sync::{broadcast::{self, Sender}, Mutex};
use crate::web::{Stepper, Mcp, Encoder};
use mcp230xx::Mcp23017;
use rppal::gpio::{Gpio, OutputPin};

#[derive(Template)]
#[template(path = "amplifier2.html")]
pub struct IndexTemplate<'a> {
    pub name: &'a str,
}
#[derive(Template)]
#[template(path = "config2.html")]
pub struct ConfigTemplate {
    pub enc: bool,
    pub enc_val: Vec<String>,
    pub tune: Vec<String>,
    pub ind: Vec<String>,
    pub load: Vec<String>,
    pub pins: Vec<u8>,
    pub files: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SseData {
    pub tune: u32,
    pub ind: u32,
    pub load: u32,
    pub max: HashMap<String, u32>,
    pub sw_pos: Option<Select>,
    pub band: Bands,
    pub ratio: HashMap<String, u8>,
    pub i2c_devices: Vec<u16>,
    pub plate_v: u32,
    pub plate_a: u32,
    pub screen_a: u32,
    pub grid_a: u32,
    pub pwr_btns: HashMap<String, [String; 2]>,
    pub temperature: f64,
    pub call_sign: String,
    pub time: String,
    pub status: String,
}
impl SseData {
    pub fn new() -> SseData {
        SseData {
            tune: 0,
            ind: 0,
            load: 0,
            max: HashMap::from([
                ("tune".to_string(), 100000),
                ("ind".to_string(), 100000),
                ("load".to_string(), 100000),
            ]),
            sw_pos: None,
            band: Bands::M11,
            ratio: HashMap::from([
                ("tune".to_string(), 1),
                ("ind".to_string(), 1),
                ("load".to_string(), 1),
            ]),
            i2c_devices: Vec::new(),
            plate_v: 0,
            plate_a: 0,
            screen_a: 0,
            grid_a: 0,
            pwr_btns: HashMap::from([
                ("Blwr".to_string(), ["OFF".to_string(), "OFF".to_string()]),
                ("Fil".to_string(), ["OFF".to_string(), "OFF".to_string()]),
                ("HV".to_string(), ["OFF".to_string(), "OFF".to_string()]),
                ("Oper".to_string(), ["OFF".to_string(), "OFF".to_string()]),
            ]),
            temperature: 0.0,
            time: String::new(),
            call_sign: String::from("-----"),
            status: "Hello ALL BAND AMP".to_string(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct StoredData {
    pub tune: HashMap<String, u32>,
    pub ind: HashMap<String, u32>,
    pub load: HashMap<String, u32>,
    pub enc: HashMap<String, u32>,
    pub mem: HashMap<String, HashMap<String, u32>>,
    pub band: Bands,
    pub call_sign: String,
}
impl StoredData {
    pub fn new() -> Self {
        Self {
            tune: HashMap::new(),
            ind: HashMap::new(),
            load: HashMap::new(),
            enc: HashMap::new(),
            mem: HashMap::new(),
            band: Bands::M10,
            call_sign: String::from("-----"),
        }
    }
}
#[derive(Clone)]
pub struct AppState {
    pub tune: Arc<Mutex<Stepper>>,
    pub ind: Arc<Mutex<Stepper>>,
    pub load: Arc<Mutex<Stepper>>,
    pub enc: Option<Encoder>,
    pub sw_pos: Option<Select>,
    pub band: Bands,
    pub gauges: Gauges,
    pub file: String,
    pub sleep: bool,
    pub enable_pin: Arc<Mutex<OutputPin>>,
    pub pwr_btns: PwrBtns,
    pub pwr_btns_state: HashMap<String, [String;2]>,
    pub temperature: f64,
    pub gpio_pins: Vec<u8>,
    pub call_sign: String,
    pub status: String,
    pub sender: Sender<String>,
    pub meter_sender: Option<mpsc::Sender<bool>>,
}
#[derive(Clone, Serialize, Deserialize)]
pub enum Select {
    Tune,
    Ind,
    Load,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub enum Bands {
    M10,
    M11,
    M20,
    M40,
    M80,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Gauges {
    pub plate_v: u32,
    pub plate_a: u32,
    pub screen_a: u32,
    pub grid_a: u32,
}
#[derive(Clone)]
pub struct PwrBtns {
    pub Blwr: [Mcp23017; 1],
    pub Fil: [Mcp23017; 2],
    pub HV: [Mcp23017; 2],
    pub Oper: [Mcp23017; 1],
    pub mcp: Mcp,
    pub bands: [Mcp23017; 5],
}
impl PwrBtns {
    pub fn new() -> Self {
        let mcp = Mcp::new();
        Self {
            Blwr: [*mcp.pins.get("A0").unwrap()],
            Fil: [*mcp.pins.get("A1").unwrap(), *mcp.pins.get("A2").unwrap()],
            HV: [*mcp.pins.get("A3").unwrap(), *mcp.pins.get("A4").unwrap()],
            Oper: [*mcp.pins.get("A5").unwrap()],
            bands: [*mcp.pins.get("B0").unwrap(),
                    *mcp.pins.get("B1").unwrap(),
                    *mcp.pins.get("B2").unwrap(),
                    *mcp.pins.get("B3").unwrap(),
                    *mcp.pins.get("B4").unwrap(),],
            mcp: {let mut output  = Mcp::new();
                output.init();
                output}

        }
    }
}

