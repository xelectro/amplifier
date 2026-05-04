let bar_meters = document.getElementById("bar_meters");
let bar_meter_tune = document.createElement("td");
let bar_meter_ind = document.createElement("td");
let bar_meter_load = document.createElement("td");
let temperature = document.createElement("td");
let temperature_data = document.createElement("h2");
temperature_data.setAttribute("style", "color: green")
temperature.appendChild(temperature_data);
let clock = document.createElement("td");
let adv_voltage = document.createElement("button");
let advVoltageAttributes = {"class" : "button2", "id": "adv_voltage_btn", "name": "adv_voltage", "value": "adv_voltage", "type": "submit"};
for ([key, val] of Object.entries(advVoltageAttributes)) {
    adv_voltage.setAttribute(key, val)
}
adv_voltage.classList.add("active")
adv_voltage.addEventListener("mouseover", (event) => {
    event.target.classList.add("mouseover");
    });
adv_voltage.addEventListener("mouseout", (event) => {
    event.target.classList.remove("mouseover");
    });
adv_voltage.innerText = "Adv Voltage";
let voltageWindow;
adv_voltage.addEventListener("click", event =>{
    event.preventDefault();
    console.log(event.target.name);
    console.log(event.target);
    voltageWindow = window.open(
        "/voltage",
        "Advanced_Voltage",
        "width=300, height=300",
    );
    

})
let clock_data = document.createElement("h3");
clock_data.setAttribute("style", "color: magenta");
clock.appendChild(adv_voltage);
let call_sign = document.createElement("td");
let call_sign_data = document.createElement("h1");
call_sign_data.setAttribute("style", "color: cyan");
call_sign.appendChild(call_sign_data);

// Move clock/callsign into the header placeholders (no behavior change)
const headerAddr = document.getElementById("header_addr");
if (headerAddr) { headerAddr.textContent = window.location.host; }

const headerClock = document.getElementById("header_clock");
if (headerClock) {
    headerClock.replaceChildren(clock_data);
    //clock.style.display = "none";  // keep table layout stable
}

const headerCall = document.getElementById("header_callsign");
if (headerCall) {
    headerCall.replaceChildren(call_sign_data);
    call_sign.style.display = "none";  // keep table layout stable
}
const headerConn = document.getElementById("header_conn");
const headerConnDot = document.getElementById("header_conn_dot");

const themeToggle = document.getElementById("theme_toggle");
const THEME_KEY = "amplifier-theme";
let themeMeterColors = { fill: "#0f0", face: "#fff" };

function applyTheme(theme) {
    const selectedTheme = theme === "dark" ? "dark" : "light";
    document.body.dataset.theme = selectedTheme;
    const styles = getComputedStyle(document.body);
    themeMeterColors = {
        fill: styles.getPropertyValue("--meter-fill").trim() || "#0f0",
        face: styles.getPropertyValue("--meter-face").trim() || "#fff",
    };
    if (themeToggle) {
        const darkMode = selectedTheme === "dark";
        themeToggle.textContent = darkMode ? "Light" : "Dark";
        themeToggle.setAttribute("aria-pressed", darkMode ? "true" : "false");
        themeToggle.setAttribute("title", `Switch to ${darkMode ? "light" : "dark"} theme`);
    }
}

applyTheme(localStorage.getItem(THEME_KEY) || "light");

if (themeToggle) {
    themeToggle.addEventListener("click", () => {
        const nextTheme = document.body.dataset.theme === "dark" ? "light" : "dark";
        localStorage.setItem(THEME_KEY, nextTheme);
        applyTheme(nextTheme);
        renderMeters();
    });
}

