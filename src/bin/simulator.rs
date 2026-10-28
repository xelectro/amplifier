use async_stream::stream;
use axum::{
    Router,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::StatusCode,
    response::{
        Html, IntoResponse,
        sse::{Event, KeepAlive, Sse},
    },
    routing::{get, post},
};
use futures_util::{StreamExt, stream::Stream};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::{
    convert::Infallible,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;
use tower_http::services::ServeDir;

const MAX_TUNE: u32 = 12_000;
const MAX_IND: u32 = 8_000;
const MAX_LOAD: u32 = 10_000;

#[derive(Clone)]
struct SimState {
    started: Instant,
    selected: Arc<Mutex<String>>,
}

#[tokio::main]
async fn main() {
    let state = SimState {
        started: Instant::now(),
        selected: Arc::new(Mutex::new("Tune".to_string())),
    };

    let app = Router::new()
        .route("/", get(index))
        .route("/sse", get(sse))
        .route("/round_dial", get(round_dial))
        .route("/round_dial_ws", get(round_dial_ws))
        .route("/config", get(simulation_page))
        .route("/voltage", get(simulation_page))
        .route("/selector/{name}", post(select_tuner))
        .route("/store/{band}", post(ok))
        .route("/recall/{band}", post(ok))
        .route("/stop", post(ok))
        .route("/pwr_btn", post(ok))
        .route("/mousewheel", post(ok))
        .nest_service("/static", ServeDir::new("static"))
        .with_state(state);

    let address = "127.0.0.1:3000";
    let listener = tokio::net::TcpListener::bind(address).await.unwrap();
    println!("Amplifier UI simulator: http://{address}");
    axum::serve(listener, app).await.unwrap();
}

async fn index() -> Html<&'static str> {
    Html(include_str!("../../templates/amplifier2.html"))
}

async fn round_dial() -> Html<&'static str> {
    Html(include_str!("../../templates/round_dial.html"))
}

async fn simulation_page() -> Html<&'static str> {
    Html(
        "<h1>Simulation mode</h1><p>Hardware configuration is disabled.</p><p><a href=\"/\">Return to amplifier UI</a></p>",
    )
}

async fn ok() -> StatusCode {
    StatusCode::NO_CONTENT
}

async fn select_tuner(
    axum::extract::Path(name): axum::extract::Path<String>,
    State(state): State<SimState>,
) -> StatusCode {
    *state.selected.lock().await = match name.to_ascii_lowercase().as_str() {
        "ind" => "Ind",
        "load" => "Load",
        _ => "Tune",
    }
    .to_string();
    StatusCode::NO_CONTENT
}

fn triangle_wave(elapsed_ms: u128, period_ms: u128, max: u32, offset_ms: u128) -> u32 {
    let phase = (elapsed_ms + offset_ms) % period_ms;
    let half = period_ms / 2;
    let progress = if phase <= half {
        phase
    } else {
        period_ms - phase
    };
    ((progress as f64 / half as f64) * max as f64).round() as u32
}

async fn sse(State(state): State<SimState>) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let output = stream! {
        let mut ticker = tokio::time::interval(Duration::from_millis(100));
        loop {
            ticker.tick().await;
            let elapsed = state.started.elapsed().as_millis();
            let tune = triangle_wave(elapsed, 24_000, MAX_TUNE, 0);
            let ind = triangle_wave(elapsed, 31_000, MAX_IND, 7_000);
            let load = triangle_wave(elapsed, 27_000, MAX_LOAD, 13_000);
            let selected = state.selected.lock().await.clone();
            let seconds = state.started.elapsed().as_secs_f64();

            let payload = json!({
                "tune": tune,
                "ind": ind,
                "load": load,
                "max": {"tune": MAX_TUNE, "ind": MAX_IND, "load": MAX_LOAD},
                "ratio": {"tune": 1, "ind": 2, "load": 1},
                "sw_pos": selected,
                "band": "M10",
                "i2c_devices": [],
                "plate_v": 4200 + (seconds.sin() * 250.0) as i32,
                "plate_a": 1.25 + seconds.sin() * 0.35,
                "screen_a": 85 + (seconds.sin() * 20.0) as i32,
                "grid_a": 18 + (seconds.cos() * 7.0) as i32,
                "pwr_btns": {
                    "Blwr": ["ON", "OFF"],
                    "Fil": ["ON", "ON"],
                    "HV": ["ON", "ON"],
                    "Oper": ["OFF", "OFF"]
                },
                "temperature": 31.0 + seconds.sin() * 2.0,
                "call_sign": "SIMULATOR",
                "time": chrono::Local::now().format("%m-%d-%Y, %H:%M:%S").to_string(),
                "status": "Local simulation — no GPIO or I²C hardware",
            });

            yield Ok(Event::default().data(payload.to_string()));
        }
    };

    Sse::new(output).keep_alive(KeepAlive::default())
}

#[derive(Deserialize)]
struct DialQuery {
    tuner: String,
}

#[derive(Deserialize)]
struct DialRequest {
    #[serde(rename = "type")]
    message_type: String,
    display_position: Option<u32>,
}

#[derive(Serialize)]
struct DialResponse<'a> {
    #[serde(rename = "type")]
    message_type: &'static str,
    tuner: &'a str,
    label: &'a str,
    ratio: u8,
    display_position: u32,
    raw_position: u32,
    max_display: u32,
    max_raw: u32,
    status: &'static str,
}

async fn round_dial_ws(ws: WebSocketUpgrade, Query(query): Query<DialQuery>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| dial_socket(socket, query.tuner))
}

async fn dial_socket(mut socket: WebSocket, tuner: String) {
    let (slug, label, max_display, ratio) = match tuner.to_ascii_lowercase().as_str() {
        "tune" => ("tune", "Tune", MAX_TUNE, 1),
        "ind" => ("ind", "Inductor", MAX_IND / 2, 2),
        "load" => ("load", "Load", MAX_LOAD, 1),
        _ => {
            let _ = socket
                .send(Message::Text(
                    json!({"type": "error", "status": "Invalid tuner"})
                        .to_string()
                        .into(),
                ))
                .await;
            return;
        }
    };

    let mut display_position = max_display / 3;
    if send_dial_state(
        &mut socket,
        slug,
        label,
        ratio,
        display_position,
        max_display,
    )
    .await
    .is_err()
    {
        return;
    }

    while let Some(Ok(message)) = socket.next().await {
        match message {
            Message::Text(text) => {
                if let Ok(request) = serde_json::from_str::<DialRequest>(&text) {
                    if request.message_type == "set" {
                        if let Some(position) = request.display_position {
                            display_position = position.min(max_display);
                            if send_dial_state(
                                &mut socket,
                                slug,
                                label,
                                ratio,
                                display_position,
                                max_display,
                            )
                            .await
                            .is_err()
                            {
                                break;
                            }
                        }
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}

async fn send_dial_state(
    socket: &mut WebSocket,
    slug: &str,
    label: &str,
    ratio: u8,
    display_position: u32,
    max_display: u32,
) -> Result<(), axum::Error> {
    let response = DialResponse {
        message_type: "state",
        tuner: slug,
        label,
        ratio,
        display_position,
        raw_position: display_position * ratio as u32,
        max_display,
        max_raw: max_display * ratio as u32,
        status: "Simulation connected",
    };
    socket
        .send(Message::Text(
            serde_json::to_string(&response).unwrap().into(),
        ))
        .await
}
