
use askama::Template;
use axum::response::sse::KeepAlive;
use mcp230xx::Mcp23017;
use mcp230xx;
use std::env;
use rppal::gpio::Gpio;
use axum::response::{Html, IntoResponse, Redirect};
use axum::{
    Router,
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::sse::{Event, Sse},
    routing::{get, post},
};
use axum_extra::TypedHeader;
use async_stream::stream;
use futures_util::stream::Stream;
use serde_json;
use std::collections::HashMap;
use std::fs;
use std::io::Error;
use std::path;
use std::sync::atomic::Ordering;
use std::sync::{Arc, mpsc};
use std::time::Duration;
use tokio::sync::{broadcast, Mutex};
use tokio::time::interval;
use tower_http::{services::ServeDir};
use chrono;
use anyhow::Result;
pub mod web;
pub mod data;
use web::{Encoder, Stepper};
use data::{IndexTemplate, ConfigTemplate, SseData,
    AppState, StoredData, Select,
    PwrBtns, Bands, Gauges};
const ENABLE_PIN: u8 = 16;

#[tokio::main]
async fn main() -> Result<()>{
    let (tx, _rx) = broadcast::channel(1024);
    let app_state = Arc::new(Mutex::new(AppState {
        tune: Arc::new(Mutex::new(Stepper::new("tune"))),
        ind: Arc::new(Mutex::new(Stepper::new("ind"))),
        load: Arc::new(Mutex::new(Stepper::new("load"))),
        enc: None, //Some(Encoder::new(24, 23)),
        sw_pos: None,
        band: Bands::M10,
        gauges: Gauges {
            plate_v: 0,
            plate_a: 1,
            screen_a: 50,
            grid_a: 10,
        },
        file: String::from("amplifier.json"),
        sleep: false,
        enable_pin: {
            let gpio = Gpio::new().unwrap();
            let mut pin = gpio.get(ENABLE_PIN).unwrap().into_output();
            pin.set_high();
            Arc::new(Mutex::new(pin))
        },
        pwr_btns : PwrBtns::new(),
        pwr_btns_state: HashMap::from([
                ("Blwr".to_string(), ["OFF".to_string(), "OFF".to_string()]),
                ("Fil".to_string(), ["OFF".to_string(), "OFF".to_string()]),
                ("HV".to_string(), ["OFF".to_string(), "OFF".to_string()]),
                ("Oper".to_string(), ["OFF".to_string(), "OFF".to_string()]),
            ]),
        temperature: 0.0,
        gpio_pins: vec![17, 27, 22, 5, 6, 13, 19,
                        26,14, 15, 18, 23, 24, 25,
                        12, 20, 21],
        call_sign: String::new(),
        status: String::new(),
        sender: tx,
        meter_sender: None,
    }));

    tokio::spawn(aquire_data(app_state.clone()));
    tokio::spawn(aquire_i2c_data(app_state.clone()));
    let listener = tokio::net::TcpListener::bind("0.0.0.0:3000").await.unwrap();
    let app = Router::new()
        .route("/sse", get(sse_handler))
        .route("/config", get(config_get).post(config_post))
        .route("/voltage", get(voltage_get))
        .route(
            "/",
            get(|| async {
                let template = IndexTemplate { name: "Axum User" };
                Html(template.render().unwrap())
            }),
        )
        //.route("/", get(default))
        .nest_service("/static", ServeDir::new("static"))
        .route("/selector/{val}", post(selector))
        .route("/store/{band}", post(store))
        .route("/recall/{band}", post(recall))
        .route("/stop", post(stop))
        .route("/load",  post(load))
        .route("/pwr_btn", post(pwr_btn_handler))
        .with_state(app_state);
    let _ = axum::serve(listener, app).await;
    Ok(())
}

