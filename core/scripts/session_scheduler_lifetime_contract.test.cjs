const assert = require("node:assert/strict");
const fs = require("node:fs");
const path = require("node:path");
const test = require("node:test");

const ROOT = path.resolve(__dirname, "..", "..");

function read(relativePath) {
  return fs.readFileSync(path.join(ROOT, relativePath), "utf8");
}

test("session message scheduler spawner strongly owns the scheduler host", () => {
  const messageCommands = read("core/crates/ctx-daemon/src/daemon/session_route_handles/message_commands.rs");
  const sessionRoutes = read("core/crates/ctx-daemon/src/daemon/route_builders/sessions.rs");
  const taskRoutes = read("core/crates/ctx-daemon/src/daemon/route_builders/tasks.rs");

  assert.match(
    messageCommands,
    /struct SessionMessageSchedulerSpawner \{\s+host: Arc<SessionSchedulerWorkerHost>,\s+\}/,
  );
  assert.match(
    messageCommands,
    /fn new\(host: Arc<SessionSchedulerWorkerHost>\) -> Self/,
  );
  assert.match(
    messageCommands,
    /let host = Arc::downgrade\(&self\.host\);[\s\S]*session_worker\(host, session, rx\)/,
  );
  assert.doesNotMatch(
    messageCommands,
    /struct SessionMessageSchedulerSpawner \{\s+host: Weak<SessionSchedulerWorkerHost>,\s+\}/,
  );

  for (const source of [sessionRoutes, taskRoutes]) {
    assert.doesNotMatch(
      source,
      /SessionMessageSchedulerSpawner::new\(Arc::downgrade/,
    );
  }
});
