function execute() {
  __RAW_script__;
}

function resolve(result) {
  __TAURI_INTERNALS__.invoke("plugin:automation|resolve", {
    id: __TEMPLATE_id__,
    result:
      result instanceof Error
        ? {
            error: result.name,
            message: result.message,
            stacktrace: result.stack,
          }
        : result,
  });
}

function mapArg(arg) {
  if (
    typeof arg === "object" &&
    "__selector__" in arg &&
    "__position__" in arg
  ) {
    return document.querySelectorAll(arg.__selector__)[arg.__position__];
  }
  return arg;
}

try {
  execute(...[...__TEMPLATE_args__.map(mapArg), resolve]);
} catch (error) {
  resolve(error);
}
