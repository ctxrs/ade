export async function parseWsJson(data: unknown): Promise<any | null> {
  let text: string | null = null;

  if (typeof data === "string") {
    text = data;
  } else if (
    data &&
    typeof data === "object" &&
    (typeof (data as any).text === "function" ||
      typeof (data as any).arrayBuffer === "function" ||
      (typeof (data as any).slice === "function" && typeof (data as any).size === "number") ||
      Object.prototype.toString.call(data) === "[object Blob]" ||
      Object.prototype.toString.call(data) === "[object File]")
  ) {
    // Blob-like (WebSocket implementations vary; some WebViews deliver text frames as Blob).
    const anyBlob = data as any;
    if (typeof anyBlob.text === "function") {
      text = await anyBlob.text();
    } else if (typeof anyBlob.arrayBuffer === "function") {
      const ab = (await anyBlob.arrayBuffer()) as ArrayBuffer;
      text = new TextDecoder().decode(new Uint8Array(ab));
    } else if (typeof (globalThis as any).Response === "function") {
      text = await new (globalThis as any).Response(anyBlob).text();
    } else if (typeof (globalThis as any).FileReader === "function") {
      text = await new Promise<string>((resolve, reject) => {
        const reader = new (globalThis as any).FileReader();
        reader.onload = () => resolve(String(reader.result ?? ""));
        reader.onerror = () => reject(reader.error ?? new Error("Failed to read Blob"));
        reader.readAsText(anyBlob);
      });
    } else {
      return null;
    }
  } else if (data instanceof ArrayBuffer) {
    text = new TextDecoder().decode(new Uint8Array(data));
  } else if (ArrayBuffer.isView(data)) {
    text = new TextDecoder().decode(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
  } else {
    return null;
  }

  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}