bar_meters.appendChild(bar_meter_tune);
bar_meters.appendChild(bar_meter_ind);
bar_meters.appendChild(bar_meter_load);
bar_meters.appendChild(temperature);
bar_meters.appendChild(clock);
bar_meters.appendChild(call_sign);
let meter_tune = document.getElementById("meter_tune");
let meterReadingElement_tune = document.getElementById("tune");
let meter_ind = document.getElementById("meter_ind");
let meterReadingElement_ind = document.getElementById("ind");
let meter_load = document.getElementById("meter_load");
let meterReadingElement_load = document.getElementById("load");
const meterContainers = {
    tune: meter_tune?.closest(".meter-container"),
    ind: meter_ind?.closest(".meter-container"),
    load: meter_load?.closest(".meter-container"),
};
let learn_update = new EventSource("/sse");
let last_meter_value_tune = 0;
let last_meter_value_ind = 0;
let last_meter_value_load = 0;
let timeStamp = Date.now();
let sleep = false;
let meter_values = {};
let meter_color = "";
let storeMode = false;
let lastActivityKey = "";
let lastRenderedValues = {
    tuneTurns: null,
    tuneArc: null,
    indTurns: null,
    indArc: null,
    loadTurns: null,
    loadArc: null,
    plate_v: null,
    plate_a: null,
    screen_a: null,
    grid_a: null,
};

function setStreamStatus(isOnline) {
    if (headerConn) {
        headerConn.textContent = isOnline ? "ONLINE" : "OFFLINE";
    }
    if (headerConnDot) {
        headerConnDot.classList.toggle("amp-dot-online", isOnline);
        headerConnDot.classList.toggle("amp-dot-offline", !isOnline);
    }
}

setStreamStatus(false);
// tune, ind, load button configuration.
const storeBtn = document.getElementById("store_btn");
storeBtn.addEventListener("click", (event) => {
    storeMode = storeMode == false ? true : false;
    if (storeMode == true) {
        storeBtn.classList.add("hover_not_disabled");
    } else {
        removeStore();
    }
    console.log(bandSelectors.childNodes);
});

const tuneBtn = document.getElementById("tune_button");
const indBtn = document.getElementById("ind_button");
const loadBtn = document.getElementById("load_button");
const bandSelectors = document.getElementById("band_selectors");
    bandSelectors.replaceChildren();
const bands = ["M10", "M11", "M20", "M40", "M80"];
bands.forEach((band, i) => {
    console.log(band);
    console.log(i);
    removeStore();
    let btnBox = document.createElement("td");
    let btn = document.createElement("button");
    btn.classList.add("button");
    btn.setAttribute("id", band);
    btn.innerText = band.slice(1, 3) + band.slice(0, 1);
    btn.addEventListener("click", (event) => {
        if (storeMode == true) {
            removeStore();
            fetch(`/store/${band}`, {
                method: "POST",
            });
        } else {
            fetch(`/recall/${band}`, {
                method: "POST",
            });
        }
    });
    btnBox.appendChild(btn);
    bandSelectors.appendChild(btnBox);
});
const myButtons = document.querySelectorAll(".button");
const configBtn = document.getElementById("config_btn");
const saveBtn = document.getElementById("save_btn");
const statusBar = document.getElementById("status_bar");
const statusBarContents = document.createElement("h3");
// Power buttons at the bottom.
pwrBtsArr = ["Blwr", "Fil", "HV", "Oper"];
pwrBtnTable = document.getElementById("power_btns");
pwrBtnTableRow = document.createElement("tr");
pwrBtnTable.appendChild(pwrBtnTableRow);
pwrBtsArr.forEach((btn) => {
    const tableData = document.createElement("td");
    const newForm = document.createElement("form");
    newForm.setAttribute("class", "form");
    newForm.setAttribute("id", `sw_${btn}`);
    newForm.setAttribute("action", "#");
    const newLabel = document.createElement("label");
    newLabel.setAttribute("class", "switch");
    const newInput = document.createElement("input");
    newInput.setAttribute("name", btn);
    newInput.setAttribute("type", "checkbox");
    newInput.addEventListener("change", pwrBtnAction);
    const newSpan = document.createElement("span");
    newSpan.setAttribute("class", "slider round");
    newLabel.appendChild(newInput);
    newLabel.appendChild(newSpan);
    newForm.appendChild(newLabel);
    tableData.appendChild(newForm);
    pwrBtnTableRow.appendChild(tableData);
});
pwrBtnTable.appendChild(pwrBtnTableRow);
function pwrBtnAction(event) {
    let formData = new FormData();
    formData.append("ID", event.target.name);
    formData.append("value", event.target.checked ? "ON" : "OFF");
    if (event.target.name == "Fil" || event.target.name == "HV") {
        formData.append("delay", "OFF");
    }

    if (meter_values && meter_values.pwr_btns && meter_values.pwr_btns[event.target.name]) {
        console.log(meter_values.pwr_btns[event.target.name][0]);
    }
    if (event.target.name == "Fil" || event.target.name == "HV") {
        if (event.target.checked) {
            fetch("/pwr_btn", {
                method: "POST",
                body: formData,
            });
            setTimeout(() => {
                formData.set("delay", "ON");
                fetch("/pwr_btn", {
                    method: "POST",
                    body: formData,
                });
            }, 3000);
        } else {
            fetch("/pwr_btn", {
                method: "POST",
                body: formData,
            });
        }
    } else {
        fetch("/pwr_btn", {
            method: "POST",
            body: formData,
        });
    }
}

