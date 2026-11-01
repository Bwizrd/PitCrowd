// relay_door.mjs — the relay's front door.
//
// macOS's application firewall blocks incoming connections to any program it has not
// been told about, and it learns about programs by asking in a dialog the first time
// one listens. The relay is a freshly built, ad-hoc signed cargo binary started
// headless on a locked Mac: nobody ever sees that dialog, and every rebuild would
// reset the answer anyway. So from everywhere but this machine, :8099 timed out.
//
// Node is already allowed through the firewall (the market tape runs on it), and the
// firewall judges the node binary, not the script. So node owns the public port and
// hands every request, unchanged, to the relay listening on loopback only:
//
//   browser / desk  ->  node  0.0.0.0:8099  ->  jev_relay  127.0.0.1:8100
//
// Plain HTTP only (the relay has no websockets). Status, headers and bodies pass
// through both ways; a relay that is down answers 502 with a JSON body and CORS
// headers so the page can show a reason instead of a hang.
//
//   RELAY_DOOR_LISTEN   address:port to listen on   (default 0.0.0.0:8099)
//   RELAY_UPSTREAM      the relay's address:port    (default 127.0.0.1:8100)
//
// Started by start-floor.sh after the relay itself; log in jev_relay/log/relay_door.out.

import http from "node:http";

const listen = process.env.RELAY_DOOR_LISTEN || "0.0.0.0:8099";
const upstream = process.env.RELAY_UPSTREAM || "127.0.0.1:8100";
const [listenHost, listenPort] = splitHostPort(listen);
const [upHost, upPort] = splitHostPort(upstream);
const UPSTREAM_TIMEOUT_MS = 120_000; // a /jev call can wait on the model

const agent = new http.Agent({ keepAlive: true, maxSockets: 64 });
let forwarded = 0;
let failed = 0;

function splitHostPort(s) {
  const i = s.lastIndexOf(":");
  return [s.slice(0, i), Number(s.slice(i + 1))];
}

function stamp() {
  return new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
}

function fail(res, code, reason) {
  failed += 1;
  if (!res.headersSent) {
    res.writeHead(code, {
      "content-type": "application/json",
      "access-control-allow-origin": "*",
      "access-control-allow-headers": "*",
    });
  }
  res.end(JSON.stringify({ error: reason, door: listen, upstream }));
}

const server = http.createServer((req, res) => {
  const headers = { ...req.headers, host: upstream };
  const from = req.socket.remoteAddress || "";
  headers["x-forwarded-for"] = headers["x-forwarded-for"] ? `${headers["x-forwarded-for"]}, ${from}` : from;
  const out = http.request(
    { host: upHost, port: upPort, method: req.method, path: req.url, headers, agent, timeout: UPSTREAM_TIMEOUT_MS },
    (up) => {
      res.writeHead(up.statusCode || 502, up.headers);
      up.pipe(res);
      up.on("end", () => { forwarded += 1; });
    },
  );
  out.on("timeout", () => { out.destroy(new Error("upstream timeout")); });
  out.on("error", (e) => {
    console.log(`${stamp()} relay_door: ${req.method} ${req.url} from ${from}: relay unreachable (${e.message})`);
    fail(res, 502, `relay on ${upstream} unreachable: ${e.message}`);
  });
  req.on("aborted", () => out.destroy());
  req.pipe(out);
});

server.on("error", (e) => {
  console.log(`${stamp()} relay_door: cannot listen on ${listen}: ${e.message}`);
  process.exit(1);
});

server.listen(listenPort, listenHost, () => {
  console.log(`${stamp()} relay_door: listening on ${listen}, forwarding to the relay on ${upstream}`);
});

// One line an hour so the log says it is alive without saying it for every poll.
setInterval(() => {
  console.log(`${stamp()} relay_door: forwarded ${forwarded}  failed ${failed}`);
}, 3_600_000).unref();

for (const sig of ["SIGTERM", "SIGINT"]) {
  process.on(sig, () => {
    console.log(`${stamp()} relay_door: ${sig}, closing`);
    server.close(() => process.exit(0));
    setTimeout(() => process.exit(0), 2000).unref();
  });
}