// receiver form data from config page.
async fn voltage_get(State(app_state): State<Arc<Mutex<AppState>>>) -> impl IntoResponse {
    Html::from(read_html_file(&path::Path::new("templates/voltage.html")).unwrap())
}
async fn config_post(
    State(state): State<Arc<Mutex<AppState>>>,
    form: Multipart,
) -> impl IntoResponse {
    let form_data = process_form(form).await;
    let mut state = state.lock().await;
    if let Some(_) = state.enc  {
        if form_data.contains_key("del_enc") {
            let pin_a = state.enc.clone().unwrap().pin_a;
            let pin_b = state.enc.clone().unwrap().pin_b;
            let _ = process_pins(&mut state.gpio_pins, pin_a, false);
            let _ = process_pins(&mut state.gpio_pins, pin_b, false);
            *state.enc.clone().unwrap().stop.lock().unwrap() = true;
            state.enc = None;
            state.status = "Encoder has benn deleted!".to_string();
            
        }
        else if form_data.contains_key("add_tune") {
            if let Some(_) = state.tune.lock().await.pin_a {
                println!("PinA already initialized for Tune");
            } else {
                handle_stepper(&mut state, form_data.clone(),  "Tune", true,|state| state.tune.clone()).await;
                
            }
        }
        else if form_data.contains_key("del_tune") {
            handle_stepper(&mut state, form_data.clone(),  "Tune", false, |state| state.tune.clone()).await; 
        }
        else if form_data.contains_key("add_ind") {
            if let Some(_) = state.ind.lock().await.pin_a {
                println!("PinA already initialized for Ind");
            } else {
                handle_stepper(&mut state, form_data.clone(),  "Ind", true,|state| state.ind.clone()).await; 
            }
        }
        else if form_data.contains_key("del_ind") {
            handle_stepper(&mut state, form_data.clone(),  "Ind", false ,|state| state.ind.clone()).await; 
        }
        else if form_data.contains_key("add_load") {
            if let Some(_) = state.load.lock().await.pin_a {
                println!("PinA already initialized for Load");
            } else {
                handle_stepper(&mut state, form_data.clone(),  "Load", true,|state| state.load.clone()).await; 
                
            }
        }
        else if form_data.contains_key("del_load") {
            handle_stepper(&mut state, form_data.clone(),  "Load", false ,|state| state.load.clone()).await; 
            } 
        else if form_data.contains_key("start") {
            state.sw_pos = None;
            match form_data.get("start").unwrap().as_str() {
                "tune" => {
                    let state_tune = state.tune.lock().await;
                    state_tune.pos.store(0, Ordering::Relaxed);
                }
                "ind" => {
                    let state_ind = state.ind.lock().await;
                    state_ind.pos.store(0, Ordering::Relaxed);
                }
                "load" => {
                    let state_load = state.load.lock().await;
                    state_load.pos.store(0, Ordering::Relaxed);
                }
                _ => println!("Invalid argument")
            }
        }  
        else if form_data.contains_key("max") {
            match form_data.get("max").unwrap().as_str() {
                "tune" => {
                    let state_tune = state.tune.lock().await;
                    state_tune.max.store(state_tune.pos.load(Ordering::Relaxed), Ordering::Relaxed);
                }
                "ind" => {
                    let state_ind = state.ind.lock().await;
                    state_ind.max.store(state_ind.pos.load(Ordering::Relaxed), Ordering::Relaxed);
                }
                "load" => {
                    let state_load = state.load.lock().await;
                    state_load.max.store(state_load.pos.load(Ordering::Relaxed), Ordering::Relaxed);
                }
                _ => println!("Invalid argument") 
            }
            println!("Max was set");
        }  else if form_data.contains_key("reset") {
            match form_data.get("reset").unwrap().as_str() {
                "tune" => {
                    let state_tune = state.tune.lock().await;
                    state_tune.max.store(100000, Ordering::Relaxed);
                }
                "ind" => {
                    let state_ind = state.ind.lock().await;
                    state_ind.max.store(100000, Ordering::Relaxed);
                }
                "load" => {
                    let state_load = state.load.lock().await;
                    state_load.max.store(100000, Ordering::Relaxed);
                }
                _ => println!("Invalid argument")
            }
        }
    } else {
        if form_data.contains_key("PinA") && form_data.contains_key("PinB") {
                if form_data.get("PinA").unwrap() != "" && form_data.get("PinB").unwrap() != "" {
                let pin_a = form_data.get("PinA").unwrap().parse().unwrap();
                let pin_b = form_data.get("PinB").unwrap().parse().unwrap();
                state.enc = Some(Encoder::new(
                    pin_a,
                    pin_b,
                ));
                let _ = state.enc.clone().unwrap().run();
                let _ = process_pins(&mut state.gpio_pins, form_data.get("PinA").unwrap().parse().unwrap(), true);
                let _ = process_pins(&mut state.gpio_pins, form_data.get("PinB").unwrap().parse().unwrap(), true);
                println!("Encoder Added");
                state.status = format!(
                    "Encoder Added on pins: {:?}, {:?}",
                    form_data.get("PinA").unwrap(),
                    form_data.get("PinB").unwrap(),
                );
            }
        }
    }
    if form_data.clone().contains_key("call_sign") {
        state.call_sign = form_data.get("call_sign").unwrap().clone();
        println!("Callsign added: {}", state.call_sign);
    }
    Redirect::to("/config")
}

