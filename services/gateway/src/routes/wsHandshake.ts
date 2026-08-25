import type { Duplex } from "node:stream";

export { writeWebSocketHandshake } from "../../../../packages/transport-safety/src/ws_handshake_response.js";

// Ensure this file remains a module in TS output.
export type _WsHandshakeDuplex = Duplex;

