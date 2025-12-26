import { Buffer } from "buffer";

type BlobLike = {
  text?: () => Promise<string>;
  arrayBuffer?: () => Promise<ArrayBuffer>;
};

const decodeBytes = (bytes: Uint8Array): string => {
  if (typeof TextDecoder !== "undefined") {
    return new TextDecoder().decode(bytes);
  }
  return Buffer.from(bytes).toString("utf-8");
};

export async function parseWsJson(data: unknown): Promise<any | null> {
  let text: string | null = null;

  if (typeof data === "string") {
    text = data;
  } else if (data && typeof data === "object") {
    const maybeBlob = data as BlobLike;
    if (typeof maybeBlob.text === "function") {
      text = await maybeBlob.text();
    } else if (typeof maybeBlob.arrayBuffer === "function") {
      const ab = await maybeBlob.arrayBuffer();
      text = decodeBytes(new Uint8Array(ab));
    } else if (data instanceof ArrayBuffer) {
      text = decodeBytes(new Uint8Array(data));
    } else if (ArrayBuffer.isView(data)) {
      text = decodeBytes(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
    }
  } else if (data instanceof ArrayBuffer) {
    text = decodeBytes(new Uint8Array(data));
  } else if (ArrayBuffer.isView(data)) {
    text = decodeBytes(new Uint8Array(data.buffer, data.byteOffset, data.byteLength));
  }

  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return null;
  }
}