fn process_pins(pin_list: &mut Vec<u8>, val: u8, remove: bool) -> Result<()> {
    if remove {
        if let Some(out) = pin_list.iter().position(|&x| x == val) {
            pin_list.remove(out);
            
        }
        return Ok(())
    } else {
        pin_list.push(val);
        return Ok(())
    }
  
}
// Route handler for GET request for config page.
async fn config_get(State(state): State<Arc<Mutex<AppState>>>) -> Html<String> {
    println!("Config get was called.");
    let state = state.lock().await;
    let tune = state.tune.lock().await;
    let ind = state.ind.lock().await;
    let load = state.load.lock().await;
    let template = ConfigTemplate {
        enc: if let Some(_) = state.enc { true } else { false },
        enc_val: if let Some(_) = state.enc {
            vec![
                state.enc.clone().unwrap().pin_a.to_string(),
                state.enc.clone().unwrap().pin_b.to_string(),
            ]
        } else {
            vec!["None".to_string(), "None".to_string()]
        },
        tune: if let Some(_) = tune.pin_a {
            vec![
                tune.pin_a.unwrap().to_string(),
                tune.pin_b.unwrap().to_string(),
                tune.ratio.to_string(),
            ]
        } else {
            vec!["None".to_string(), "None".to_string(), 1.to_string()]
        },
        ind: if let Some(_) = ind.pin_a {
            vec![
                ind.pin_a.unwrap().to_string(),
                ind.pin_b.unwrap().to_string(),
                ind.ratio.to_string(),
            ]
        } else {
            vec!["None".to_string(), "None".to_string(), 1.to_string()]
        },
        load: if let Some(_) = load.pin_a {
            vec![
                load.pin_a.unwrap().to_string(),
                load.pin_b.unwrap().to_string(),
                load.ratio.to_string(),
            ]
        } else {
            vec!["None".to_string(), "None".to_string(), 1.to_string()]
        },
        files: {
            let home_path = env::current_dir().unwrap().join("static");
            let mut output: Vec<String> = Vec::new();
            let files =
                fs::read_dir(home_path).unwrap();
            files.for_each(|f| {
                let temp_file = f.unwrap().file_name().to_string_lossy().to_string();
                if temp_file.ends_with("json") {
                    output.push(temp_file);
                }
            }); 
            output
        },
        pins: state.gpio_pins.clone(),
    };
    Html(template.render().unwrap().to_string())
}
// Processes initial SSE Request (Route Handler).
async fn sse_handler(
    TypedHeader(_): TypedHeader<headers::UserAgent>,
    State(app_state): State<Arc<Mutex<AppState>>>,
) -> Sse<impl Stream<Item = Result<Event>>> {
    let state_lck = app_state.lock().await;
    let mut rx = state_lck.sender.subscribe();
    Sse::new(stream! {
        while let Ok(msg) = rx.recv().await {
            yield Ok(Event::default().data::<String>(msg));
        }
    }).keep_alive(KeepAlive::default())

}

//Selects a stepper to be tuned.
async fn selector(
    Path(val): Path<String>, State(app_state): State<Arc<Mutex<AppState>>>,
    mut form_data: Multipart,
) -> impl IntoResponse {
    println!("Form handler");
    println!("{}", val);
    app_state.lock().await.enable_pin.lock().await.set_low();
    let state_lck = app_state.lock().await.clone();
    let tune = state_lck.tune.lock().await.clone();
    let ind = state_lck.ind.lock().await.clone();
    let load = state_lck.load.lock().await.clone();
    if  *tune.operate.lock().unwrap() == false && *ind.operate.lock().unwrap() == false && *load.operate.lock().unwrap() == false {
        while let Some(val) = form_data.next_field().await.unwrap() {
            println!("Name: {}", val.name().unwrap().to_string());
            match val.name().unwrap() {
                "tune" => {
                    let mut state = app_state.lock().await;
                    if let Ok(_) = selector_handler(&mut state, |x| x.tune.clone()).await {
                        state.status = "Tune is selected".to_string();
                        state.sw_pos = Some(Select::Tune);
                    }
                }
                "ind" => {
                    let mut state = app_state.lock().await;
                    if let Ok(_) = selector_handler(&mut state, |x| x.ind.clone()).await {
                        state.status = "Ind is selected".to_string();
                        state.sw_pos = Some(Select::Ind);
                        
                        
                    }
                }
                "load" => {
                    let mut state = app_state.lock().await;
                    if let Ok(_) = selector_handler(&mut state, |x| x.load.clone()).await {
                        state.status = "Load is selected".to_string();
                        state.sw_pos = Some(Select::Load);
                    }
                    
                }
                _ => {
                    println!("Invalid form Entry");
                }
            }
        }
    } else {
        app_state.lock().await.status = format!("Cannot select a tuner while tune is in progress ! ! !");
    }
    StatusCode::OK
}

async fn selector_handler<F>(state: &mut AppState,  callback: F) -> Result<()>
where F:
        Fn(&mut AppState) -> Arc<Mutex<Stepper>> {
    let _ = state.meter_sender.clone().unwrap().send(false);
    let stepper = callback(state);
    if let Some(enc) = state.clone().enc {
        enc.count.store(stepper.clone().lock().await.pos.load(Ordering::Relaxed), Ordering::Relaxed);
        return Ok(())
    } else {
        state.status = format!("No Encoder present! ! !");
        Err(Error::new(std::io::ErrorKind::Other, "No Encoder Forund").into())
        
    }

}
//Recalls bands from memory.
async fn recall(Path(path): Path<String>, State(state): State<Arc<Mutex<AppState>>>) {
    println!("{}", path);
    let state_lck = state.lock().await.clone();
        if *state_lck.tune.lock().await.operate.lock().unwrap() == false && *state_lck.ind.lock().await.operate.lock().unwrap() == false && *state_lck.load.lock().await.operate.lock().unwrap() == false  {
            state.lock().await.sleep = true;
            match path.as_str() {
                "M10" => {
                    if let Ok(_) = recall_handler(state.clone(), "10M".to_string(), Bands::M10).await {
                        
                    } else  {
                        state.lock().await.status = format!("No Encoder Present");
                    }
                }
                "M11" => {
                    if let Ok(_) = recall_handler(state.clone(), "11M".to_string(), Bands::M11).await {
        
                    } else  {
                        state.lock().await.status = format!("No Encoder Present");
                    }
                }
                "M20" => {
                    if let Ok(_) = recall_handler(state.clone(), "20M".to_string(), Bands::M20).await {
            
                    } else  {
                        state.lock().await.status = format!("No Encoder Present");
                    }
                }
                "M40" => {
                    if let Ok(_) = recall_handler(state.clone(), "40M".to_string(), Bands::M40).await {
        
                    } else  {
                        state.lock().await.status = format!("No Encoder Present");
                    }
                }
                "M80" => {
                    if let Ok(_) = recall_handler(state.clone(), "80M".to_string(), Bands::M80).await {
                    } else  {
                        state.lock().await.status = format!("No Encoder Present");
                    }
                }
                _ => {
                    println!("Invalid band selected!!")
                }
            }
        } else {
        state.lock().await.status = format!("Attempted to recall while motors still in motion!!");
    }
}
// Saves data to JSON file from AppState.
async fn store(Path(path): Path<String>, State(state): State<Arc<Mutex<AppState>>>) {
    println!("Store Called");
    println!("{}", path);
    match path.as_str() {
        "M10" => {
            store_handler(state, "10M".to_string()).await;
        }
        "M11" => {
            store_handler(state, "11M".to_string()).await;
        }
        "M20" => {
            store_handler(state, "20M".to_string()).await;
        }
        "M40" => {
            store_handler(state, "40M".to_string()).await;
        }
        "M80" => {
            store_handler(state, "80M".to_string()).await;
        }
        _ => {
            println!("Invalid band selected!!")
        }
    }
}