let configWindow;
statusBar.appendChild(statusBarContents);
configBtn.addEventListener("click", (event) => {
    console.log(event);
    configWindow = window.open(
        "/config",
        "Config-Page",
        "width=980, height=760",
    );
});
saveBtn.addEventListener("click", (event) => {
    formData = new FormData();
    console.log(event.target.name);
    formData.append(event.target.name, event.target.value);
    fetch("/stop", {
        method: "POST",
        body: formData,
    });
});
const formVals = ["tune", "ind", "load"];
let lastSelectorPosition = "";
let lastBandSelected = "";
let roundDialWindow = null;

function openRoundDial(tuner) {
    const url = `/round_dial?tuner=${encodeURIComponent(tuner)}`;
    const popupWidth = 520;
    const popupHeight = 720;
    const popupLeft = Math.max(0, window.screenX + window.outerWidth - popupWidth);
    const popupTop = Math.max(0, window.screenY + 40);
    roundDialWindow = window.open(
        url,
        "Round_Dial",
        `width=${popupWidth},height=${popupHeight},left=${popupLeft},top=${popupTop}`,
    );
    if (roundDialWindow) {
        roundDialWindow.focus();
        try {
            roundDialWindow.moveTo(popupLeft, popupTop);
        } catch (_err) {}
    }
}

Object.entries(meterContainers).forEach(([tuner, container]) => {
    if (!container) {
        return;
    }
    container.style.cursor = "pointer";
    container.title = `Open ${tuner} touch dial`;
    container.addEventListener("click", () => openRoundDial(tuner));
});

myButtons.forEach((button, i) => {
    button.classList.add("active");
    if (i < 3) {
        const formData = new FormData();
        formData.append(formVals[i], "submit");
        console.log(formData);
        button.addEventListener("click", (event) => {
            fetch(`/selector/${formVals[i]}`, {
                method: "POST",
                body: formData,
            });
        });
        button.addEventListener("mousewheel", (event) => {
            const formData = new FormData();
            formData.append(formVals[i], event.wheelDelta);
            console.log(formData);
            fetch("/mousewheel", {
                method: "POST",
                body: formData,
            });
        });
    }
    button.addEventListener("mouseover", (event) => {
        event.target.classList.add("mouseover");
    });
    button.addEventListener("mouseout", (event) => {
        event.target.classList.remove("mouseover");
    });
});
let gauges = document.querySelectorAll(".gauge");
function updateBarMeter(container, currentValue, maxValue) {
    let meter = container.querySelector("meter");
    let label = container.querySelector(".meter-label");
    if (!meter || !label) {
        container.replaceChildren();
        label = document.createElement("p");
        label.className = "meter-label";
        meter = document.createElement("meter");
        label.appendChild(document.createTextNode(""));
        label.appendChild(document.createElement("br"));
        label.appendChild(meter);
        container.appendChild(label);
    }
    label.firstChild.textContent = `Current value = ${currentValue}: `;
    meter.value = currentValue;
    meter.min = 0;
    meter.max = maxValue;
    meter.low = 0;
    meter.high = 800;
    meter.optimum = 500;
}

