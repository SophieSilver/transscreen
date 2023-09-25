"use strict";
const message_container = document.getElementById("message_container");
const host = document.location.host;
const socket = new WebSocket(`ws://${host}/websocket`);
socket.onmessage = (event) => {
    const message = document.createElement("p");
    if (typeof event.data !== "string") {
        console.log(`Wrong socket type: ${typeof event.data}`);
        return;
    }
    message.textContent = event.data;
    message_container === null || message_container === void 0 ? void 0 : message_container.append(message);
};
socket.onerror = (event) => {
    console.log(event);
};