async fn stop(State(state): State<Arc<Mutex<AppState>>>) {
    println!("Save stop request received");
    sleep_save(state).await;

}
// Loads data from config file and initialized AppState.
async fn load(State(state): State<Arc<Mutex<AppState>>>, form: Multipart) ->
    impl IntoResponse {
    println!("Config PostForm Handler");
    let form_data = process_form(form).await;
    if form_data.contains_key("files") && form_data.contains_key("load") {
        let file_name = form_data.get("files").unwrap();
        println!("Filename: {}", file_name);
        let full_path = env::current_dir().unwrap().join("static").join(file_name);
        if let Ok(file_data) = fs::read_to_string(full_path) {
            let output: StoredData = serde_json::from_str(&file_data).unwrap();
            println!("{:?}", output);
            let mut state_lck = state.lock().await;
            state_lck.file = file_name.to_string();
            let mut my_stepper_arr = [
                state_lck.tune.clone(),
                state_lck.ind.clone(),
                state_lck.load.clone(),
                ];
            let bands = ["10M", "11M", "20M", "40M", "80M"];
            let  my_output_arr = [&output.tune, &output.ind, &output.load];
            for (i, stepper) in my_stepper_arr.iter_mut().enumerate() {
                let name = &stepper.lock().await.clone().name;
                handle_stepper(&mut state_lck, form_data.clone(), name, false, |_x| stepper.clone()).await;
                println!("TEST AREA");
                interval(Duration::from_millis(10)).tick().await;
                println!("Adding PinA: {:?}", stepper.lock().await.pin_a);
                println!("Adding PinB: {:?}", stepper.lock().await.pin_b);
                stepper.lock().await.pin_a = if my_output_arr[i].contains_key("PinA") {Some(*my_output_arr[i].get("PinA").unwrap() as u8)} else {None};
                stepper.lock().await.pin_b = if my_output_arr[i].contains_key("PinB") {Some(*my_output_arr[i].get("PinB").unwrap() as u8)} else {None};
                stepper.lock().await.ena = if my_output_arr[i].contains_key("ena") {Some(*my_output_arr[i].get("ena").unwrap() as u8)} else {None};
                stepper.lock().await.max.store(*my_output_arr[i].get("max").unwrap() as i32, Ordering::Relaxed);
                stepper.lock().await.pos.store(*my_output_arr[i].get("pos").unwrap() as i32, Ordering::Relaxed);
                stepper.lock().await.ratio = *my_output_arr[i].get("ratio").unwrap() as u8;
                let mut stepper_lck = stepper.lock().await;
                if stepper_lck.name == "ind" {
                    println!("Inductor set to lower speed");
                    stepper_lck.speed = Duration::from_micros(400);
                }
                if let Some(_) = stepper_lck.pin_a {
                    stepper_lck.run_2();
                }
                drop(stepper_lck);
                for band in bands {
                    let mut stepper_lck = stepper.lock().await;
                    println!("Stepper name: {}", stepper_lck.name);
                    let value = *output.mem.get(&stepper_lck.name).unwrap().get(&band.to_string()).unwrap_or(&0) as i32;
                    stepper_lck.mem.entry(band.to_string()).and_modify(|v| v.store(value, Ordering::Relaxed));
                } 
            }  
            state_lck.enc = if output.enc.contains_key("PinA") && output.enc.contains_key("PinB") {
                if let Some(enc) = &state_lck.enc {
                    let pin_a = enc.pin_a;
                    let pin_b = enc.pin_b;
                    let _ = process_pins(&mut state_lck.clone().gpio_pins, pin_a, false);
                    let _ = process_pins(&mut state_lck.clone().gpio_pins, pin_b, false);
                    *enc.stop.lock().unwrap() = true;
                    interval(Duration::from_millis(50)).tick().await;
                    state_lck.enc = None;
                    println!("Deconfiguring Encoder to load new config");
                }
                Some(Encoder::new( 
                    *output.enc.get("PinA").unwrap() as u8,
                    *output.enc.get("PinB").unwrap() as u8,
                ))
            } else {
                None
            };
            if let Some(mut enc) = state_lck.enc.clone() {
                let _ = enc.run();
                println!("Ecoder Run activated from File load Fn");
            }
            state_lck.band = output.band;
            state_lck.call_sign = output.call_sign;
            state_lck.status = format!("Sucessfully loaded: {} as a profile", file_name);
        }
        
    } else if form_data.contains_key("file_name") {
            let mut file_name = form_data.get("file_name").unwrap().clone().to_string();
            file_name.push_str(".json");
            state.lock().await.file = file_name.clone();
            state.lock().await.status = format!("Saved data to: {}", file_name);
            println!("{}", file_name);
            println!("New file saved");
            sleep_save(state).await;
        }
    return Redirect::to("/config");
}