function powerButtonState(name, index) {
    return meter_values?.pwr_btns?.[name]?.[index] || "OFF";
}

function renderMeters() {
    if (!meter_values || !meter_values.ratio) {
        return;
    }
    const tuneDisplay = displayReading(meter_values.tune, meter_values.ratio.tune);
    const indDisplay = displayReading(meter_values.ind, meter_values.ratio.ind);
    const loadDisplay = displayReading(meter_values.load, meter_values.ratio.load);
    const tuneArc = tuneDisplay[1] * 0.9;
    const indArc = indDisplay[1] * 0.9;
    const loadArc = loadDisplay[1] * 0.9;

    if (lastRenderedValues.tuneTurns !== tuneDisplay[0]) {
        meterReadingElement_tune.innerText = tuneDisplay[0];
        lastRenderedValues.tuneTurns = tuneDisplay[0];
    }
    if (lastRenderedValues.tuneArc !== tuneArc) {
        meter_tune.style.background = `conic-gradient(${themeMeterColors.fill} ${tuneArc}deg, ${themeMeterColors.face} 0deg)`;
        lastRenderedValues.tuneArc = tuneArc;
    }

    if (lastRenderedValues.indTurns !== indDisplay[0]) {
        meterReadingElement_ind.innerText = indDisplay[0];
        lastRenderedValues.indTurns = indDisplay[0];
    }
    if (lastRenderedValues.indArc !== indArc) {
        meter_ind.style.background = `conic-gradient(${themeMeterColors.fill} ${indArc}deg, ${themeMeterColors.face} 0deg)`;
        lastRenderedValues.indArc = indArc;
    }

    if (lastRenderedValues.loadTurns !== loadDisplay[0]) {
        meterReadingElement_load.innerText = loadDisplay[0];
        lastRenderedValues.loadTurns = loadDisplay[0];
    }
    if (lastRenderedValues.loadArc !== loadArc) {
        meter_load.style.background = `conic-gradient(${themeMeterColors.fill} ${loadArc}deg, ${themeMeterColors.face} 0deg)`;
        lastRenderedValues.loadArc = loadArc;
    }

    gauges.forEach((gauge, i) => {
        switch (i) {
            case 0:
                if (lastRenderedValues.plate_v !== meter_values.plate_v) {
                    gauge.style.setProperty("--value", meter_values.plate_v / 10000);
                    gauge.textContent = Math.round(meter_values.plate_v) + "V";
                    lastRenderedValues.plate_v = meter_values.plate_v;
                }
                break;
            case 1:
                if (lastRenderedValues.plate_a !== meter_values.plate_a) {
                    const plateAmps = Number(meter_values.plate_a) || 0;
                    gauge.style.setProperty("--value", plateAmps / 3);
                    gauge.textContent = (plateAmps < 1 ? plateAmps.toFixed(2) : plateAmps.toFixed(1)) + "A";
                    lastRenderedValues.plate_a = meter_values.plate_a;
                }
                break;
            case 2:
                if (lastRenderedValues.screen_a !== meter_values.screen_a) {
                    gauge.style.setProperty("--value", meter_values.screen_a / 200);
                    gauge.textContent = Math.round(meter_values.screen_a) + "mA";
                    lastRenderedValues.screen_a = meter_values.screen_a;
                }
                break;
            case 3:
                if (lastRenderedValues.grid_a !== meter_values.grid_a) {
                    gauge.style.setProperty("--value", meter_values.grid_a / 50);
                    gauge.textContent = Math.round(meter_values.grid_a) + "mA";
                    lastRenderedValues.grid_a = meter_values.grid_a;
                }
                break;
        }
    });
}

