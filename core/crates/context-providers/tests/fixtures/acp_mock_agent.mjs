import readline from "node:readline";

const rl = readline.createInterface({
  input: process.stdin,
  crlfDelay: Infinity,
});

let nextSession = 1;
const pendingPrompts = new Map();

function send(obj) {
  process.stdout.write(JSON.stringify(obj));
  process.stdout.write("\n");
}

function respondToPending() {
  const entries = Array.from(pendingPrompts.values());
  if (entries.length < 2) {
    return;
  }

  for (const entry of entries) {
    send({
      jsonrpc: "2.0",
      method: "session/update",
      params: {
        sessionId: entry.sessionId,
        update: {
          sessionUpdate: "agent_message_chunk",
          content: {
            type: "text",
            text: `hello-${entry.sessionId}`,
          },
        },
      },
    });
  }

  for (const entry of entries) {
    send({
      jsonrpc: "2.0",
      id: entry.id,
      result: {
        stopReason: "end_turn",
      },
    });
  }

  pendingPrompts.clear();
}

rl.on("line", (line) => {
  let msg;
  try {
    msg = JSON.parse(line);
  } catch {
    return;
  }

  if (msg.method === "initialize") {
    send({
      jsonrpc: "2.0",
      id: msg.id,
      result: {
        protocolVersion: 1,
        agentCapabilities: {
          promptCapabilities: {
            image: false,
            embeddedContext: true,
          },
          loadSession: false,
        },
      },
    });
    return;
  }

  if (msg.method === "session/new") {
    const sessionId = `sess_${nextSession++}`;
    send({
      jsonrpc: "2.0",
      id: msg.id,
      result: {
        sessionId,
      },
    });
    return;
  }

  if (msg.method === "session/load") {
    const sessionId = msg.params?.sessionId || `sess_${nextSession++}`;
    send({
      jsonrpc: "2.0",
      id: msg.id,
      result: {
        sessionId,
      },
    });
    return;
  }

  if (msg.method === "session/prompt") {
    const sessionId = msg.params?.sessionId || `sess_${nextSession++}`;
    pendingPrompts.set(sessionId, { id: msg.id, sessionId });
    respondToPending();
    return;
  }

  if (msg.method === "session/cancel") {
    return;
  }
});