//power button handler.
async fn pwr_btn_handler(State(state): State<Arc<Mutex<AppState>>>, form: Multipart) {
    let form_data = process_form(form).await;
    if form_data.contains_key("ID") {
        let sw = form_data.get("ID").unwrap();
        println!("Switch: {}", sw);
        let action = form_data.get("value").unwrap();
        println!("Action: {}", action);
        match sw.as_str() {
            "Blwr" => {
                let mut state_lck = state.lock().await;
                let pin = state_lck.pwr_btns.Blwr[0];
                let _ = state_lck.pwr_btns.mcp.set_pin(pin, if action == "ON" {mcp230xx::Level::High} else {mcp230xx::Level::Low}).unwrap_or(());
                state_lck.status = format!("{}", if action == "ON" {"Blower ON"} else {"Blower OFF"});

            },
            "Fil" => {
                let status = match step_start(&mut state.lock().await.clone(), form_data,"Filament".to_string(), |x| x.pwr_btns.Fil) {
                    Ok(n) => n,
                    Err(e) => {
                        println!("Error occured in Fillament Step start: {}", e);
                        format!("Error is Fil Step start")
                    },
                };
                state.lock().await.status = status;
            },
            "HV" => {
                let status = match step_start(&mut state.lock().await.clone(), form_data,"HV".to_string(), |x| x.pwr_btns.HV) {
                    Ok(n) => n,
                    Err(e) => {
                        println!("Error occured in HV Step Start: {}", e);
                        format!("Error is HV Step start")
                    },
                };
                state.lock().await.status = status;
            },
            "Oper" => {
                let mut state_lck = state.lock().await;
                let pin = state_lck.pwr_btns.Oper[0];
                let _ = state_lck.pwr_btns.mcp.set_pin(pin, if action == "ON" {mcp230xx::Level::High} else {mcp230xx::Level::Low});
                state_lck.status = format!("{}", if action == "ON" {"Operate"} else {"Standby"});

            },

            _ => println!("Invalid selection of swithes")
        }
    }
}

//step start helper function
fn step_start<F>(state_lck: &mut AppState, form_data: HashMap<String, String>, name: String, callback: F) -> Result<String>
where
    F: Fn(&mut AppState) -> [Mcp23017;2],
    {
        let action = form_data.get("value").unwrap();
        let my_btns = callback(state_lck);
        let pin1 = my_btns[0];
        let pin2 = my_btns[1];
        let pin1_status = state_lck.pwr_btns.mcp.read_pin(pin1)?;
        let _ = state_lck.pwr_btns.mcp.set_pin(pin1, if action == "ON" {mcp230xx::Level::High} else {mcp230xx::Level::Low});  
        if form_data.contains_key("delay") {
            let delay = form_data.get("delay").unwrap();
            let _ = state_lck.pwr_btns.mcp.set_pin(pin2, if delay == "ON"  && pin1_status == mcp230xx::Level::High {mcp230xx::Level::High} else {mcp230xx::Level::Low});
            state_lck.status = format!("{}", if action == "ON" && delay == "OFF" {
                println!("Stepstart: {}", name);
                format!("{} Step Start !!!",  name)
            } else if pin1_status == mcp230xx::Level::High && delay == "ON" {
                println!("ON: {}", name);
                format!("{}  ON ! ! !", name)
            } else {
                println!("OFF: {}", name);
                format!("{} Shutting Down...", name)
            });
            println!("STATUS: {}", state_lck.status);
        } 
        Ok(state_lck.status.clone())
    }
    