learn_update.onmessage = (e) => {
    if (e.data == "close") {
        setStreamStatus(false);
        learn_update.close();
    } else {
        setStreamStatus(true);
        meter_values = JSON.parse(e.data);
        if (configWindow != null) {
            configWindow.postMessage(
                meter_values.status,
                window.location.origin,
            );
        }
        temperature_data.innerText = "Temp:" +  Math.round(meter_values.temperature) + " C";
        if (voltageWindow != null) {
            //voltageWindow.postMessage(meter_values, window.location.origin);
        }
        clock_data.innerText = meter_values.time;
        call_sign_data.innerText = meter_values.call_sign;
        updateBarMeter(bar_meter_tune, meter_values.tune, meter_values.max.tune);
        updateBarMeter(bar_meter_ind, meter_values.ind, meter_values.max.ind);
        updateBarMeter(bar_meter_load, meter_values.load, meter_values.max.load);
        statusBarContents.innerText = `Status Bar: ${meter_values.status}`;
        if (powerButtonState("Fil", 1) == "ON") {
            pwrBtnTableRow.childNodes[1].childNodes[0].childNodes[0].childNodes[1].setAttribute(
                "style",
                "background-color: magenta;",
            );
        } else {
            pwrBtnTableRow.childNodes[1].childNodes[0].childNodes[0].childNodes[1].removeAttribute(
                "style",
            );
        }
        if (powerButtonState("HV", 1) == "ON") {
            pwrBtnTableRow.childNodes[2].childNodes[0].childNodes[0].childNodes[1].setAttribute(
                "style",
                "background-color: magenta;",
            );
        } else {
            pwrBtnTableRow.childNodes[2].childNodes[0].childNodes[0].childNodes[1].removeAttribute(
                "style",
            );
        }
        if (
            lastSelectorPosition !== meter_values.sw_pos ||
            lastBandSelected !== meter_values.band
        ) {
            sleep = false;
            removeStore();
            timeStamp = Date.now();
            myButtons.forEach((button) => {
                button.classList.remove("hover_not_disabled");
            });

            switch (meter_values.sw_pos) {
                case "Tune":
                    tuneBtn.classList.add("hover_not_disabled");
                    break;
                case "Ind":
                    indBtn.classList.add("hover_not_disabled");
                    break;
                case "Load":
                    loadBtn.classList.add("hover_not_disabled");
                    break;
            }

            if (meter_values.band != "") {
                document
                    .getElementById(meter_values.band)
                    .classList.add("hover_not_disabled");
            }
            lastSelectorPosition = meter_values.sw_pos;
            lastBandSelected = meter_values.band;
        }

        const activityKey = [
            meter_values.tune,
            meter_values.ind,
            meter_values.load,
            meter_values.band,
            meter_values.sw_pos,
            meter_values.temperature,
            meter_values.plate_v,
            meter_values.plate_a,
            meter_values.screen_a,
            meter_values.grid_a,
            meter_values.status,
        ].join("|");
        if (lastActivityKey === activityKey) {
            if (Date.now() - timeStamp > 120000 && sleep === false) {
                sleep = true;
                let formData = new FormData();
                formData.append("action", "stop");
                fetch("/stop", {
                    method: "POST",
                    body: formData,
                });
            }
        } else {
            timeStamp = Date.now();
            lastActivityKey = activityKey;
        }
        renderMeters();
    }
};
learn_update.onopen = () => {
    setStreamStatus(true);
};

learn_update.onerror = () => {
    setStreamStatus(false);
};
function removeStore() {
    storeBtn.classList.remove("hover_not_disabled");
    storeMode = false;
}
function float2Int(val) {
    return val | 0;
}

function setColor(val) {
    if (val <= 100) {
        meter_color = "#0f0";
    } else if (val > 200 && val < 349) {
        meter_color = "#ff0";
    } else if (val >= 349) {
        meter_color = "#ff0000";
    }
    return meter_color;
}
function displayReading(val, ratio) {
    val = Math.round(val / ratio);
    let count = float2Int(val / 400);
    let meter_value = val - 400 * count;
    let color = setColor(meter_value);
    return [count, meter_value, color];
}
