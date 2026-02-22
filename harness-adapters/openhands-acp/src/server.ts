import { Readable, Writable } from "node:stream";
import { AgentSideConnection, ndJsonStream } from "@agentclientprotocol/sdk";
import { OpenHandsAcpAgent } from "./agent.js";

export function runAcp(): void {
  const stdout = Writable.toWeb(process.stdout) as WritableStream<Uint8Array>;
  const stdin = Readable.toWeb(process.stdin) as ReadableStream<Uint8Array>;
  const stream = ndJsonStream(stdout, stdin);

  new AgentSideConnection((client) => new OpenHandsAcpAgent(client), stream);
}