// Aquires data from peripheral devices and feeds SSE via a broadcast channel.
async fn aquire_data(state: Arc<Mutex<AppState>>) {
    let mut interval = interval(Duration::from_millis(10));
    println!("Aquire data");
    let mut count = 0;
    loop {
        interval.tick().await;
        let date_time = chrono::offset::Local::now().format("%m-%d-%Y, %H:%M:%S").to_string();
        let val = state.lock().await.clone();
        let call_sign = state.lock().await.call_sign.clone();
        let tune = val.tune.lock().await.clone();
        let ind = val.ind.lock().await.clone();
        let load = val.load.lock().await.clone();
        if *tune.operate.lock().unwrap() == false && *ind.operate.lock().unwrap() == false && *load.operate.lock().unwrap() == false && val.sleep == true {
            count += 1;
            if count >= 10 {
                sleep_save(state.clone()).await;
                count = 0;
            }
        } else {
            count = 0;
        }
        if let Some(_) = val.enc {
            let clone = val.enc.clone().unwrap().enc();
            if clone >= 0 {
                match val.sw_pos {
                    Some(Select::Tune) => {
                        if  clone < tune.max.load(Ordering::Relaxed)-1 && clone > 0 {
                            if let Some(_) = tune.pin_a {
                                if let Some(ch) = tune.channel.clone() {
                                    let _ = ch.send((clone as u32, false, true));
                                }
                            } else {
                                tune.pos.store(clone, Ordering::Relaxed);
                            }
                        }
                    }
                    Some(Select::Ind) => {
                        if  clone < ind.max.load(Ordering::Relaxed)-1 && clone > 0 {
                            if let Some(_) = ind.pin_a {
                                if let Some(ch) = ind.channel.clone() {
                                    let _ = ch.send((clone as u32, false, true));
                                }
                            } else {
                                ind.pos.store(clone, Ordering::Relaxed);
                            }
                        }
                    }
                    Some(Select::Load) => {
                        if  clone < load.max.load(Ordering::Relaxed)-1 && clone > 0 {
                            if let Some(_) = load.pin_a {
                                if let Some(ch) = load.channel.clone() {
                                    let _ = ch.send((clone as u32, false, true));
                                }
                            } else {
                                load.pos.store(clone, Ordering::Relaxed);
                            }
                        }
                    }
                    None => {}
                }
            } else {
                val.enc.clone().unwrap().count.store(0, Ordering::Relaxed);
            }
        }
        let mut sse_output = SseData::new();
        sse_output.time = date_time;
        sse_output.call_sign = call_sign;
        sse_output.tune = tune.pos.load(Ordering::Relaxed) as u32;
        sse_output.ind = ind.pos.load(Ordering::Relaxed) as u32;
        sse_output.load = load.pos.load(Ordering::Relaxed) as u32;
        sse_output.sw_pos = val.sw_pos.clone();
        sse_output.band = val.band.clone();
        sse_output.max.entry("tune".to_string()).insert_entry(tune.max.load(Ordering::Relaxed) as u32);
        sse_output.max.entry("ind".to_string()).insert_entry(ind.max.load(Ordering::Relaxed) as u32);
        sse_output.max.entry("load".to_string()).insert_entry(load.max.load(Ordering::Relaxed) as u32);
        let temp_bands = HashMap::from([
            ("tune".to_string(), tune.ratio),
            ("ind".to_string(), ind.ratio),
            ("load".to_string(), load.ratio),
        ]);
        for (key, val) in temp_bands {
            sse_output.ratio.entry(key).insert_entry(val);
        }
        sse_output.pwr_btns = val.pwr_btns_state;
        sse_output.i2c_devices = val.pwr_btns.mcp.device_list.clone();
        sse_output.plate_v = val.gauges.plate_v;
        sse_output.plate_a = val.gauges.plate_a;
        sse_output.screen_a = val.gauges.screen_a;
        sse_output.grid_a = val.gauges.grid_a;
        sse_output.temperature = val.temperature;
        sse_output.status = val.status.clone();
        let _ = val.sender.send(serde_json::to_string(&sse_output).unwrap());    
    }
}

//aquires I2C data and loads it to the AppState global Mutex.
async fn aquire_i2c_data(state: Arc<Mutex<AppState>>) {
    let mut interval = interval(Duration::from_millis(100));
    let mut temp_data: HashMap<String, [String;2]> = HashMap::new();
    let (tx, rx) = mpsc::channel();
    state.lock().await.meter_sender = Some(tx);
    let mut run = true;
    loop {
        interval.tick().await;
        let mut val = state.lock().await.pwr_btns.clone();
        let btn_arr = [val.Blwr[0], val.Fil[0], val.Fil[1], val.HV[0], val.HV[1]];
        btn_arr.iter().enumerate().for_each(|btn|{
            if let Ok(val) = val.mcp.read_pin(*btn.1) {
                match btn.0 {
                    0 => {
                        temp_data.insert("Blwr".to_string(), [
                        if val == mcp230xx::Level::High {"ON".to_string()} else {"OFF".to_string()},
                        "OFF".to_string()]);
                    },
                    1 | 2 => {
                        temp_data.insert("Fil".to_string(), [
                        if val == mcp230xx::Level::High {"ON".to_string()} else {"OFF".to_string()},
                        if val == mcp230xx::Level::High {"ON".to_string()} else {"OFF".to_string()}]);
                    }
                    3 | 4 => {
                        temp_data.insert("HV".to_string(), [
                        if val == mcp230xx::Level::High {"ON".to_string()} else {"OFF".to_string()},
                        if val == mcp230xx::Level::High {"ON".to_string()} else {"OFF".to_string()}]);
                    
                    }
                    _ => println!("Match statement error with MCP Pins")

                }
                    
            } 
        });
        if let Ok(val) = rx.try_recv() {
            run = val;
        }
        let mut temp = 0.0;
        let mut plate_a = 0_u32;
        let mut plate_v = 0_u32;
        let mut screen_a = 0_u32;
        let mut grid_a = 0_u32;
        if run {
            if let Ok(t) =  val.mcp.read_val() {
                plate_v = t[2].abs() as u32;
                plate_a = t[1].abs() as u32;
                temp = t[0];
                screen_a = t[3].abs() as u32;
                grid_a = t[4].abs() as u32;
            } 
        } 
        let mut state_lck = state.lock().await;
        state_lck.pwr_btns_state = temp_data.clone();
        state_lck.temperature = temp;
        state_lck.gauges.plate_a = plate_a;
        state_lck.gauges.plate_v = plate_v as u32 * 100;
        state_lck.gauges.screen_a = screen_a;
        state_lck.gauges.grid_a = grid_a;
    }
        
}

