// tape_door.mjs — the tape's front door on macOS (the same reason as jev_relay/relay_door.mjs).
//
// The macOS application firewall silently blocks incoming connections to a freshly built,
// ad-hoc signed binary started headless, so floor_tape on 0.0.0.0 answers on this machine and
// times out from every other one. Node is normally allowed, and the firewall judges the node
// binary, not the script. So node owns the public port and pipes every connection, raw TCP,
// to floor_tape on loopback: HTTP and websockets both pass through untouched.
//
//   browser / desk  ->  node  0.0.0.0:8083  ->  floor_tape  127.0.0.1:8183
//
//   TAPE_DOOR_LISTEN    address:port to listen on   (default 0.0.0.0:8083)
//   TAPE_UPSTREAM       floor_tape's address:port   (default 127.0.0.1:8183)
import net from "node:net";

const split = (s) => { const i = s.lastIndexOf(":"); return [s.slice(0, i), Number(s.slice(i + 1))]; };
const [lh, lp] = split(process.env.TAPE_DOOR_LISTEN || "0.0.0.0:8083");
const [uh, up] = split(process.env.TAPE_UPSTREAM || "127.0.0.1:8183");

net.createServer((client) => {
  const tape = net.connect(up, uh);
  client.pipe(tape).pipe(client);
  const close = () => { client.destroy(); tape.destroy(); };
  client.on("error", close);
  tape.on("error", close);
  client.on("close", close);
  tape.on("close", close);
}).listen(lp, lh, () => console.log(`${new Date().toISOString()} tape door ${lh}:${lp} -> ${uh}:${up}`));