//assistant function to create and initialize stepper motors
async fn handle_stepper<F> (state: &mut AppState, form_data: HashMap<String, String>, name: &str, add: bool, process: F)
where
    F: Fn(&mut AppState) -> Arc<Mutex<Stepper>>,
    
 {
    let stepper = process(state);
    println!("Before LOCK");
    let mut state_stepper = stepper.lock().await;
    println!("AFTER LOCK");
    if add {
        state.sw_pos = None;
        if form_data.get("PinA").unwrap() != "" && form_data.get("PinB").unwrap() != "" {
            println!("Adding Stepper");
            let pin_a: u8 = form_data.get("PinA").unwrap().parse().unwrap();
            let pin_b: u8 = form_data.get("PinB").unwrap().parse().unwrap();
            let ratio: u8 = form_data.get("ratio").unwrap().parse().unwrap_or(1);
            state_stepper.name = name.to_string().to_lowercase();
            state_stepper.pin_a = Some(pin_a);
            state_stepper.pin_b = Some(pin_b);
            state_stepper.ratio = ratio;
            let _ = process_pins(&mut state.gpio_pins, pin_a, true);
            let _ = process_pins(&mut state.gpio_pins, pin_b, true);
            if name == "Ind" {
                state_stepper.speed = Duration::from_micros(400);
            }
            state_stepper.run_2();
        } else {
            println!("No pins Selected");
        }
    } else {
        println!("Resetting {} to default settings", name
    );
        if let Some(_) = state_stepper.pin_a {
            println!("Deleting {}", state_stepper.name);
            let pin_a = state_stepper.pin_a.unwrap();
            let pin_b = state_stepper.pin_b.unwrap();
            let _ = process_pins(&mut state.gpio_pins, pin_a, false);
            let _ = process_pins(&mut state.gpio_pins, pin_b, false);
            let _ = state_stepper.channel.clone().unwrap().send((state_stepper.pos.load(Ordering::Relaxed) as u32, true, false));
            state_stepper.pin_a = None;
            state_stepper.pin_b = None;
            state_stepper.ratio = 1;
        }
    }
    let pina = state_stepper.pin_a.unwrap_or(0);
    let pinb = state_stepper.pin_b.unwrap_or(0);
    let ratio = state_stepper.ratio;
    let name: String = state_stepper.name.clone().to_lowercase();
    drop(state_stepper);
    state.status = {
        if add {
            format!("{} Added on Pins: {}, {}, ratio of {}",name, pina, pinb, ratio)
        } else {
            format!("{} Deleted...", name)
        }
    }
        
 }
// Assistand function for recall route.
async fn recall_handler (state: Arc<Mutex<AppState>>, band: String, band_enum: Bands) -> Result<()> {
    let mut state_lck = state.lock().await;
    if let Some(_) = state_lck.enc {
        let _ = state_lck.meter_sender.clone().unwrap().send(false);
        state_lck.pwr_btns.clone().bands.iter().for_each(|pin|{
            let _ = state_lck.pwr_btns.clone().mcp.set_pin(*pin, mcp230xx::Level::Low);
        });
        match band_enum {
            Bands::M10 => {let _ = state_lck.pwr_btns.clone().mcp.set_pin(state_lck.pwr_btns.clone().bands[0], mcp230xx::Level::High);},
            Bands::M11 => {let _ = state_lck.pwr_btns.clone().mcp.set_pin(state_lck.pwr_btns.clone().bands[1], mcp230xx::Level::High);},
            Bands::M20 => {let _ = state_lck.pwr_btns.clone().mcp.set_pin(state_lck.pwr_btns.clone().bands[2], mcp230xx::Level::High);},
            Bands::M40 => {let _ = state_lck.pwr_btns.clone().mcp.set_pin(state_lck.pwr_btns.clone().bands[3], mcp230xx::Level::High);},
            Bands::M80 => {let _ = state_lck.pwr_btns.clone().mcp.set_pin(state_lck.pwr_btns.clone().bands[4], mcp230xx::Level::High);},
        }
        state_lck.band = band_enum;
        state_lck.sw_pos = None;
        state_lck.sleep = true;
        state_lck.enable_pin.lock().await.set_low();
        let my_locks = [
            state_lck.tune.clone(),
            state_lck.ind.clone(),
            state_lck.load.clone(),
        ];
        if state_lck.enable_pin.lock().await.is_set_low() {
            drop(state_lck);
            for x in my_locks {
                let value = band.clone();
                tokio::spawn(async move {
                    let temp_lck = x.lock().await.clone();
                    if let Some(_) = temp_lck.pin_a { 
                        let _ = temp_lck.channel.unwrap().send((temp_lck.mem.get(&value).unwrap().load(Ordering::Relaxed) as u32, false, false));
                    } else {
                        temp_lck.pos.store(temp_lck.mem.get(&value).unwrap().load(Ordering::Relaxed), Ordering::Relaxed);
                    }
                    println!("Run thread ended");

                });
                
            }
            let mut state_lck = state.lock().await;
            state_lck.status = format!("Recalled {} Band ! ! !", band);
        } else {
            state_lck.status = format!("Error with enable pin!");
        }
    return Ok(())
    } else {
        Err(Error::new(std::io::ErrorKind::Other, "No Encoder Present").into())
    }
}
async fn store_handler(state: Arc<Mutex<AppState>>, band: String) {
    let mut state_lck = state.lock().await;
    let my_locks = [
        state_lck.tune.clone(),
        state_lck.ind.clone(),
        state_lck.load.clone(),
    ];
    for lock in my_locks {
        let value = band.clone();
        let mut stepper = lock.lock().await;
        let pos = stepper.pos.load(Ordering::Relaxed);
        stepper.mem.entry(value).and_modify(|v| v.store(pos,Ordering::Relaxed));
    }
    state_lck.status = format!("Stored {} Band", band);

}
//funtion that stores all data when either save is presssed or after recall has been completed.
async fn sleep_save(state: Arc<Mutex<AppState>>) {
    let mut state_lck = state.lock().await;
    state_lck.sleep = false;
    println!("Sleep is: {}", state_lck.sleep);
    state_lck.enable_pin.lock().await.set_high();
    println!("Sleep_Save Ran");
    state_lck.sw_pos = None;
    let file_path = path::Path::new(&state_lck.file);
    let dir = env::current_dir().unwrap();
    let full_path = dir.join("static").join(file_path);
    if !fs::exists(&full_path).unwrap() {
        let _ = fs::File::create(&full_path);
    }
    let mut saved_state = StoredData::new();
    saved_state.enc.entry("PinA".to_string()).insert_entry(state_lck.clone().enc.unwrap().pin_a as u32);
    saved_state.enc.entry("PinB".to_string()).insert_entry(state_lck.clone().enc.unwrap().pin_b as u32);
    saved_state.mem.entry("tune".to_string()).insert_entry(store_data_creator(&mut state_lck.clone(), &mut saved_state.tune, |x| x.tune.clone()).await);
    saved_state.mem.entry("ind".to_string()).insert_entry(store_data_creator(&mut state_lck.clone(), &mut saved_state.ind, |x| x.ind.clone()).await);
    saved_state.mem.entry("load".to_string()).insert_entry(store_data_creator(&mut state_lck.clone(), &mut saved_state.load, |x| x.load.clone()).await);
    saved_state.band = state_lck.band.clone();
    saved_state.call_sign = state_lck.call_sign.clone();
    println!("Attempting to save data");
    if let Ok(output_data) = serde_json::to_string_pretty(&saved_state) {
        println!("Saving file to {}", full_path.to_string_lossy().to_string());
        if let Ok(_) = fs::write(full_path, output_data) {
            state_lck.status = format!("All data successfully saved !");
            let _ = state_lck.meter_sender.clone().unwrap().send(true);
        }
    }
    
}
//Assistant function to store route
async fn store_data_creator<F>(state_lck: &mut AppState, data: &mut HashMap<String,u32>, callback: F) -> HashMap<String, u32>
where
    F: Fn (&mut AppState) -> Arc<Mutex<Stepper>>,
    {
    let stepper = callback(state_lck);
    if let Some(pin_a) = stepper.lock().await.pin_a {
        data.entry("PinA".to_string()).insert_entry(pin_a as u32);
        
    }
    if let Some(pin_b) = stepper.lock().await.pin_b {
        data.entry("PinB".to_string()).insert_entry(pin_b as u32);

    }
    if let Some(ena) = stepper.lock().await.ena {
        data.entry("ena".to_string()).insert_entry(ena as u32);

    }
    data.entry("ratio".to_string()).insert_entry(stepper.lock().await.ratio as u32);
    data.entry("max".to_string()).insert_entry(stepper.lock().await.max.load(Ordering::Relaxed) as u32);
    data.entry("pos".to_string()).insert_entry(stepper.lock().await.pos.load(Ordering::Relaxed).clone() as u32);
    let mut temp_mem_data = HashMap::new();
    for (k, v) in stepper.lock().await.mem.clone() {
        temp_mem_data.entry(k).insert_entry(v.load(Ordering::Relaxed)as u32);
        
    }
    temp_mem_data
    
    }

//processes all Multi-part form data for all post request handlers.
async fn process_form(mut form: Multipart) -> HashMap<String, String> {
    let mut form_data: HashMap<String, String> = HashMap::new();
    println!("Config PostForm Handler");
    while let Some(val) = form.next_field().await.unwrap() {
        println!("Name: {:?}", val.name().unwrap().to_string());
        let k = val.name().unwrap().to_string();
        let v = val.text().await.unwrap().to_string();
        println!("Key: {}, Value: {}", k, v);
        form_data.insert(k.clone(), v.clone());
    }
    println!("Pwr Button form data {:?}", form_data);
    form_data
}
fn read_html_file(path: &path::Path) -> Result<String> {
    let output = fs::read_to_string(path)?;
    Ok(output)
}
